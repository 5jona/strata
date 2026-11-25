//! Atomic synchronization of the static musl `strata-debug` agent binary into containers
//! and cleanup of legacy Python `/usr/local/lib/strata-debug` artifacts.

use std::fs;
use std::path::Path;

use strata_sys::rootfs_io::RootDir;

use crate::error::{RuntimeError, RuntimeResult};

const MODULE_DEBUG_BIN_REL: &str = "bin/strata-debug";
const CONTAINER_DEBUG_BIN_REL: &str = "usr/local/bin/strata-debug";
const LEGACY_PYTHON_DEBUG_DIR_REL: &str = "usr/local/lib/strata-debug";

/// Atomically syncs `<mod_root>/bin/strata-debug` (static musl ELF) into
/// `<mount_dir>/usr/local/bin/strata-debug` (`0755`), and removes any legacy
/// `/usr/local/lib/strata-debug` Python package directory inside the container.
pub(crate) fn sync_strata_debug_binary(
    mod_root: &Path,
    mount_dir: &Path,
    root: &RootDir,
) -> RuntimeResult<()> {
    let src_bin = mod_root.join(MODULE_DEBUG_BIN_REL);
    if src_bin.is_file() {
        let bytes = fs::read(&src_bin).map_err(|source| RuntimeError::Io {
            op: "read module bin/strata-debug",
            source,
        })?;
        root.sync_bytes_if_changed(&bytes, Path::new(CONTAINER_DEBUG_BIN_REL), 0o755)
            .map_err(RuntimeError::from_sys)?;
    }

    cleanup_legacy_python_debug_dir(mount_dir, root);
    Ok(())
}

fn cleanup_legacy_python_debug_dir(mount_dir: &Path, root: &RootDir) {
    let legacy_rel = Path::new(LEGACY_PYTHON_DEBUG_DIR_REL);
    if root.is_dir(legacy_rel) {
        // `root.is_dir` verified via `O_DIRECTORY | O_NOFOLLOW` that no symlinks exist along
        // `usr/local/lib/strata-debug`, so removing the anchored directory cannot escape `mount_dir`.
        let _ = fs::remove_dir_all(mount_dir.join(legacy_rel));
    } else {
        let _ = root.remove_file(legacy_rel);
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("strata-debug-sync-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn syncs_static_binary_with_0755_and_cleans_legacy_python_dir() {
        let base = unique_temp_dir("binary-sync");
        let mod_root = base.join("mod");
        let mnt = base.join("mnt");
        let legacy_dir = mnt.join(LEGACY_PYTHON_DEBUG_DIR_REL);
        let pycache = legacy_dir.join("__pycache__");

        fs::create_dir_all(mod_root.join("bin")).expect("create mod bin");
        fs::create_dir_all(mnt.join("usr/local/bin")).expect("create container bin");
        fs::create_dir_all(&pycache).expect("create legacy pycache");

        let fake_elf = b"\x7fELF\x02\x01\x01\x00strata-debug-musl-v2";
        fs::write(mod_root.join(MODULE_DEBUG_BIN_REL), fake_elf).expect("write binary");
        fs::write(legacy_dir.join("cli.py"), b"print('legacy')\n").expect("write legacy cli.py");
        fs::write(pycache.join("cli.cpython-312.pyc"), b"\x00\x00").expect("write pyc");
        fs::write(
            mnt.join(CONTAINER_DEBUG_BIN_REL),
            b"#!/bin/sh\nexec /usr/bin/python3 /usr/local/lib/strata-debug/cli.py\n",
        )
        .expect("write legacy launcher");

        let root = RootDir::open(&mnt).expect("open root");
        sync_strata_debug_binary(&mod_root, &mnt, &root).expect("sync binary");

        let installed = mnt.join(CONTAINER_DEBUG_BIN_REL);
        assert!(installed.is_file());
        assert_eq!(fs::read(&installed).expect("read installed"), fake_elf);
        let mode = fs::metadata(&installed)
            .expect("stat installed")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o755);
        assert!(!legacy_dir.exists());

        // Second sync is idempotent when bytes already match.
        sync_strata_debug_binary(&mod_root, &mnt, &root).expect("idempotent sync");
        assert_eq!(fs::read(&installed).expect("read after resync"), fake_elf);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn cleans_legacy_python_dir_even_when_module_binary_absent() {
        let base = unique_temp_dir("cleanup-only");
        let mod_root = base.join("mod");
        let mnt = base.join("mnt");
        let legacy_dir = mnt.join(LEGACY_PYTHON_DEBUG_DIR_REL);

        fs::create_dir_all(&mod_root).expect("create mod root");
        fs::create_dir_all(&legacy_dir).expect("create legacy dir");
        fs::write(legacy_dir.join("cli.py"), b"legacy\n").expect("write legacy");

        let root = RootDir::open(&mnt).expect("open root");
        sync_strata_debug_binary(&mod_root, &mnt, &root).expect("cleanup sync");

        assert!(!legacy_dir.exists());
        assert!(!mnt.join(CONTAINER_DEBUG_BIN_REL).exists());

        let _ = fs::remove_dir_all(&base);
    }
}
