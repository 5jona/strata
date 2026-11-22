//! Synchronization and stale-artifact pruning for the in-container `strata-debug` Python package.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::error::{RuntimeError, RuntimeResult};
use crate::fs_atomic::{ensure_dir_mode, sync_bytes_if_changed};

const STRATA_DEBUG_LAUNCHER: &[u8] =
    b"#!/bin/sh\nexec /usr/bin/python3 /usr/local/lib/strata-debug/cli.py \"$@\"\n";

/// Syncs `.py` files from `<mod_root>/container/strata-debug/` into
/// `<mount_dir>/usr/local/lib/strata-debug/`, pruning stale `.py` files,
/// `.pyc` files, and `__pycache__` directories in the destination.
pub(crate) fn sync_strata_debug_package(mod_root: &Path, mount_dir: &Path) -> RuntimeResult<()> {
    let src_dir = mod_root.join("container/strata-debug");
    if !src_dir.is_dir() {
        return Ok(());
    }
    let dst_dir = mount_dir.join("usr/local/lib/strata-debug");
    ensure_dir_mode(&dst_dir, 0o755)?;

    let entries = fs::read_dir(&src_dir).map_err(|source| RuntimeError::Io {
        op: "read strata-debug dir",
        source,
    })?;

    let mut expected_py = BTreeSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".py") {
            continue;
        }
        if let Ok(bytes) = fs::read(&path) {
            let mode = if name == "cli.py" || name == "forward_bridge.py" {
                0o755
            } else {
                0o644
            };
            sync_bytes_if_changed(&bytes, &dst_dir.join(name), mode)?;
            expected_py.insert(name.to_owned());
        }
    }

    prune_stale_debug_artifacts(&dst_dir, &expected_py)?;

    if !expected_py.is_empty() {
        sync_bytes_if_changed(
            STRATA_DEBUG_LAUNCHER,
            &mount_dir.join("usr/local/bin/strata-debug"),
            0o755,
        )?;
    }
    Ok(())
}

fn prune_stale_debug_artifacts(
    dst_dir: &Path,
    expected_py: &BTreeSet<String>,
) -> RuntimeResult<()> {
    let entries = fs::read_dir(dst_dir).map_err(|source| RuntimeError::Io {
        op: "read destination strata-debug dir",
        source,
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if name == "__pycache__" && path.is_dir() {
            let _ = fs::remove_dir_all(&path);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        if name.ends_with(".pyc") || name.ends_with(".pyo") {
            let _ = fs::remove_file(&path);
            continue;
        }
        if name.ends_with(".py") && !expected_py.contains(name) {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("strata-debug-sync-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn syncs_py_files_and_prunes_stale_modules_and_pycache() {
        let base = unique_temp_dir("prune");
        let mod_root = base.join("mod");
        let src_dir = mod_root.join("container/strata-debug");
        let mnt = base.join("mnt");
        let dst_dir = mnt.join("usr/local/lib/strata-debug");
        let pycache = dst_dir.join("__pycache__");

        fs::create_dir_all(&src_dir).expect("create src");
        fs::create_dir_all(mnt.join("usr/local/bin")).expect("create bin");
        fs::create_dir_all(&pycache).expect("create pycache");

        fs::write(src_dir.join("cli.py"), b"print('v2')\n").expect("write cli.py");
        fs::write(src_dir.join("notes.txt"), b"ignored\n").expect("write notes.txt");

        // Pre-populate stale artifacts in destination.
        fs::write(dst_dir.join("obsolete.py"), b"print('old')\n").expect("write obsolete.py");
        fs::write(dst_dir.join("stale.pyc"), b"\x00\x00").expect("write stale.pyc");
        fs::write(pycache.join("cli.cpython-312.pyc"), b"\x00\x00").expect("write cached pyc");

        sync_strata_debug_package(&mod_root, &mnt).expect("sync package");

        assert!(dst_dir.join("cli.py").is_file());
        assert!(!dst_dir.join("notes.txt").exists());
        assert!(!dst_dir.join("obsolete.py").exists());
        assert!(!dst_dir.join("stale.pyc").exists());
        assert!(!pycache.exists());
        assert!(mnt.join("usr/local/bin/strata-debug").is_file());

        let _ = fs::remove_dir_all(&base);
    }
}
