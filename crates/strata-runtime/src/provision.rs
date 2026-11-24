//! Container rootfs provisioning, archive streaming, sparse clone, resize, and safe delete.

pub mod archive;
pub mod clone;
pub mod export;
pub mod resize;

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use strata_protocol::{ContainerConfig, validate_container_id};
use strata_sys::rootfs_io::RootDir;

use crate::error::{RuntimeError, RuntimeResult};
use crate::sync;

const COLOROS_APT_SANDBOX_CONF: &str = "APT::Sandbox::User \"root\";\n";

/// Applies ColorOS-safe APT sandbox configuration, runtime DNS/`host.env`/bridge sync,
/// and optional SSH public key injection onto a freshly extracted mounted container rootfs.
pub fn provision_container_rootfs(
    mount_dir: &Path,
    cfg: &ContainerConfig,
    ssh_keys: &[String],
    module_dir: Option<&Path>,
) -> RuntimeResult<()> {
    let root = RootDir::open(mount_dir).map_err(RuntimeError::from_sys)?;

    // Enforce APT::Sandbox::User "root" on Debian/Ubuntu rootfs images to avoid ColorOS watchdog reboots.
    if root.is_dir(Path::new("etc/apt")) {
        root.ensure_dir(Path::new("etc/apt/apt.conf.d"), 0o755)
            .map_err(RuntimeError::from_sys)?;
        root.write_atomic(
            Path::new("etc/apt/apt.conf.d/99strata-sandbox"),
            COLOROS_APT_SANDBOX_CONF.as_bytes(),
            0o644,
        )
        .map_err(RuntimeError::from_sys)?;
    }

    sync::sync_container_runtime_files(mount_dir, cfg, module_dir)?;

    if !ssh_keys.is_empty() {
        let joined = ssh_keys.join("\n");
        let _ = sync::add_ssh_keys(mount_dir, &joined)?;
    }

    Ok(())
}

/// Safely deletes container storage artifacts strictly inside `<containers_root>/<container_id>`.
/// Refuses to delete any path outside the container jail, containing traversal components, or symlinks.
pub fn safe_delete_container_files(
    containers_root: &Path,
    container_id: &str,
    image_path: &Path,
    mount_dir: &Path,
    delete_image: bool,
) -> RuntimeResult<bool> {
    validate_container_id(container_id)
        .map_err(|e| RuntimeError::InvalidArgument(e.to_string()))?;

    let jail_dir = containers_root.join(container_id);
    let mut image_deleted = false;

    if delete_image {
        ensure_within_jail(&jail_dir, image_path)?;
        match fs::symlink_metadata(image_path) {
            Ok(meta) => {
                if !meta.is_file() {
                    return Err(RuntimeError::InvalidArgument(format!(
                        "refusing to delete non-regular or symlink image path '{}'",
                        image_path.display()
                    )));
                }
                fs::remove_file(image_path).map_err(|source| RuntimeError::Io {
                    op: "remove container image",
                    source,
                })?;
                image_deleted = true;
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(source) => {
                return Err(RuntimeError::Io {
                    op: "stat container image for delete",
                    source,
                });
            }
        }
    }

    if ensure_within_jail(&jail_dir, mount_dir).is_ok()
        && fs::symlink_metadata(mount_dir).is_ok_and(|m| m.is_dir())
    {
        let _ = fs::remove_dir(mount_dir);
    }

    if delete_image && fs::symlink_metadata(&jail_dir).is_ok_and(|m| m.is_dir()) {
        let _ = fs::remove_dir(&jail_dir);
    }

    Ok(image_deleted)
}

/// Validates that `candidate` is strictly inside `jail_dir` with no `..` traversal or symlink escape.
pub fn ensure_within_jail(jail_dir: &Path, candidate: &Path) -> RuntimeResult<()> {
    let norm_jail = normalize_abs_lexical(jail_dir)?;
    let norm_cand = normalize_abs_lexical(candidate)?;

    if norm_cand == norm_jail || !norm_cand.starts_with(&norm_jail) {
        return Err(RuntimeError::InvalidArgument(format!(
            "path '{}' escapes container jail '{}'",
            candidate.display(),
            jail_dir.display()
        )));
    }

    if fs::symlink_metadata(jail_dir).is_ok() {
        let real_jail = fs::canonicalize(jail_dir).map_err(|source| RuntimeError::Io {
            op: "canonicalize jail dir",
            source,
        })?;
        if let Some(parent) = candidate.parent()
            && fs::symlink_metadata(parent).is_ok()
        {
            let real_parent = fs::canonicalize(parent).map_err(|source| RuntimeError::Io {
                op: "canonicalize candidate parent",
                source,
            })?;
            if !real_parent.starts_with(&real_jail) {
                return Err(RuntimeError::InvalidArgument(format!(
                    "path '{}' resolves via symlink outside container jail '{}'",
                    candidate.display(),
                    jail_dir.display()
                )));
            }
        }
    }

    Ok(())
}

fn normalize_abs_lexical(path: &Path) -> RuntimeResult<PathBuf> {
    if !path.is_absolute() {
        return Err(RuntimeError::InvalidArgument(format!(
            "expected absolute path, got '{}'",
            path.display()
        )));
    }
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::RootDir => out.push("/"),
            Component::CurDir => {}
            Component::Normal(seg) => out.push(seg),
            Component::ParentDir | Component::Prefix(_) => {
                return Err(RuntimeError::InvalidArgument(format!(
                    "disallowed traversal component in '{}'",
                    path.display()
                )));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn safe_delete_enforces_strict_container_jail() {
        let base = std::env::temp_dir().join(format!("strata-jail-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("containers");
        let lab = root.join("lab");
        let mnt = lab.join("mnt");
        fs::create_dir_all(&mnt).expect("create container dir");
        let img = lab.join("rootfs.img");
        fs::write(&img, b"fake-img").expect("write img");

        let outside = base.join("outside.img");
        fs::write(&outside, b"keep-me").expect("write outside");

        // Must reject deleting anything outside <root>/lab
        assert!(safe_delete_container_files(&root, "lab", &outside, &mnt, true).is_err());
        assert!(outside.is_file());

        // Must reject traversal components
        let tricky = lab.join("../outside.img");
        assert!(safe_delete_container_files(&root, "lab", &tricky, &mnt, true).is_err());
        assert!(outside.is_file());

        // Must reject dangling symlinks inside jail
        let dangling = lab.join("dangling.img");
        symlink(base.join("nonexistent-host-file"), &dangling).expect("dangling symlink");
        assert!(safe_delete_container_files(&root, "lab", &dangling, &mnt, true).is_err());
        let _ = fs::remove_file(&dangling);

        // Must succeed for valid image inside jail
        let deleted = safe_delete_container_files(&root, "lab", &img, &mnt, true).expect("delete");
        assert!(deleted);
        assert!(!img.exists());
        assert!(!lab.exists());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn provisions_coloros_apt_sandbox_and_ssh_keys() {
        let base = std::env::temp_dir().join(format!("strata-prov-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("etc/apt")).expect("create etc/apt");

        let mut cfg = ContainerConfig::default_for_id("lab");
        cfg.sync_android_dns = false;
        let keys = vec!["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestKey user@host".to_owned()];
        provision_container_rootfs(&base, &cfg, &keys, None).expect("provision");

        let apt_conf =
            fs::read_to_string(base.join("etc/apt/apt.conf.d/99strata-sandbox")).expect("apt conf");
        assert_eq!(apt_conf, COLOROS_APT_SANDBOX_CONF);
        assert_eq!(sync::list_ssh_keys(&base).expect("keys"), keys);

        let _ = fs::remove_dir_all(&base);
    }
}
