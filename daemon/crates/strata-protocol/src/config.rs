//! Multi-container configuration schema and strict validation rules.

pub mod validate;

use std::collections::BTreeMap;

pub use crate::generated::config::{
    BindMount, ContainerConfig, ContainerServiceRuntime, ContainerServiceSpec, ServiceDeleteResult,
    ServiceLogsParams, ServiceLogsResult, ServiceRestartPolicy, ServiceSelectorParams,
    ServiceStateChangedPayload, ServiceStatus, ServiceUpsertParams, SideVeilBridgeConfig,
    SshConfig, StrataConfig, WebConsoleConfig,
};
pub use validate::{ConfigValidationError, validate_container_config, validate_container_id};

use crate::capabilities::CapProfile;
use crate::constants::{
    CONFIG_SCHEMA_VERSION, CONTAINER_DROPBEAR_PID_PATH, DEFAULT_CONTAINERS_ROOT,
    DEFAULT_DNS_SERVERS, DEFAULT_EXTRA_CHAR_DEVS, DEFAULT_OOM_SCORE_ADJ, DEFAULT_SIDEVEIL_HUB_SOCK,
    DEFAULT_SSH_KEEPALIVE_SECS, DEFAULT_SSH_LISTEN_ADDR, DEFAULT_SSH_PORT, DEFAULT_WEB_PORT,
    L5_DIAG_MOUNT_PATHS, SIDEVEIL_PLUGIN_ID,
};

impl Default for WebConsoleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            lan_enabled: false,
            port: DEFAULT_WEB_PORT,
        }
    }
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

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_addr: DEFAULT_SSH_LISTEN_ADDR.to_owned(),
            port: DEFAULT_SSH_PORT,
            keepalive_secs: DEFAULT_SSH_KEEPALIVE_SECS,
        }
    }
}

/// Returns the canonical opt-in read-only L5 kernel diagnostic bind mounts.
#[must_use]
pub fn l5_diag_mounts() -> Vec<BindMount> {
    L5_DIAG_MOUNT_PATHS
        .iter()
        .map(|path| BindMount {
            host_path: (*path).to_owned(),
            container_path: (*path).to_owned(),
            read_only: true,
        })
        .collect()
}

impl Default for StrataConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_SCHEMA_VERSION,
            default_container: None,
            sideveil: SideVeilBridgeConfig::default(),
            web: WebConsoleConfig::default(),
            containers: BTreeMap::new(),
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
            oom_score_adj: Some(DEFAULT_OOM_SCORE_ADJ),
            host_bridge: true,
            host_runtime: false,
            bind_data_local_tmp: true,
            extra_char_devs: default_extra_char_devs(),
            extra_mounts: Vec::new(),
            sync_android_dns: true,
            dns_servers: default_dns_list(),
            fsck_on_mount: true,
            noatime: true,
            init_command: Vec::new(),
            services: Vec::new(),
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
            "-e",
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
    /// Resolves an optional container ID to the configured default container (or first container).
    #[must_use]
    pub fn resolve_container_id<'a>(&'a self, requested: Option<&'a str>) -> Option<&'a str> {
        match requested {
            Some(name) if !name.trim().is_empty() => Some(name.trim()),
            _ => self
                .default_container
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| self.containers.keys().next().map(String::as_str)),
        }
    }

    /// Returns `true` if `id` is currently selected as `defaultContainer`.
    #[must_use]
    pub fn is_default_container(&self, id: &str) -> bool {
        self.default_container.as_deref() == Some(id)
    }

    /// Looks up a container configuration by optional ID (defaulting to `default_container`).
    #[must_use]
    pub fn get_container<'a>(
        &'a self,
        requested: Option<&'a str>,
    ) -> Option<(&'a str, &'a ContainerConfig)> {
        let id = self.resolve_container_id(requested)?;
        self.containers
            .get_key_value(id)
            .map(|(k, v)| (k.as_str(), v))
    }

    /// Validates all identifiers, paths, bounds, and cross-container conflict invariants.
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        validate::validate_strata_config(self)
    }
}

fn default_dns_list() -> Vec<String> {
    DEFAULT_DNS_SERVERS
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

fn default_extra_char_devs() -> Vec<String> {
    DEFAULT_EXTRA_CHAR_DEVS
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_with_main() -> StrataConfig {
        let mut cfg = StrataConfig {
            default_container: Some("main".to_owned()),
            ..StrataConfig::default()
        };
        cfg.containers
            .insert("main".to_owned(), ContainerConfig::default_for_id("main"));
        cfg
    }

    #[test]
    fn default_config_starts_empty_and_validates() {
        let cfg = StrataConfig::default();
        assert!(cfg.validate().is_ok());
        assert!(cfg.default_container.is_none());
        assert!(cfg.containers.is_empty());
        assert!(cfg.get_container(None).is_none());
    }

    #[test]
    fn container_default_config_generates_key_only_dropbear() {
        let cfg = sample_with_main();
        assert!(cfg.validate().is_ok());
        let (id, main) = cfg.get_container(None).expect("main container must exist");
        assert_eq!(id, "main");
        assert!(cfg.is_default_container("main"));
        assert_eq!(main.extra_char_devs, vec!["uinput".to_owned()]);
        let init = main.effective_init_command();
        assert_eq!(init[0], "/usr/sbin/dropbear");
        assert!(init.contains(&"-e".to_owned()));
        assert!(init.contains(&"-s".to_owned()));
        assert!(init.contains(&"-g".to_owned()));
        assert!(init.contains(&"0.0.0.0:2222".to_owned()));
    }

    #[test]
    fn effective_init_command_respects_ssh_disabled_and_custom_init() {
        let mut cfg = sample_with_main();
        let main = cfg.containers.get_mut("main").expect("main container");
        main.ssh.enabled = false;
        main.ssh.port = 0;
        assert!(cfg.validate().is_ok());

        let headless_init = cfg.containers["main"].effective_init_command();
        assert_eq!(headless_init[0], "/bin/sh");
        assert!(!headless_init.iter().any(|arg| arg.contains("dropbear")));

        let main = cfg.containers.get_mut("main").expect("main container");
        main.init_command = ["/usr/bin/env", "sleep", "infinity"]
            .map(str::to_owned)
            .to_vec();
        assert_eq!(
            main.effective_init_command(),
            vec!["/usr/bin/env", "sleep", "infinity"]
        );
    }

    #[test]
    fn deserializes_legacy_config_without_web_or_noatime() {
        let legacy = r#"{
            "version": 1,
            "defaultContainer": "main",
            "containers": {
                "main": {
                    "imagePath": "/data/local/strata/containers/main/rootfs.img",
                    "mountDir": "/data/local/strata/containers/main/mnt",
                    "hostname": "strata-main"
                }
            }
        }"#;
        let parsed: StrataConfig = serde_json::from_str(legacy).expect("parse legacy config");
        assert!(parsed.validate().is_ok());
        assert_eq!(parsed.default_container.as_deref(), Some("main"));
        assert_eq!(parsed.web, WebConsoleConfig::default());
        assert!(parsed.containers["main"].noatime);
    }
}
