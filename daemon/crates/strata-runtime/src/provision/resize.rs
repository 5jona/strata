//! Sparse ext4 image creation, ext4 superblock verification, and offline resize.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use strata_protocol::constants::SELINUX_IMAGE_CONTEXT;
use strata_sys::selinux;

use crate::error::{RuntimeError, RuntimeResult};
use crate::fs_atomic::ensure_dir_mode;
use crate::storage::E2FSCK_BIN;

const MIN_EXT4_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const EXT4_SUPERBLOCK_MAGIC_OFFSET: u64 = 1024 + 0x38;
const EXT4_SUPER_MAGIC: u16 = 0xEF53;
const MKE2FS_ANDROID_BIN: &str = "/system/bin/mke2fs";
const RESIZE2FS_ANDROID_BIN: &str = "/system/bin/resize2fs";

/// Creates a sparse ext4 image of `size_bytes` with `-m 0` and lazy inode/journal initialization.
pub fn create_sparse_ext4(image_path: &Path, size_bytes: u64) -> RuntimeResult<()> {
    if size_bytes < MIN_EXT4_IMAGE_BYTES {
        return Err(RuntimeError::InvalidArgument(format!(
            "ext4 image size {size_bytes} is below 64 MiB minimum"
        )));
    }
    if let Some(parent) = image_path.parent() {
        ensure_dir_mode(parent, 0o755)?;
    }

    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(image_path)
        .map_err(|source| RuntimeError::Io {
            op: "create sparse image",
            source,
        })?;
    file.set_len(size_bytes)
        .map_err(|source| RuntimeError::Io {
            op: "ftruncate sparse image",
            source,
        })?;
    drop(file);

    let mke2fs = resolve_tool(MKE2FS_ANDROID_BIN, "mke2fs");
    let out = Command::new(&mke2fs)
        .args([
            "-t",
            "ext4",
            "-F",
            "-q",
            "-m",
            "0",
            "-E",
            "lazy_itable_init=1,lazy_journal_init=1",
        ])
        .arg(image_path)
        .output()
        .map_err(|source| RuntimeError::Io {
            op: "run mke2fs",
            source,
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        let _ = fs::remove_file(image_path);
        return Err(RuntimeError::InvalidArgument(format!(
            "mke2fs failed (code={}): {stderr}",
            out.status.code().unwrap_or(-1)
        )));
    }

    let _ = fs::set_permissions(image_path, fs::Permissions::from_mode(0o600));
    let _ = selinux::set_file_context(image_path, SELINUX_IMAGE_CONTEXT);
    Ok(())
}

/// Checks whether `image_path` starts with a valid ext4 superblock magic (`0xEF53` at offset 1080).
pub fn validate_ext4_magic(image_path: &Path) -> RuntimeResult<bool> {
    let mut file = File::open(image_path).map_err(|source| RuntimeError::Io {
        op: "open image for magic check",
        source,
    })?;
    if file
        .seek(SeekFrom::Start(EXT4_SUPERBLOCK_MAGIC_OFFSET))
        .is_err()
    {
        return Ok(false);
    }
    let mut magic = [0u8; 2];
    if file.read_exact(&mut magic).is_err() {
        return Ok(false);
    }
    Ok(u16::from_le_bytes(magic) == EXT4_SUPER_MAGIC)
}

/// Safely resizes an unmounted ext4 image to `new_size_bytes` (`e2fsck` + `resize2fs` + `ftruncate`).
/// Returns `(previous_bytes, new_bytes)`.
pub fn resize_ext4_image(image_path: &Path, new_size_bytes: u64) -> RuntimeResult<(u64, u64)> {
    if !image_path.is_file() {
        return Err(RuntimeError::ImageMissing(image_path.to_path_buf()));
    }
    if new_size_bytes < MIN_EXT4_IMAGE_BYTES {
        return Err(RuntimeError::InvalidArgument(format!(
            "target size {new_size_bytes} is below 64 MiB minimum"
        )));
    }
    let prev_bytes = fs::metadata(image_path)
        .map_err(|source| RuntimeError::Io {
            op: "stat image for resize",
            source,
        })?
        .len();
    if prev_bytes == new_size_bytes {
        return Ok((prev_bytes, new_size_bytes));
    }

    let e2fsck = resolve_tool(E2FSCK_BIN, "e2fsck");
    let resize2fs = resolve_tool(RESIZE2FS_ANDROID_BIN, "resize2fs");

    if new_size_bytes > prev_bytes {
        let f = OpenOptions::new()
            .write(true)
            .open(image_path)
            .map_err(|source| RuntimeError::Io {
                op: "open image to grow",
                source,
            })?;
        f.set_len(new_size_bytes)
            .map_err(|source| RuntimeError::Io {
                op: "ftruncate grow",
                source,
            })?;
        drop(f);
        run_e2fsck(&e2fsck, image_path, "-p")?;
        run_resize2fs(&resize2fs, image_path, None)?;
    } else {
        run_e2fsck(&e2fsck, image_path, "-y")?;
        let target_kib = format!("{}K", new_size_bytes / 1024);
        run_resize2fs(&resize2fs, image_path, Some(&target_kib))?;
        let f = OpenOptions::new()
            .write(true)
            .open(image_path)
            .map_err(|source| RuntimeError::Io {
                op: "open image to shrink",
                source,
            })?;
        f.set_len(new_size_bytes)
            .map_err(|source| RuntimeError::Io {
                op: "ftruncate shrink",
                source,
            })?;
    }

    let _ = selinux::set_file_context(image_path, SELINUX_IMAGE_CONTEXT);
    Ok((prev_bytes, new_size_bytes))
}

fn run_e2fsck(bin: &str, image_path: &Path, flag: &str) -> RuntimeResult<()> {
    let out = Command::new(bin)
        .args(["-f", flag])
        .arg(image_path)
        .output()
        .map_err(|source| RuntimeError::Io {
            op: "run e2fsck before resize",
            source,
        })?;
    let code = out.status.code().unwrap_or(-1);
    if code != 0 && code != 1 {
        return Err(RuntimeError::FsckFailed {
            path: image_path.to_path_buf(),
            code,
            detail: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

fn run_resize2fs(bin: &str, image_path: &Path, size_arg: Option<&str>) -> RuntimeResult<()> {
    let mut cmd = Command::new(bin);
    cmd.arg(image_path);
    if let Some(arg) = size_arg {
        cmd.arg(arg);
    }
    let out = cmd.output().map_err(|source| RuntimeError::Io {
        op: "run resize2fs",
        source,
    })?;
    if !out.status.success() {
        return Err(RuntimeError::InvalidArgument(format!(
            "resize2fs failed (code={}): {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

fn resolve_tool(android_path: &str, fallback: &str) -> String {
    if Path::new(android_path).is_file() {
        android_path.to_owned()
    } else {
        fallback.to_owned()
    }
}
