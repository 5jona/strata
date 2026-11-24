//! Synchronization and stale-artifact pruning for the in-container `strata-debug` Python package.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use strata_sys::rootfs_io::RootDir;

use crate::error::{RuntimeError, RuntimeResult};

const STRATA_DEBUG_LAUNCHER: &[u8] =
    b"#!/bin/sh\nexec /usr/bin/python3 /usr/local/lib/strata-debug/cli.py \"$@\"\n";

/// Syncs `.py` files from `<mod_root>/container/strata-debug/` into
/// `<mount_dir>/usr/local/lib/strata-debug/`, pruning stale `.py` files,
/// `.pyc` files, and `__pycache__` directories in the destination safely within `root`.
pub(crate) fn sync_strata_debug_package(
    mod_root: &Path,
    mount_dir: &Path,
    root: &RootDir,
) -> RuntimeResult<()> {
    let src_dir = mod_root.join("container/strata-debug");
    if !src_dir.is_dir() {
        return Ok(());
    }
    let rel_dst = Path::new("usr/local/lib/strata-debug");
    root.ensure_dir(rel_dst, 0o755)
        .map_err(RuntimeError::from_sys)?;

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
            root.sync_bytes_if_changed(&bytes, &rel_dst.join(name), mode)
                .map_err(RuntimeError::from_sys)?;
            expected_py.insert(name.to_owned());
        }
    }

    // Only prune inside `mount_dir/usr/local/lib/strata-debug` after `root.ensure_dir` proved no symlinks.
    prune_stale_debug_artifacts(&mount_dir.join(rel_dst), root, rel_dst, &expected_py)?;

    if !expected_py.is_empty() {
        root.sync_bytes_if_changed(
            STRATA_DEBUG_LAUNCHER,
            Path::new("usr/local/bin/strata-debug"),
            0o755,
        )
        .map_err(RuntimeError::from_sys)?;
    }
    Ok(())
}

fn prune_stale_debug_artifacts(
    dst_dir: &Path,
    root: &RootDir,
    rel_dst: &Path,
    expected_py: &BTreeSet<String>,
) -> RuntimeResult<()> {
    let entries = fs::read_dir(dst_dir).map_err(|source| RuntimeError::Io {
        op: "read destination strata-debug dir",
        source,
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let rel_item = rel_dst.join(name);

        if name == "__pycache__" && meta.is_dir() {
            let _ = fs::remove_dir_all(&path);
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        if name.ends_with(".pyc") || name.ends_with(".pyo") {
            let _ = root.remove_file(&rel_item);
            continue;
        }
        if name.ends_with(".py") && !expected_py.contains(name) {
            let _ = root.remove_file(&rel_item);
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

        let root = RootDir::open(&mnt).expect("root");
        sync_strata_debug_package(&mod_root, &mnt, &root).expect("sync package");

        assert!(dst_dir.join("cli.py").is_file());
        assert!(!dst_dir.join("notes.txt").exists());
        assert!(!dst_dir.join("obsolete.py").exists());
        assert!(!dst_dir.join("stale.pyc").exists());
        assert!(!pycache.exists());
        assert!(mnt.join("usr/local/bin/strata-debug").is_file());

        let _ = fs::remove_dir_all(&base);
    }
}
