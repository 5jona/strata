//! Transport route construction (`ssh` -> `adb-ssh` -> `adb-host`) and ControlMaster management.

use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

use crate::devices::{
    DeviceRecord, DeviceRegistry, device_control_path, device_known_hosts_path, is_ip_literal,
};

/// Single candidate transport path (`ssh`, `adb-ssh`, or `adb-host`) to reach a device.
#[derive(Debug, Clone)]
pub(crate) struct TransportRoute {
    pub(crate) kind: &'static str,
    pub(crate) host: String,
    pub(crate) port: Option<u16>,
    pub(crate) user: Option<String>,
    pub(crate) control_path: PathBuf,
    pub(crate) known_hosts_path: PathBuf,
    pub(crate) adb_serial: Option<String>,
    pub(crate) local_port: Option<u16>,
    pub(crate) remote_ssh_port: u16,
}

impl TransportRoute {
    pub(crate) fn target_spec(&self) -> String {
        match self.user.as_deref().filter(|u| !u.is_empty()) {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        }
    }

    pub(crate) fn session_key(&self) -> String {
        match self.kind {
            "adb-host" => format!("adb-host:{}", self.adb_serial.as_deref().unwrap_or("")),
            "adb-ssh" => format!(
                "adb-ssh:{}:{}:{}",
                self.adb_serial.as_deref().unwrap_or(""),
                self.local_port.unwrap_or(0),
                self.remote_ssh_port
            ),
            _ => format!("ssh:{}:{}", self.target_spec(), self.port.unwrap_or(2222)),
        }
    }

    pub(crate) fn with_remote_port(&self, remote_port: u16) -> Self {
        let mut next = self.clone();
        next.remote_ssh_port = remote_port;
        next
    }

    pub(crate) fn base_ssh_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(p) = self.port {
            args.push("-p".to_owned());
            args.push(p.to_string());
        }
        args.extend([
            "-o".to_owned(),
            "BatchMode=yes".to_owned(),
            "-o".to_owned(),
            "ConnectTimeout=8".to_owned(),
            "-o".to_owned(),
            "ServerAliveInterval=15".to_owned(),
            "-o".to_owned(),
            "ServerAliveCountMax=2".to_owned(),
            "-o".to_owned(),
            "StrictHostKeyChecking=accept-new".to_owned(),
            "-o".to_owned(),
            "ControlMaster=auto".to_owned(),
            "-o".to_owned(),
            "ControlPersist=300".to_owned(),
            "-o".to_owned(),
            format!("ControlPath={}", self.control_path.display()),
            "-o".to_owned(),
            format!("UserKnownHostsFile={}", self.known_hosts_path.display()),
            self.target_spec(),
        ]);
        args
    }
}

/// Optional per-call target routing overrides passed by MCP tools.
#[derive(Debug, Clone, Default)]
pub(crate) struct TargetOverride {
    pub(crate) device: Option<String>,
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
}

/// Builds ordered failover transport routes for a tool invocation.
pub(crate) async fn build_routes(
    registry: &DeviceRegistry,
    target: &TargetOverride,
) -> Result<Vec<TransportRoute>, String> {
    if let Some(rec) = registry
        .resolve(target.device.as_deref(), target.host.as_deref())
        .await?
    {
        return routes_for_record(registry, &rec, target.port).await;
    }

    let eff_host = target
        .host
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            std::env::var("STRATA_SSH_HOST")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .ok_or_else(|| "no device discovered and STRATA_SSH_HOST is not set".to_owned())?;
    let eff_port = target
        .port
        .or_else(|| {
            std::env::var("STRATA_SSH_PORT")
                .ok()
                .and_then(|s| s.trim().parse::<u16>().ok())
        })
        .or_else(|| is_ip_literal(&eff_host).then_some(2222));
    let eff_user = std::env::var("STRATA_SSH_USER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| is_ip_literal(&eff_host).then(|| "root".to_owned()));
    let key = format!("host-{eff_host}-{}", eff_port.unwrap_or(0));
    Ok(vec![TransportRoute {
        kind: "ssh",
        host: eff_host,
        port: eff_port,
        user: eff_user,
        control_path: device_control_path(&key),
        known_hosts_path: device_known_hosts_path(&key),
        adb_serial: None,
        local_port: None,
        remote_ssh_port: eff_port.unwrap_or(2222),
    }])
}

async fn routes_for_record(
    registry: &DeviceRegistry,
    rec: &DeviceRecord,
    port_override: Option<u16>,
) -> Result<Vec<TransportRoute>, String> {
    let ssh_ctl = device_control_path(&format!("{}-ssh", rec.id));
    let adb_ctl = device_control_path(&format!("{}-adb", rec.id));
    let kh = device_known_hosts_path(&rec.id);
    let mut ssh_route = None;
    let mut adb_route = None;

    if let Some(host) = rec.ssh_host.as_ref() {
        let p = port_override
            .or(rec.ssh_port)
            .or_else(|| is_ip_literal(host).then_some(2222));
        let u = rec
            .ssh_user
            .clone()
            .or_else(|| is_ip_literal(host).then(|| "root".to_owned()));
        ssh_route = Some(TransportRoute {
            kind: "ssh",
            host: host.clone(),
            port: p,
            user: u,
            control_path: ssh_ctl,
            known_hosts_path: kh.clone(),
            adb_serial: None,
            local_port: None,
            remote_ssh_port: p.unwrap_or(2222),
        });
    }

    let mut adb_host_route = None;
    let serial = rec
        .adb_usb_serial
        .as_deref()
        .or(rec.adb_tcpip_serial.as_deref())
        .or(rec.adb_serial.as_deref());
    if let Some(ser) = serial
        && rec.adb_state.as_deref() == Some("device")
    {
        let local_p = registry.allocate_forward_port(&rec.id).await?;
        let remote_p = port_override.or(rec.ssh_port).unwrap_or(2222);
        adb_route = Some(TransportRoute {
            kind: "adb-ssh",
            host: "127.0.0.1".to_owned(),
            port: Some(local_p),
            user: Some(rec.ssh_user.clone().unwrap_or_else(|| "root".to_owned())),
            control_path: adb_ctl,
            known_hosts_path: kh.clone(),
            adb_serial: Some(ser.to_owned()),
            local_port: Some(local_p),
            remote_ssh_port: remote_p,
        });
        adb_host_route = Some(TransportRoute {
            kind: "adb-host",
            host: "adb-host".to_owned(),
            port: None,
            user: None,
            control_path: PathBuf::new(),
            known_hosts_path: kh,
            adb_serial: Some(ser.to_owned()),
            local_port: None,
            remote_ssh_port: remote_p,
        });
    }

    let mut routes = Vec::new();
    if rec.prefer_transport == "adb" {
        if let Some(r) = adb_route {
            routes.push(r);
        }
        if let Some(r) = ssh_route {
            routes.push(r);
        }
    } else {
        if let Some(r) = ssh_route {
            routes.push(r);
        }
        if let Some(r) = adb_route {
            routes.push(r);
        }
    }
    if let Some(r) = adb_host_route {
        routes.push(r);
    }
    if routes.is_empty() {
        return Err(format!(
            "device '{}' has no usable SSH or online ADB transport",
            rec.id
        ));
    }
    Ok(routes)
}

/// Closes a stale SSH ControlMaster socket cleanly (no-op for `adb-host`).
pub(crate) async fn reset_control_socket(route: &TransportRoute) {
    if route.kind == "adb-host" {
        return;
    }
    let _ = tokio::time::timeout(
        Duration::from_secs(4),
        Command::new("ssh")
            .args([
                "-o",
                &format!("ControlPath={}", route.control_path.display()),
                "-O",
                "exit",
                &route.target_spec(),
            ])
            .output(),
    )
    .await;
    let _ = std::fs::remove_file(&route.control_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_and_adb_routes_use_distinct_control_paths() {
        let ssh_ctl = device_control_path("demo-tab-ssh");
        let adb_ctl = device_control_path("demo-tab-adb");
        assert_ne!(ssh_ctl, adb_ctl);
    }

    #[tokio::test]
    async fn builds_three_tier_routes_ssh_adb_ssh_adb_host() {
        let reg = DeviceRegistry::default();
        let rec = DeviceRecord {
            id: "demo-dev".to_owned(),
            alias: Some("demo-dev".to_owned()),
            model: Some("TESTDEV".to_owned()),
            product: None,
            adb_serial: Some("SYNTHETIC_SER_1".to_owned()),
            adb_state: Some("device".to_owned()),
            adb_transports: vec!["usb".to_owned()],
            adb_usb_serial: Some("SYNTHETIC_SER_1".to_owned()),
            adb_tcpip_serial: None,
            ssh_host: Some("demo-dev".to_owned()),
            ssh_hostname: None,
            ssh_port: Some(2222),
            ssh_user: Some("root".to_owned()),
            prefer_transport: "ssh".to_owned(),
            sources: vec!["adb".to_owned(), "ssh".to_owned()],
        };
        let routes = routes_for_record(&reg, &rec, None).await.expect("routes");
        let kinds: Vec<&str> = routes.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, vec!["ssh", "adb-ssh", "adb-host"]);
    }
}
