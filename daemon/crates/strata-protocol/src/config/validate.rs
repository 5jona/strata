//! Strict validation rules for container identifiers, paths, hostnames, and cross-container conflicts.

use std::collections::BTreeSet;

use thiserror::Error;

#[path = "validate_helpers.rs"]
mod helpers;

use helpers::{
    is_disallowed_mount_host_path, is_safe_char_dev_name, is_valid_hostname, normalize_abs_slashes,
    validate_abs_path,
};

use crate::config::{ContainerConfig, StrataConfig};
use crate::constants::CONFIG_SCHEMA_VERSION;

const MAX_SERVICE_SPEC_BYTES: usize = 32_768;

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
    #[error("web console has invalid port {0}")]
    InvalidWebPort(u16),
    #[error("container '{container}' has invalid service '{service_id}': {reason}")]
    InvalidService {
        container: String,
        service_id: String,
        reason: &'static str,
    },
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
    if cfg.web.enabled && cfg.web.port == 0 {
        return Err(ConfigValidationError::InvalidWebPort(cfg.web.port));
    }
    if let Some(default_id) = cfg.default_container.as_deref() {
        validate_container_id(default_id)?;
        if !cfg.containers.contains_key(default_id) {
            return Err(ConfigValidationError::MissingDefaultContainer(
                default_id.to_owned(),
            ));
        }
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

/// Validates a single container's paths, hostname, OOM score, SSH port, char devices, mounts, and services.
pub fn validate_container_config(
    id: &str,
    cfg: &ContainerConfig,
) -> Result<(), ConfigValidationError> {
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
        if is_disallowed_mount_host_path(bm) {
            return Err(ConfigValidationError::InvalidPath {
                container: id.to_owned(),
                path: bm.host_path.clone(),
                reason: "disallowed sensitive host mount path (/dev, /proc, /sys, /data/adb)",
            });
        }
        if normalize_abs_slashes(&bm.container_path) == "/" {
            return Err(ConfigValidationError::InvalidPath {
                container: id.to_owned(),
                path: bm.container_path.clone(),
                reason: "container mount target must not be '/'",
            });
        }
    }
    let mut seen_services = BTreeSet::new();
    for svc in &cfg.services {
        validate_service_spec(id, svc)?;
        if !seen_services.insert(svc.id.as_str()) {
            return Err(ConfigValidationError::InvalidService {
                container: id.to_owned(),
                service_id: svc.id.clone(),
                reason: "duplicate service id",
            });
        }
    }
    Ok(())
}

/// Validates a single managed container background service specification.
pub fn validate_service_spec(
    container: &str,
    svc: &crate::config::ContainerServiceSpec,
) -> Result<(), ConfigValidationError> {
    if validate_container_id(&svc.id).is_err() {
        return Err(ConfigValidationError::InvalidService {
            container: container.to_owned(),
            service_id: svc.id.clone(),
            reason: "service id must be 2..=32 ASCII [a-z0-9-] chars starting with letter",
        });
    }
    if svc.name.trim().is_empty() || svc.name.len() > 64 {
        return Err(ConfigValidationError::InvalidService {
            container: container.to_owned(),
            service_id: svc.id.clone(),
            reason: "service name must be 1..=64 characters",
        });
    }
    if svc.command.is_empty() || svc.command[0].trim().is_empty() {
        return Err(ConfigValidationError::InvalidService {
            container: container.to_owned(),
            service_id: svc.id.clone(),
            reason: "service command must not be empty",
        });
    }
    for arg in &svc.command {
        if arg.contains('\0') {
            return Err(ConfigValidationError::InvalidService {
                container: container.to_owned(),
                service_id: svc.id.clone(),
                reason: "service command must not contain NUL bytes",
            });
        }
    }
    if let Some(wd) = &svc.workdir {
        validate_abs_path(container, wd)?;
    }
    for (k, v) in &svc.env {
        if k.is_empty() || k.contains('=') || k.contains('\0') || v.contains('\0') {
            return Err(ConfigValidationError::InvalidService {
                container: container.to_owned(),
                service_id: svc.id.clone(),
                reason: "invalid environment variable key or value",
            });
        }
    }
    let spec_bytes: usize = svc.command.iter().map(String::len).sum::<usize>()
        + svc.workdir.as_ref().map_or(0, String::len)
        + svc
            .env
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>();
    if spec_bytes > MAX_SERVICE_SPEC_BYTES {
        return Err(ConfigValidationError::InvalidService {
            container: container.to_owned(),
            service_id: svc.id.clone(),
            reason: "service command and env exceed 32 KiB limit",
        });
    }
    Ok(())
}
