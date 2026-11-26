//! Path, hostname, char-device, and bind-mount safety helpers + validation tests.

use crate::config::BindMount;
use crate::config::validate::ConfigValidationError;
use crate::constants::L5_DIAG_MOUNT_PATHS;

pub(super) fn validate_abs_path(container: &str, path: &str) -> Result<(), ConfigValidationError> {
    if !path.starts_with('/') || path.len() < 2 || normalize_abs_slashes(path) == "/" {
        return Err(ConfigValidationError::InvalidPath {
            container: container.to_owned(),
            path: path.to_owned(),
            reason: "must be a non-root absolute path",
        });
    }
    if path.contains('\0') || path.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(ConfigValidationError::InvalidPath {
            container: container.to_owned(),
            path: path.to_owned(),
            reason: "must not contain NUL bytes or '.'/'..' traversal",
        });
    }
    Ok(())
}

pub(super) fn normalize_abs_slashes(path: &str) -> String {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", segs.join("/"))
    }
}

pub(super) fn is_disallowed_mount_host_path(bm: &BindMount) -> bool {
    let norm = normalize_abs_slashes(&bm.host_path);
    if bm.read_only && L5_DIAG_MOUNT_PATHS.contains(&norm.as_str()) {
        return false;
    }
    matches_prefix(&norm, "/dev")
        || matches_prefix(&norm, "/proc")
        || matches_prefix(&norm, "/sys")
        || matches_prefix(&norm, "/data/adb")
}

fn matches_prefix(norm_path: &str, prefix: &str) -> bool {
    norm_path == prefix
        || norm_path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(super) fn is_valid_hostname(name: &str) -> bool {
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

pub(super) fn is_safe_char_dev_name(dev: &str) -> bool {
    if dev.is_empty()
        || dev.starts_with('/')
        || dev.ends_with('/')
        || dev.contains('\\')
        || dev.contains('\0')
    {
        return false;
    }
    let mut segs = dev.split('/');
    for seg in &mut segs {
        if seg.is_empty() || seg == "." || seg == ".." || seg == "block" {
            return false;
        }
    }
    dev.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'))
}

#[cfg(test)]
mod tests {
    use crate::config::validate::{ConfigValidationError, validate_container_id};
    use crate::config::{BindMount, ContainerConfig, StrataConfig, l5_diag_mounts};
    use crate::constants::CONFIG_SCHEMA_VERSION;

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

        let mut cfg = StrataConfig {
            default_container: Some("main".to_owned()),
            ..StrataConfig::default()
        };
        cfg.containers
            .insert("main".to_owned(), ContainerConfig::default_for_id("main"));
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
        for bypass in ["./block/sda", "foo/block/sda", "input/.", "input//event0"] {
            assert!(
                !crate::config::validate::helpers::is_safe_char_dev_name(bypass),
                "expected char dev rejection for '{bypass}'"
            );
        }
    }

    #[test]
    fn validates_extra_char_devs_and_extra_mounts() {
        let mut cfg = StrataConfig {
            default_container: Some("main".to_owned()),
            ..StrataConfig::default()
        };
        cfg.containers
            .insert("main".to_owned(), ContainerConfig::default_for_id("main"));
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
            ("/dev", "/mnt/d"),
            ("/dev/block", "/mnt/b"),
            ("/dev/block/sda", "/mnt/b"),
            ("/proc", "/mnt/p"),
            ("/sys", "/mnt/s"),
            ("/data/adb", "/mnt/adb"),
            ("/data/adb/strata/run", "/mnt/run"),
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
