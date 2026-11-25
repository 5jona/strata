//! Crash-safe atomic file persistence (`temp -> fsync -> chmod -> rename`) and log rotation.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{RuntimeError, RuntimeResult};

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Creates directory `path` recursively and applies `mode`.
pub fn ensure_dir_mode(path: &Path, mode: u32) -> RuntimeResult<()> {
    fs::create_dir_all(path).map_err(|source| RuntimeError::Io {
        op: "create_dir_all",
        source,
    })?;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    Ok(())
}

/// Persists `bytes` to `dst` atomically via sibling temporary file + `sync_all` + `rename`.
pub fn write_atomic(dst: &Path, bytes: &[u8], mode: u32) -> RuntimeResult<()> {
    let parent = dst.parent().ok_or_else(|| {
        RuntimeError::InvalidArgument(format!("path '{}' has no parent directory", dst.display()))
    })?;
    fs::create_dir_all(parent).map_err(|source| RuntimeError::Io {
        op: "create parent dir",
        source,
    })?;

    let tmp_path = unique_temp_sibling(dst);
    let write_res = (|| -> RuntimeResult<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)
            .map_err(|source| RuntimeError::Io {
                op: "create temp file",
                source,
            })?;
        file.write_all(bytes).map_err(|source| RuntimeError::Io {
            op: "write temp file",
            source,
        })?;
        file.sync_all().map_err(|source| RuntimeError::Io {
            op: "sync temp file",
            source,
        })?;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(mode)).map_err(|source| {
            RuntimeError::Io {
                op: "chmod temp file",
                source,
            }
        })?;
        fs::rename(&tmp_path, dst).map_err(|source| RuntimeError::Io {
            op: "rename temp file",
            source,
        })?;
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();

    if write_res.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_res
}

/// Atomically writes `bytes` to `dst` only when existing content differs, ensuring `mode`.
pub fn sync_bytes_if_changed(bytes: &[u8], dst: &Path, mode: u32) -> RuntimeResult<bool> {
    if let Ok(existing) = fs::read(dst)
        && existing == bytes
    {
        let _ = fs::set_permissions(dst, fs::Permissions::from_mode(mode));
        return Ok(false);
    }
    write_atomic(dst, bytes, mode)?;
    Ok(true)
}

/// Rotates `log_path` to `<log_path>.old` when its file size exceeds `max_bytes`.
pub fn rotate_log_file(log_path: &Path, max_bytes: u64) -> RuntimeResult<bool> {
    if let Some(parent) = log_path.parent() {
        ensure_dir_mode(parent, 0o700)?;
    }
    let Ok(meta) = fs::metadata(log_path) else {
        return Ok(false);
    };
    if meta.len() <= max_bytes {
        return Ok(false);
    }

    let mut old_name = log_path.as_os_str().to_os_string();
    old_name.push(".old");
    let old_path = PathBuf::from(old_name);
    let _ = fs::rename(log_path, &old_path);
    write_atomic(log_path, b"", 0o600)?;
    Ok(true)
}

/// Builds a sibling temporary path used for atomic replacement.
#[must_use]
pub fn unique_temp_sibling(dst: &Path) -> PathBuf {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let name = dst
        .file_name()
        .map_or_else(|| "file".to_owned(), |n| n.to_string_lossy().into_owned());
    dst.with_file_name(format!(".{name}.strata.{pid}.{seq}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_sync_and_rotation_work_cleanly() {
        let base = std::env::temp_dir().join(format!("strata-atomic-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let target = base.join("config.json");

        write_atomic(&target, b"{\"v\":1}\n", 0o600).expect("atomic write");
        assert_eq!(fs::read(&target).expect("read"), b"{\"v\":1}\n");

        let changed = sync_bytes_if_changed(b"{\"v\":1}\n", &target, 0o600).expect("unchanged");
        assert!(!changed);
        let changed = sync_bytes_if_changed(b"{\"v\":2}\n", &target, 0o600).expect("changed");
        assert!(changed);

        let log = base.join("sshd.log");
        write_atomic(&log, &vec![b'x'; 256], 0o600).expect("seed log");
        assert!(rotate_log_file(&log, 128).expect("rotate"));
        assert_eq!(fs::metadata(&log).expect("new log").len(), 0);
        assert!(base.join("sshd.log.old").exists());

        let _ = fs::remove_dir_all(&base);
    }
}
