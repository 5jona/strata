//! Auto-discovery of `mise`, `uv`, system package managers, and Strata bridge binaries.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use strata_protocol::ContainerConfig;
use strata_protocol::rpc::{ToolInstallSource, ToolManagerHealth, ToolManagerKind};
use strata_sys::rootfs_io::RootDir;

use crate::tools::ToolsManager;
use crate::tools::exec::run_container_command_bounded;

pub(super) const MISE_CANDIDATES: &[&str] = &[
    "/root/.local/bin/mise",
    "/usr/local/bin/mise",
    "/usr/bin/mise",
    "/opt/strata/bin/mise",
    "/root/.cargo/bin/mise",
];

pub(super) const UV_CANDIDATES: &[&str] = &[
    "/root/.local/bin/uv",
    "/root/.local/share/mise/shims/uv",
    "/usr/local/bin/uv",
    "/usr/bin/uv",
    "/opt/strata/bin/uv",
    "/root/.cargo/bin/uv",
];

pub(super) const HOST_EXEC_CANDIDATES: &[&str] = &[
    "/usr/local/bin/host-exec",
    "/usr/bin/host-exec",
    "/opt/strata/bin/host-exec",
];

pub(super) const STRATA_DEBUG_CANDIDATES: &[&str] = &[
    "/usr/local/bin/strata-debug",
    "/usr/bin/strata-debug",
    "/opt/strata/bin/strata-debug",
];

const SYS_PKG_CANDIDATES: &[(&str, &str, ToolManagerKind)] = &[
    ("apt-get", "/usr/bin/apt-get", ToolManagerKind::Apt),
    ("apk", "/sbin/apk", ToolManagerKind::Apk),
    ("pacman", "/usr/bin/pacman", ToolManagerKind::Pacman),
    ("dnf", "/usr/bin/dnf", ToolManagerKind::Dnf),
];

const RUNTIME_PROBE_SCRIPT: &str = r#"
probe() {
  key="$1"; shift
  found=""
  if p=$(command -v "$1" 2>/dev/null); then
    found="$p"
  else
    shift
    for cand in "$@"; do
      if [ -x "$cand" ]; then
        found="$cand"
        break
      fi
    done
  fi
  if [ -n "$found" ]; then
    v=$("$found" --version 2>/dev/null | head -n1 || true)
    printf 'BIN:%s|%s|%s\n' "$key" "$found" "$v"
  fi
}
probe mise mise /root/.local/bin/mise /usr/local/bin/mise /usr/bin/mise /opt/strata/bin/mise /root/.cargo/bin/mise
probe uv uv /root/.local/bin/uv /root/.local/share/mise/shims/uv /usr/local/bin/uv /usr/bin/uv /opt/strata/bin/uv /root/.cargo/bin/uv
probe apt-get apt-get /usr/bin/apt-get
probe apk apk /sbin/apk /usr/sbin/apk
probe pacman pacman /usr/bin/pacman
probe dnf dnf /usr/bin/dnf
probe host-exec host-exec /usr/local/bin/host-exec /usr/bin/host-exec /opt/strata/bin/host-exec
probe strata-debug strata-debug /usr/local/bin/strata-debug /usr/bin/strata-debug /opt/strata/bin/strata-debug
"#;

/// Classifies a binary path into user-installed, Strata-managed, or system origin.
pub(super) fn classify_source(bin_path: &str) -> ToolInstallSource {
    if bin_path.starts_with("/opt/strata/") {
        ToolInstallSource::StrataManaged
    } else if bin_path.starts_with("/root/")
        || bin_path.starts_with("/home/")
        || bin_path.starts_with("/usr/local/")
    {
        ToolInstallSource::UserInstalled
    } else {
        ToolInstallSource::System
    }
}

/// Finds the first existing candidate binary path inside an opened container rootfs.
pub(super) fn find_bin_in_root(root: &RootDir, candidates: &[&str]) -> Option<String> {
    for abs in candidates {
        let rel = abs.trim_start_matches('/');
        if root.open_file_ro_nofollow(Path::new(rel)).is_ok() {
            return Some((*abs).to_owned());
        }
    }
    None
}

/// Static host-side probe snapshot from the mounted rootfs.
#[derive(Debug, Default)]
pub(super) struct HostManagerPaths {
    pub(super) mise: Option<String>,
    pub(super) uv: Option<String>,
    pub(super) apt: Option<String>,
    pub(super) host_exec: Option<String>,
    pub(super) strata_debug: Option<String>,
    pub(super) system_pkg_manager: Option<ToolManagerKind>,
    pub(super) config_paths: Vec<String>,
    pub(super) legacy_mise_installs: bool,
}

pub(super) fn discover_host_managers(root: &RootDir) -> HostManagerPaths {
    let mise = find_bin_in_root(root, MISE_CANDIDATES);
    let uv = find_bin_in_root(root, UV_CANDIDATES);
    let apt = find_bin_in_root(root, &["/usr/bin/apt-get"]);
    let host_exec = find_bin_in_root(root, HOST_EXEC_CANDIDATES);
    let strata_debug = find_bin_in_root(root, STRATA_DEBUG_CANDIDATES);

    let mut system_pkg_manager = None;
    for (_, path, kind) in SYS_PKG_CANDIDATES {
        if root
            .open_file_ro_nofollow(Path::new(path.trim_start_matches('/')))
            .is_ok()
        {
            system_pkg_manager = Some(*kind);
            break;
        }
    }

    let mut config_paths = Vec::new();
    for cand in [
        "/root/.config/mise/config.toml",
        "/etc/strata/mise/config.toml",
        "/workspace/mise.toml",
    ] {
        if root
            .open_file_ro_nofollow(Path::new(cand.trim_start_matches('/')))
            .is_ok()
        {
            config_paths.push(cand.to_owned());
        }
    }

    let legacy_mise_installs = root.dir_has_entries(Path::new("opt/strata/mise/installs"));

    HostManagerPaths {
        mise,
        uv,
        apt,
        host_exec,
        strata_debug,
        system_pkg_manager,
        config_paths,
        legacy_mise_installs,
    }
}

/// Live in-container tool binary discovery result.
#[derive(Debug, Default)]
pub(super) struct RuntimeDiscovery {
    pub(super) bins: BTreeMap<String, (String, Option<String>)>,
    pub(super) system_pkg_manager: Option<ToolManagerKind>,
    pub(super) mise_data_dir: Option<String>,
    pub(super) mise_config_paths: Vec<String>,
}

/// Resolved executable paths for `mise` and `uv`.
#[derive(Debug, Default, Clone)]
pub(super) struct LiveToolBins {
    pub(super) mise: Option<String>,
    pub(super) uv: Option<String>,
}

pub(super) fn health_from_path(path: Option<String>) -> ToolManagerHealth {
    let source = path.as_deref().map(classify_source);
    ToolManagerHealth {
        installed: path.is_some(),
        version: None,
        bin_path: path,
        source,
    }
}

pub(super) fn apply_probe_health(
    health: &mut ToolManagerHealth,
    bins: &BTreeMap<String, (String, Option<String>)>,
    key: &str,
) {
    if let Some((path, ver)) = bins.get(key) {
        health.installed = true;
        health.bin_path = Some(path.clone());
        health.source = Some(classify_source(path));
        if ver.is_some() {
            health.version = ver.clone();
        }
    }
}

/// Probes live binary paths, versions, and `mise` doctor/config directories inside a container.
pub(super) async fn discover_runtime_managers(
    tools: &ToolsManager,
    container: &str,
    ns_pid: u32,
    cfg: &ContainerConfig,
    env: &BTreeMap<String, String>,
) -> RuntimeDiscovery {
    let mut disc = RuntimeDiscovery::default();
    let Ok(out) = run_container_command_bounded(
        &tools.self_exe,
        container,
        ns_pid,
        cfg,
        vec![
            "/bin/sh".to_owned(),
            "-lc".to_owned(),
            RUNTIME_PROBE_SCRIPT.to_owned(),
        ],
        env.clone(),
        Duration::from_secs(6),
    )
    .await
    else {
        return disc;
    };

    for line in out.stdout.lines() {
        let Some(rest) = line.strip_prefix("BIN:") else {
            continue;
        };
        let mut parts = rest.splitn(3, '|');
        let (Some(key), Some(path), Some(ver)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let ver_opt = (!ver.trim().is_empty()).then(|| ver.trim().to_owned());
        disc.bins
            .insert(key.to_owned(), (path.trim().to_owned(), ver_opt));
    }

    for (key, _, kind) in SYS_PKG_CANDIDATES {
        if disc.bins.contains_key(*key) {
            disc.system_pkg_manager = Some(*kind);
            break;
        }
    }

    if let Some((mise_bin, _)) = disc.bins.get("mise").cloned() {
        let mut cfg_set = BTreeSet::new();
        if let Ok(doc) = run_container_command_bounded(
            &tools.self_exe,
            container,
            ns_pid,
            cfg,
            vec![mise_bin.clone(), "doctor".to_owned(), "--json".to_owned()],
            env.clone(),
            Duration::from_secs(6),
        )
        .await
            && let Ok(val) = serde_json::from_str::<Value>(doc.stdout.trim())
        {
            if let Some(data_dir) = val
                .get("dirs")
                .and_then(|d| d.get("data"))
                .and_then(Value::as_str)
            {
                disc.mise_data_dir = Some(data_dir.to_owned());
            }
            if let Some(files) = val.get("config_files").and_then(Value::as_array) {
                for f in files.iter().filter_map(Value::as_str) {
                    cfg_set.insert(f.to_owned());
                }
            }
        }
        if let Ok(tracked) = run_container_command_bounded(
            &tools.self_exe,
            container,
            ns_pid,
            cfg,
            vec![
                mise_bin,
                "config".to_owned(),
                "ls".to_owned(),
                "--tracked-configs".to_owned(),
                "--json".to_owned(),
            ],
            env.clone(),
            Duration::from_secs(5),
        )
        .await
            && let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(tracked.stdout.trim())
        {
            for item in arr {
                if let Some(p) = item.get("path").and_then(Value::as_str) {
                    cfg_set.insert(p.to_owned());
                }
            }
        }
        disc.mise_config_paths = cfg_set.into_iter().collect();
    }

    disc
}

/// Fast resolution of `mise`, `uv`, and system package manager binaries inside a running container.
pub(super) async fn resolve_live_tool_bins(
    tools: &ToolsManager,
    container: &str,
    ns_pid: u32,
    cfg: &ContainerConfig,
    env: &BTreeMap<String, String>,
) -> LiveToolBins {
    let Ok(out) = run_container_command_bounded(
        &tools.self_exe,
        container,
        ns_pid,
        cfg,
        vec![
            "/bin/sh".to_owned(),
            "-lc".to_owned(),
            RUNTIME_PROBE_SCRIPT.to_owned(),
        ],
        env.clone(),
        Duration::from_secs(5),
    )
    .await
    else {
        return LiveToolBins::default();
    };
    let mut bins = BTreeMap::new();
    for line in out.stdout.lines() {
        if let Some(rest) = line.strip_prefix("BIN:") {
            let mut parts = rest.splitn(3, '|');
            if let (Some(k), Some(p)) = (parts.next(), parts.next()) {
                bins.insert(k.to_owned(), p.trim().to_owned());
            }
        }
    }
    LiveToolBins {
        mise: bins.get("mise").cloned(),
        uv: bins.get("uv").cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_source_distinguishes_user_strata_and_system_paths() {
        assert_eq!(
            classify_source("/root/.local/bin/mise"),
            ToolInstallSource::UserInstalled
        );
        assert_eq!(
            classify_source("/usr/local/bin/host-exec"),
            ToolInstallSource::UserInstalled
        );
        assert_eq!(
            classify_source("/opt/strata/bin/mise"),
            ToolInstallSource::StrataManaged
        );
        assert_eq!(
            classify_source("/usr/bin/apt-get"),
            ToolInstallSource::System
        );
    }
}
