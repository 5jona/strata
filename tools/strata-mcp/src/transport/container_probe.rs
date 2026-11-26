//! Dynamic Strata container & SSH port discovery over ADB with 15-second TTL cache.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::Mutex;

use super::extract_trailing_json;

const PROBE_TTL: Duration = Duration::from_secs(15);
const STRATA_LIST_CMD: &str = "su -c '/data/adb/ksu/bin/strata --json list'";

/// Minimal container metadata extracted from `strata --json list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerSummary {
    pub(crate) id: String,
    pub(crate) is_default: bool,
    pub(crate) phase: String,
    pub(crate) ssh_enabled: bool,
    pub(crate) ssh_port: u16,
}

impl ContainerSummary {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "isDefault": self.is_default,
            "phase": self.phase,
            "sshEnabled": self.ssh_enabled,
            "sshPort": self.ssh_port,
        })
    }
}

/// Cached diagnostic snapshot of a device's Strata daemon and containers over ADB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerProbe {
    pub(crate) daemon_online: bool,
    pub(crate) containers: Vec<ContainerSummary>,
    pub(crate) active_ssh_port: Option<u16>,
    pub(crate) degraded_reason: Option<String>,
}

fn probe_cache() -> &'static Mutex<HashMap<String, (Instant, ContainerProbe)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, ContainerProbe)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Clears cached container discovery for an ADB serial (or all entries if `None`).
pub(crate) async fn invalidate_container_probe(serial: Option<&str>) {
    let mut guard = probe_cache().lock().await;
    if let Some(ser) = serial {
        guard.remove(ser);
    } else {
        guard.clear();
    }
}

/// Returns a valid cached container probe without spawning `adb`, if within the 15-second TTL.
pub(crate) async fn cached_container_probe(serial: &str) -> Option<ContainerProbe> {
    let guard = probe_cache().lock().await;
    guard
        .get(serial)
        .filter(|(ts, _)| ts.elapsed() < PROBE_TTL)
        .map(|(_, cached)| cached.clone())
}

/// Probes `strata --json list` over ADB with a 15-second TTL cache.
pub(crate) async fn probe_containers_via_adb(serial: &str, force: bool) -> ContainerProbe {
    if !force {
        let guard = probe_cache().lock().await;
        if let Some((ts, cached)) = guard.get(serial)
            && ts.elapsed() < PROBE_TTL
        {
            return cached.clone();
        }
    }

    let out = tokio::time::timeout(
        Duration::from_secs(6),
        Command::new("adb")
            .args(["-s", serial, "shell", STRATA_LIST_CMD])
            .output(),
    )
    .await;

    let probe = match out {
        Ok(Ok(res)) => parse_container_list_probe(
            &String::from_utf8_lossy(&res.stdout),
            &String::from_utf8_lossy(&res.stderr),
            res.status.success(),
        ),
        Ok(Err(err)) => ContainerProbe {
            daemon_online: false,
            containers: Vec::new(),
            active_ssh_port: None,
            degraded_reason: Some(format!("adb probe spawn failed: {err}")),
        },
        Err(_) => ContainerProbe {
            daemon_online: false,
            containers: Vec::new(),
            active_ssh_port: None,
            degraded_reason: Some("adb container probe timed out after 6s".to_owned()),
        },
    };

    let mut guard = probe_cache().lock().await;
    guard.insert(serial.to_owned(), (Instant::now(), probe.clone()));
    probe
}

/// Pure parser for `strata --json list` output used to discover active container SSH ports.
pub(crate) fn parse_container_list_probe(stdout: &str, stderr: &str, ok: bool) -> ContainerProbe {
    let Some(Value::Array(items)) = extract_trailing_json(stdout) else {
        let detail = stderr.trim();
        let reason = if !ok && !detail.is_empty() {
            format!("stratad daemon offline or unreachable ({detail})")
        } else {
            "stratad daemon offline or not started".to_owned()
        };
        return ContainerProbe {
            daemon_online: false,
            containers: Vec::new(),
            active_ssh_port: None,
            degraded_reason: Some(reason),
        };
    };

    let mut containers = Vec::with_capacity(items.len());
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let is_default = item
            .get("isDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let phase = item
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        let ssh_enabled = item
            .get("sshEnabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let ssh_port = item
            .get("sshPort")
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(0);
        containers.push(ContainerSummary {
            id: id.to_owned(),
            is_default,
            phase,
            ssh_enabled,
            ssh_port,
        });
    }

    let preferred = containers
        .iter()
        .find(|c| c.is_default && c.phase == "running" && c.ssh_enabled && c.ssh_port > 0)
        .or_else(|| {
            containers
                .iter()
                .find(|c| c.phase == "running" && c.ssh_enabled && c.ssh_port > 0)
        });

    if let Some(active) = preferred {
        return ContainerProbe {
            daemon_online: true,
            active_ssh_port: Some(active.ssh_port),
            degraded_reason: None,
            containers,
        };
    }

    let reason = if containers.is_empty() {
        "no Strata containers configured (zero-container mode)".to_owned()
    } else if containers.iter().all(|c| c.phase != "running") {
        "all Strata containers are stopped".to_owned()
    } else {
        "running Strata container(s) have Dropbear SSH disabled".to_owned()
    };

    ContainerProbe {
        daemon_online: true,
        containers,
        active_ssh_port: None,
        degraded_reason: Some(reason),
    }
}

/// Builds a structured error response for tools that strictly require a running container.
pub(crate) fn build_container_required_error(tool: &str, probe: &ContainerProbe) -> Value {
    let hint = if !probe.daemon_online {
        "stratad is offline; call device_manage(action=\"start_daemon\") and then device_manage(action=\"start_container\") to recover."
    } else if probe.containers.is_empty() {
        "No Strata container exists; create and start a container with SSH enabled."
    } else {
        "Call device_manage(action=\"start_container\") (or enable Dropbear SSH) to recover container transport."
    };
    let containers_json: Vec<Value> = probe
        .containers
        .iter()
        .map(ContainerSummary::to_json)
        .collect();
    json!({
        "ok": false,
        "error": format!("{tool} requires a running Strata container with SSH enabled"),
        "degradedReason": probe.degraded_reason,
        "daemonOnline": probe.daemon_online,
        "containers": containers_json,
        "hint": hint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_running_default_container_ssh_port() {
        let raw = r#"[
            {"id":"alt","isDefault":false,"phase":"running","sshEnabled":true,"sshPort":2225},
            {"id":"main","isDefault":true,"phase":"running","sshEnabled":true,"sshPort":2223}
        ]"#;
        let p = parse_container_list_probe(raw, "", true);
        assert!(p.daemon_online);
        assert_eq!(p.active_ssh_port, Some(2223));
        assert!(p.degraded_reason.is_none());
    }

    #[test]
    fn falls_back_to_first_running_ssh_container_when_default_stopped() {
        let raw = r#"[
            {"id":"main","isDefault":true,"phase":"stopped","sshEnabled":true,"sshPort":2222},
            {"id":"dev","isDefault":false,"phase":"running","sshEnabled":true,"sshPort":2229}
        ]"#;
        let p = parse_container_list_probe(raw, "", true);
        assert!(p.daemon_online);
        assert_eq!(p.active_ssh_port, Some(2229));
    }

    #[test]
    fn reports_zero_containers_stopped_ssh_disabled_and_offline_daemon() {
        let zero = parse_container_list_probe("[]", "", true);
        assert!(zero.daemon_online);
        assert_eq!(zero.active_ssh_port, None);
        assert!(
            zero.degraded_reason
                .as_deref()
                .unwrap_or("")
                .contains("zero-container")
        );

        let stopped = parse_container_list_probe(
            r#"[{"id":"main","isDefault":true,"phase":"stopped","sshEnabled":true,"sshPort":2222}]"#,
            "",
            true,
        );
        assert!(
            stopped
                .degraded_reason
                .as_deref()
                .unwrap_or("")
                .contains("stopped")
        );

        let no_ssh = parse_container_list_probe(
            r#"[{"id":"main","isDefault":true,"phase":"running","sshEnabled":false,"sshPort":2222}]"#,
            "",
            true,
        );
        assert!(
            no_ssh
                .degraded_reason
                .as_deref()
                .unwrap_or("")
                .contains("SSH disabled")
        );

        let offline = parse_container_list_probe("", "connect: No such file or directory", false);
        assert!(!offline.daemon_online);
        let err = build_container_required_error("kernel_probe", &offline);
        assert_eq!(err["daemonOnline"], json!(false));
        assert!(err["hint"].as_str().unwrap_or("").contains("start_daemon"));
    }
}
