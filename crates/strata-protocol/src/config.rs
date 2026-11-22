//! Multi-container configuration schema and strict validation rules.

pub mod validate;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use validate::{ConfigValidationError, validate_container_id};

use crate::capabilities::CapProfile;
use crate::constants::{
    CONFIG_SCHEMA_VERSION, CONTAINER_DROPBEAR_PID_PATH, DEFAULT_CONTAINER_ID,
    DEFAULT_CONTAINERS_ROOT, DEFAULT_DNS_SERVERS, DEFAULT_SIDEVEIL_HUB_SOCK, SIDEVEIL_PLUGIN_ID,
};

/// Top-level multi-container Strata configuration persisted at `/data/adb/strata/config.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrataConfig {
    #[serde(default = "default_schema_version")]
    pub version: u32,
    #[serde(default = "default_container_name")]
    pub default_container: String,
    #[serde(default)]
    pub sideveil: SideVeilBridgeConfig,
    #[serde(default = "default_containers_map")]
    pub containers: BTreeMap<String, ContainerConfig>,
}

/// Optional SideVeil `uds-native` plugin bridge configuration.
/// Silently retries in the background when SideVeil is absent so Strata stays 100% standalone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SideVeilBridgeConfig {
    pub enabled: bool,
    pub hub_sock: String,
    pub plugin_id: String,
}

impl Default for SideVeilBridgeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hub_sock: DEFAULT_SIDEVEIL_HUB_SOCK.to_owned(),
            plugin_id: SIDEVEIL_PLUGIN_ID.to_owned(),
        }
    }
}

/// Dropbear SSH listener settings for a container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SshConfig {
    pub enabled: bool,
    pub listen_addr: String,
    pub port: u16,
    pub keepalive_secs: u16,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_addr: "0.0.0.0".to_owned(),
            port: 2222,
            keepalive_secs: 60,
        }
    }
}

/// Extra host-to-container bind mount entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindMount {
    pub host_path: String,
    pub container_path: String,
    #[serde(default)]
    pub read_only: bool,
}

/// Per-container runtime, storage, isolation, and networking specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_start: bool,
    pub image_path: String,
    pub mount_dir: String,
    pub hostname: String,
    #[serde(default)]
    pub ssh: SshConfig,
    #[serde(default = "default_oom_score_adj")]
    pub oom_score_adj: Option<i16>,
    #[serde(default = "default_true")]
    pub host_bridge: bool,
    #[serde(default)]
    pub host_runtime: bool,
    #[serde(default = "default_true")]
    pub bind_data_local_tmp: bool,
    #[serde(default)]
    pub extra_char_devs: Vec<String>,
    #[serde(default)]
    pub extra_mounts: Vec<BindMount>,
    #[serde(default = "default_true")]
    pub sync_android_dns: bool,
    #[serde(default = "default_dns_list")]
    pub dns_servers: Vec<String>,
    #[serde(default = "default_true")]
    pub fsck_on_mount: bool,
    #[serde(default)]
    pub init_command: Vec<String>,
}

impl Default for StrataConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_SCHEMA_VERSION,
            default_container: DEFAULT_CONTAINER_ID.to_owned(),
            sideveil: SideVeilBridgeConfig::default(),
            containers: default_containers_map(),
        }
    }
}

impl ContainerConfig {
    /// Constructs a neutral default configuration for the given container ID.
    #[must_use]
    pub fn default_for_id(id: &str) -> Self {
        Self {
            enabled: true,
            auto_start: true,
            image_path: format!("{DEFAULT_CONTAINERS_ROOT}/{id}/rootfs.img"),
            mount_dir: format!("{DEFAULT_CONTAINERS_ROOT}/{id}/mnt"),
            hostname: format!("strata-{id}"),
            ssh: SshConfig::default(),
            oom_score_adj: default_oom_score_adj(),
            host_bridge: true,
            host_runtime: false,
            bind_data_local_tmp: true,
            extra_char_devs: Vec::new(),
            extra_mounts: Vec::new(),
            sync_android_dns: true,
            dns_servers: default_dns_list(),
            fsck_on_mount: true,
            init_command: Vec::new(),
        }
    }

    /// Resolves the effective Linux capability profile for this container.
    #[must_use]
    pub fn cap_profile(&self) -> CapProfile {
        if self.host_runtime {
            CapProfile::FullHostRuntime
        } else if self.host_bridge {
            CapProfile::HostBridge
        } else {
            CapProfile::Isolated
        }
    }

    /// Returns the init command: explicit `init_command`, foreground key-only Dropbear when
    /// `ssh.enabled` is true, or a signal-aware idle shell holder when SSH is disabled.
    #[must_use]
    pub fn effective_init_command(&self) -> Vec<String> {
        if !self.init_command.is_empty() {
            return self.init_command.clone();
        }
        if !self.ssh.enabled {
            return vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "trap 'exit 0' TERM INT; while :; do sleep 3600 & wait $!; done".to_owned(),
            ];
        }
        let keepalive = self.ssh.keepalive_secs.to_string();
        let bind = format!("{}:{}", self.ssh.listen_addr, self.ssh.port);
        [
            "/usr/sbin/dropbear",
            "-F",
            "-E",
            "-s",
            "-g",
            "-m",
            "-K",
            &keepalive,
            "-P",
            CONTAINER_DROPBEAR_PID_PATH,
            "-r",
            "/etc/dropbear/dropbear_ed25519_host_key",
            "-r",
            "/etc/dropbear/dropbear_rsa_host_key",
            "-p",
            &bind,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }
}

impl StrataConfig {
    /// Resolves an optional container ID to the configured default container name.
    #[must_use]
    pub fn resolve_container_id<'a>(&'a self, requested: Option<&'a str>) -> &'a str {
        match requested {
            Some(name) if !name.trim().is_empty() => name.trim(),
            _ => self.default_container.as_str(),
        }
    }

    /// Looks up a container configuration by optional ID (defaulting to `default_container`).
    #[must_use]
    pub fn get_container<'a>(
        &'a self,
        requested: Option<&'a str>,
    ) -> Option<(&'a str, &'a ContainerConfig)> {
        let id = self.resolve_container_id(requested);
        self.containers
            .get_key_value(id)
            .map(|(k, v)| (k.as_str(), v))
    }

    /// Validates all identifiers, paths, bounds, and cross-container conflict invariants.
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        validate::validate_strata_config(self)
    }
}

fn default_schema_version() -> u32 {
    CONFIG_SCHEMA_VERSION
}

fn default_container_name() -> String {
    DEFAULT_CONTAINER_ID.to_owned()
}

fn default_containers_map() -> BTreeMap<String, ContainerConfig> {
    let mut map = BTreeMap::new();
    map.insert(
        DEFAULT_CONTAINER_ID.to_owned(),
        ContainerConfig::default_for_id(DEFAULT_CONTAINER_ID),
    );
    map
}

fn default_true() -> bool {
    true
}

fn default_oom_score_adj() -> Option<i16> {
    Some(-300)
}

fn default_dns_list() -> Vec<String> {
    DEFAULT_DNS_SERVERS
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_and_generates_key_only_dropbear() {
        let cfg = StrataConfig::default();
        assert!(cfg.validate().is_ok());
        let (id, main) = cfg.get_container(None).expect("main container must exist");
        assert_eq!(id, "main");
        let init = main.effective_init_command();
        assert_eq!(init[0], "/usr/sbin/dropbear");
        assert!(init.contains(&"-s".to_owned()));
        assert!(init.contains(&"-g".to_owned()));
        assert!(init.contains(&"0.0.0.0:2222".to_owned()));
    }
}
