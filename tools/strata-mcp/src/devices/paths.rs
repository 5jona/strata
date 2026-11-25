//! Per-device SSH ControlMaster / known_hosts runtime paths and config file loaders.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::discovery::{ConfigDevice, SshEntry, home_dir, parse_ssh_config_text};

pub(crate) fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

pub(crate) fn safe_device_key(key: &str) -> String {
    let clean: String = key
        .trim()
        .chars()
        .take(40)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let label = if clean.is_empty() { "default" } else { &clean };
    format!("{label}-{:08x}", (fnv1a(key) & 0xFFFF_FFFF) as u32)
}

pub(crate) fn runtime_ssh_dir() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = base.join("strata-mcp-ssh");
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    dir
}

pub(crate) fn device_control_path(key: &str) -> PathBuf {
    runtime_ssh_dir().join(format!("ctrl-{}", safe_device_key(key)))
}

pub(crate) fn device_known_hosts_path(key: &str) -> PathBuf {
    runtime_ssh_dir().join(format!("known_hosts-{}", safe_device_key(key)))
}

pub(crate) fn load_devices_config() -> HashMap<String, ConfigDevice> {
    let path = std::env::var("STRATA_DEVICES_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".config/strata/devices.json"));
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return HashMap::new();
    };
    let target = val.get("devices").unwrap_or(&val);
    serde_json::from_value(target.clone()).unwrap_or_default()
}

pub(crate) fn load_ssh_entries(extra_hosts: &HashSet<String>) -> Vec<SshEntry> {
    let path = std::env::var("STRATA_SSH_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".ssh/config"));
    std::fs::read_to_string(path)
        .map(|txt| parse_ssh_config_text(&txt, extra_hosts))
        .unwrap_or_default()
}
