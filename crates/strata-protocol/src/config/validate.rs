//! Strict validation rules for container identifiers, paths, hostnames, and cross-container conflicts.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::config::{ContainerConfig, StrataConfig};
use crate::constants::CONFIG_SCHEMA_VERSION;

/// Errors returned when validating a `StrataConfig`.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigValidationError {
    #[error("invalid schema version {0}")]
    InvalidSchemaVersion(u32),
    #[error(
        "invalid container id '{0}': must be 2..=32 ASCII [a-z0-9-] chars starting with letter"
    )]
    InvalidContainerId(String),
    #[error("defaultContainer '{0}' is not present in containers map")]
    MissingDefaultContainer(String),
    #[error("container '{container}' has invalid path '{path}': {reason}")]
    InvalidPath {
        container: String,
        path: String,
        reason: &'static str,
    },
    #[error("container '{container}' has invalid hostname '{hostname}'")]
    InvalidHostname { container: String, hostname: String },
    #[error("container '{container}' has invalid oomScoreAdj {value} (expected -1000..=1000)")]
    InvalidOomScoreAdj { container: String, value: i32 },
    #[error("container '{container}' has invalid ssh port {port}")]
    InvalidSshPort { container: String, port: u16 },
    #[error("container '{container}' has disallowed character device '{dev}'")]
    InvalidCharDev { container: String, dev: String },
    #[error("duplicate imagePath '{0}' across multiple containers")]
    DuplicateImagePath(String),
    #[error("duplicate mountDir '{0}' across multiple containers")]
    DuplicateMountDir(String),
    #[error("multiple auto-start containers share SSH bind {listen_addr}:{port}")]
    DuplicateSshPort { listen_addr: String, port: u16 },
}

/// Validates a container identifier for safe use in cgroups, hostnames, and filenames.
pub fn validate_container_id(id: &str) -> Result<(), ConfigValidationError> {
    let bytes = id.as_bytes();
    if bytes.len() < 2 || bytes.len() > 32 {
        return Err(ConfigValidationError::InvalidContainerId(id.to_owned()));
    }
    if !bytes[0].is_ascii_lowercase() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return Err(ConfigValidationError::InvalidContainerId(id.to_owned()));
    }
    let mut prev_dash = false;
    for &b in bytes {
        let is_dash = b == b'-';
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || is_dash) {
            return Err(ConfigValidationError::InvalidContainerId(id.to_owned()));
        }
        if is_dash && prev_dash {
            return Err(ConfigValidationError::InvalidContainerId(id.to_owned()));
        }
        prev_dash = is_dash;
    }
    Ok(())
}

/// Validates a complete `StrataConfig`, checking every container and cross-container uniqueness.
pub fn validate_strata_config(cfg: &StrataConfig) -> Result<(), ConfigValidationError> {
    if cfg.version != CONFIG_SCHEMA_VERSION {
        return Err(ConfigValidationError::InvalidSchemaVersion(cfg.version));
    }
    validate_container_id(&cfg.default_container)?;
    if !cfg.containers.contains_key(&cfg.default_container) {
        return Err(ConfigValidationError::MissingDefaultContainer(
            cfg.default_container.clone(),
        ));
    }

    let mut seen_images = BTreeSet::new();
    let mut seen_mounts = BTreeSet::new();
    let mut seen_ssh_binds = BTreeSet::new();

    for (id, entry) in &cfg.containers {
        validate_container_id(id)?;
        validate_container_config(id, entry)?;

        if !seen_images.insert(entry.image_path.as_str()) {
            return Err(ConfigValidationError::DuplicateImagePath(
                entry.image_path.clone(),
            ));
        }
        if !seen_mounts.insert(entry.mount_dir.as_str()) {
            return Err(ConfigValidationError::DuplicateMountDir(
                entry.mount_dir.clone(),
            ));
        }
        if entry.enabled && entry.auto_start && entry.ssh.enabled && entry.init_command.is_empty() {
            let bind = (entry.ssh.listen_addr.as_str(), entry.ssh.port);
            if !seen_ssh_binds.insert(bind) {
                return Err(ConfigValidationError::DuplicateSshPort {
                    listen_addr: entry.ssh.listen_addr.clone(),
                    port: entry.ssh.port,
                });
            }
        }
    }
    Ok(())
}

fn validate_container_config(id: &str, cfg: &ContainerConfig) -> Result<(), ConfigValidationError> {
    validate_abs_path(id, &cfg.image_path)?;
    validate_abs_path(id, &cfg.mount_dir)?;
    if !is_valid_hostname(&cfg.hostname) {
        return Err(ConfigValidationError::InvalidHostname {
            container: id.to_owned(),
            hostname: cfg.hostname.clone(),
        });
    }
    if let Some(adj) = cfg.oom_score_adj
        && !(-1000..=1000).contains(&adj)
    {
        return Err(ConfigValidationError::InvalidOomScoreAdj {
            container: id.to_owned(),
            value: adj,
        });
    }
    if cfg.ssh.enabled && cfg.ssh.port == 0 {
        return Err(ConfigValidationError::InvalidSshPort {
            container: id.to_owned(),
            port: cfg.ssh.port,
        });
    }
    for dev in &cfg.extra_char_devs {
        if !is_safe_char_dev_name(dev) {
            return Err(ConfigValidationError::InvalidCharDev {
                container: id.to_owned(),
                dev: dev.clone(),
            });
        }
    }
    for bm in &cfg.extra_mounts {
        validate_abs_path(id, &bm.host_path)?;
        validate_abs_path(id, &bm.container_path)?;
        if is_disallowed_mount_host_path(&bm.host_path) {
            return Err(ConfigValidationError::InvalidPath {
                container: id.to_owned(),
                path: bm.host_path.clone(),
                reason: "must not bind-mount /dev/block paths",
            });
        }
        if bm.container_path == "/" {
            return Err(ConfigValidationError::InvalidPath {
                container: id.to_owned(),
                path: bm.container_path.clone(),
                reason: "container mount target must not be '/'",
            });
        }
    }
    Ok(())
}

fn validate_abs_path(container: &str, path: &str) -> Result<(), ConfigValidationError> {
    if !path.starts_with('/') || path.len() < 2 {
        return Err(ConfigValidationError::InvalidPath {
            container: container.to_owned(),
            path: path.to_owned(),
            reason: "must be a non-root absolute path",
        });
    }
    if path.contains('\0') || path.split('/').any(|seg| seg == "..") {
        return Err(ConfigValidationError::InvalidPath {
            container: container.to_owned(),
            path: path.to_owned(),
            reason: "must not contain NUL bytes or '..' traversal",
        });
    }
    Ok(())
}

fn is_disallowed_mount_host_path(path: &str) -> bool {
    let trimmed = path.trim_end_matches('/');
    trimmed == "/dev/block" || trimmed.starts_with("/dev/block/")
}

fn is_valid_hostname(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 {
        return false;
    }
    if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return false;
    }
    bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}

fn is_safe_char_dev_name(dev: &str) -> bool {
    if dev.is_empty()
        || dev.starts_with('/')
        || dev.starts_with("block")
        || dev.contains("..")
        || dev.contains('\\')
        || dev.contains('\0')
    {
        return false;
    }
    dev.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_ids_traversal_and_port_collisions() {
        assert!(validate_container_id("main").is_ok());
        assert!(validate_container_id("dev-box-2").is_ok());
        assert!(validate_container_id("../evil").is_err());
        assert!(validate_container_id("Main").is_err());
        assert!(validate_container_id("a--b").is_err());

        let bad_ver = StrataConfig {
            version: CONFIG_SCHEMA_VERSION + 1,
            ..StrataConfig::default()
        };
        assert!(matches!(
            bad_ver.validate(),
            Err(ConfigValidationError::InvalidSchemaVersion(v)) if v == CONFIG_SCHEMA_VERSION + 1
        ));

        let mut cfg = StrataConfig::default();
        let mut second = ContainerConfig::default_for_id("lab");
        second.ssh.port = 2222;
        cfg.containers.insert("lab".to_owned(), second);
        assert!(matches!(
            cfg.validate(),
            Err(ConfigValidationError::DuplicateSshPort { port: 2222, .. })
        ));

        if let Some(lab) = cfg.containers.get_mut("lab") {
            lab.ssh.port = 2223;
            lab.extra_char_devs.push("block/sda".to_owned());
        }
        assert!(matches!(
            cfg.validate(),
            Err(ConfigValidationError::InvalidCharDev { .. })
        ));
    }

    #[test]
    fn validates_extra_char_devs_and_extra_mounts() {
        use crate::config::{BindMount, l5_diag_mounts};

        let mut cfg = StrataConfig::default();
        let main = cfg.containers.get_mut("main").expect("main container");
        main.extra_char_devs = vec!["uinput".to_owned(), "input/event0".to_owned()];
        main.extra_mounts = l5_diag_mounts();
        main.extra_mounts.push(BindMount {
            host_path: "/data/local/tmp/shared".to_owned(),
            container_path: "/mnt/shared".to_owned(),
            read_only: true,
        });
        assert!(cfg.validate().is_ok());

        for (host, dst) in [
            ("/dev/block", "/mnt/b"),
            ("/dev/block/sda", "/mnt/b"),
            ("/data/local/tmp", "/"),
            ("/data/local/tmp", "/mnt/../escape"),
        ] {
            cfg.containers.get_mut("main").expect("main").extra_mounts = vec![BindMount {
                host_path: host.to_owned(),
                container_path: dst.to_owned(),
                read_only: true,
            }];
            assert!(matches!(
                cfg.validate(),
                Err(ConfigValidationError::InvalidPath { .. })
            ));
        }
    }
}
