//! Rootfs image maintenance operations: native `FITRIM` compaction, `e2fsck`, and gzip backup.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use flate2::Compression;
use flate2::write::GzEncoder;
use strata_protocol::ContainerConfig;
use strata_protocol::rpc::{CompactResult, FsckResult};
use strata_sys::{mount, procfs};

use crate::error::{RuntimeError, RuntimeResult};
use crate::fs_atomic::{ensure_dir_mode, unique_temp_sibling};
use crate::storage::{E2FSCK_BIN, mount_rootfs, teardown_container};

/// Reclaims unused blocks inside the sparse ext4 image using Linux `FITRIM` ioctl.
pub fn compact_image(
    container_id: &str,
    cfg: &ContainerConfig,
    module_dir: Option<&Path>,
) -> RuntimeResult<CompactResult> {
    let image_path = Path::new(&cfg.image_path);
    let mount_dir = Path::new(&cfg.mount_dir);
    if !image_path.is_file() {
        return Err(RuntimeError::ImageMissing(image_path.to_path_buf()));
    }

    let (before_bytes, _) = procfs::image_disk_usage(image_path).map_err(RuntimeError::from_sys)?;
    let was_mounted = mount::is_ext4_mounted(mount_dir).map_err(RuntimeError::from_sys)?;
    if !was_mounted {
        mount_rootfs(cfg, module_dir)?;
    }

    let trim_res = mount::fstrim_path(mount_dir).map_err(RuntimeError::from_sys);
    // SAFETY: `sync(2)` flushes dirty filesystem metadata before sampling sparse allocation.
    unsafe {
        libc::sync();
    }

    if !was_mounted {
        let _ = teardown_container(container_id, cfg, None, None);
    }

    let trimmed_bytes = trim_res?;
    let (after_bytes, _) = procfs::image_disk_usage(image_path).map_err(RuntimeError::from_sys)?;

    Ok(CompactResult {
        container: container_id.to_owned(),
        before_bytes,
        after_bytes,
        trimmed_bytes,
    })
}

/// Runs forced `/system/bin/e2fsck -f -y` on an unmounted container image.
pub fn fsck_image(container_id: &str, cfg: &ContainerConfig) -> RuntimeResult<FsckResult> {
    let image_path = Path::new(&cfg.image_path);
    if !image_path.is_file() {
        return Err(RuntimeError::ImageMissing(image_path.to_path_buf()));
    }
    let out = Command::new(E2FSCK_BIN)
        .arg("-f")
        .arg("-y")
        .arg(image_path)
        .output()
        .map_err(|source| RuntimeError::Io {
            op: "e2fsck -f -y",
            source,
        })?;
    let code = out.status.code().unwrap_or(-1);
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }
    let detail = combined.trim().to_owned();
    if code != 0 && code != 1 {
        return Err(RuntimeError::FsckFailed {
            path: image_path.to_path_buf(),
            code,
            detail,
        });
    }
    Ok(FsckResult {
        container: container_id.to_owned(),
        exit_code: code,
        output: detail,
    })
}

/// Creates a streaming gzip backup of `image_path` written atomically to `output_path`.
pub fn backup_image_gzip(image_path: &Path, output_path: &Path) -> RuntimeResult<u64> {
    if !image_path.is_file() {
        return Err(RuntimeError::ImageMissing(image_path.to_path_buf()));
    }
    if let Some(parent) = output_path.parent() {
        ensure_dir_mode(parent, 0o755)?;
    }

    let tmp_path = unique_temp_sibling(output_path);
    let result = (|| -> RuntimeResult<u64> {
        let src = File::open(image_path).map_err(|source| RuntimeError::Io {
            op: "open image for backup",
            source,
        })?;
        let dst = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)
            .map_err(|source| RuntimeError::Io {
                op: "create backup temp file",
                source,
            })?;
        let mut reader = BufReader::with_capacity(256 * 1024, src);
        let writer = BufWriter::with_capacity(256 * 1024, dst);
        let mut encoder = GzEncoder::new(writer, Compression::fast());
        io::copy(&mut reader, &mut encoder).map_err(|source| RuntimeError::Io {
            op: "gzip stream copy",
            source,
        })?;
        let mut inner = encoder.finish().map_err(|source| RuntimeError::Io {
            op: "finish gzip stream",
            source,
        })?;
        io::Write::flush(&mut inner).map_err(|source| RuntimeError::Io {
            op: "flush backup buffer",
            source,
        })?;
        let file = inner.into_inner().map_err(|err| RuntimeError::Io {
            op: "unwrap backup writer",
            source: err.into_error(),
        })?;
        file.sync_all().map_err(|source| RuntimeError::Io {
            op: "sync backup file",
            source,
        })?;
        let bytes = file
            .metadata()
            .map_err(|source| RuntimeError::Io {
                op: "stat backup file",
                source,
            })?
            .len();
        let _ = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o644));
        fs::rename(&tmp_path, output_path).map_err(|source| RuntimeError::Io {
            op: "rename backup file",
            source,
        })?;
        Ok(bytes)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}
