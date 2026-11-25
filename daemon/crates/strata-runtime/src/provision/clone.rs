//! Sparse-aware image cloning (`SEEK_DATA` / `SEEK_HOLE`) skipping unallocated holes.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use strata_protocol::constants::SELINUX_IMAGE_CONTEXT;
use strata_sys::selinux;

use crate::error::{RuntimeError, RuntimeResult};
use crate::fs_atomic::{ensure_dir_mode, unique_temp_sibling};

/// Sparse-aware clone using `SEEK_DATA`/`SEEK_HOLE` so unallocated holes are skipped cleanly.
/// Returns `(copied_data_bytes, virtual_size_bytes)`.
pub fn sparse_clone_image(
    src_path: &Path,
    dst_path: &Path,
    cancel: Option<&AtomicBool>,
) -> RuntimeResult<(u64, u64)> {
    if !src_path.is_file() {
        return Err(RuntimeError::ImageMissing(src_path.to_path_buf()));
    }
    if let Some(parent) = dst_path.parent() {
        ensure_dir_mode(parent, 0o755)?;
    }

    let mut src = File::open(src_path).map_err(|source| RuntimeError::Io {
        op: "open source image for clone",
        source,
    })?;
    let virtual_size = src
        .metadata()
        .map_err(|source| RuntimeError::Io {
            op: "stat source image",
            source,
        })?
        .len();

    let tmp_dst = unique_temp_sibling(dst_path);
    let clone_res = (|| -> RuntimeResult<u64> {
        let mut dst = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_dst)
            .map_err(|source| RuntimeError::Io {
                op: "create clone temp image",
                source,
            })?;
        dst.set_len(virtual_size)
            .map_err(|source| RuntimeError::Io {
                op: "set sparse clone virtual size",
                source,
            })?;

        let mut offset: u64 = 0;
        let mut copied_bytes: u64 = 0;
        let mut buf = vec![0u8; 256 * 1024];

        while offset < virtual_size {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(RuntimeError::InvalidArgument("clone cancelled".to_owned()));
            }
            let data_start = match seek_next_extent(src.as_raw_fd(), offset, libc::SEEK_DATA) {
                Ok(Some(pos)) => pos.min(virtual_size),
                Ok(None) => break,
                Err(_) => offset,
            };
            let hole_start = match seek_next_extent(src.as_raw_fd(), data_start, libc::SEEK_HOLE) {
                Ok(Some(pos)) => pos.min(virtual_size),
                Ok(None) | Err(_) => virtual_size,
            };
            if hole_start <= data_start {
                break;
            }

            src.seek(SeekFrom::Start(data_start))
                .map_err(|source| RuntimeError::Io {
                    op: "seek clone src",
                    source,
                })?;
            dst.seek(SeekFrom::Start(data_start))
                .map_err(|source| RuntimeError::Io {
                    op: "seek clone dst",
                    source,
                })?;

            let mut remaining = hole_start - data_start;
            while remaining > 0 {
                if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                    return Err(RuntimeError::InvalidArgument("clone cancelled".to_owned()));
                }
                let want = (remaining as usize).min(buf.len());
                let n = src
                    .read(&mut buf[..want])
                    .map_err(|source| RuntimeError::Io {
                        op: "read clone chunk",
                        source,
                    })?;
                if n == 0 {
                    break;
                }
                dst.write_all(&buf[..n])
                    .map_err(|source| RuntimeError::Io {
                        op: "write clone chunk",
                        source,
                    })?;
                copied_bytes += n as u64;
                remaining -= n as u64;
            }
            offset = hole_start;
        }

        dst.sync_all().map_err(|source| RuntimeError::Io {
            op: "sync cloned image",
            source,
        })?;
        let _ = fs::set_permissions(&tmp_dst, fs::Permissions::from_mode(0o600));
        fs::rename(&tmp_dst, dst_path).map_err(|source| RuntimeError::Io {
            op: "rename cloned image",
            source,
        })?;
        let _ = selinux::set_file_context(dst_path, SELINUX_IMAGE_CONTEXT);
        Ok(copied_bytes)
    })();

    if clone_res.is_err() {
        let _ = fs::remove_file(&tmp_dst);
    }
    Ok((clone_res?, virtual_size))
}

fn seek_next_extent(
    fd: std::os::fd::RawFd,
    offset: u64,
    whence: libc::c_int,
) -> Result<Option<u64>, std::io::Error> {
    // SAFETY: `lseek` on a valid open file descriptor with SEEK_DATA / SEEK_HOLE is safe.
    let rc = unsafe { libc::lseek(fd, offset as libc::off_t, whence) };
    if rc < 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENXIO) {
            return Ok(None);
        }
        return Err(err);
    }
    Ok(Some(rc as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provision::resize::validate_ext4_magic;

    #[test]
    fn sparse_clone_skips_holes_and_preserves_bytes_and_ext4_magic() {
        let base = std::env::temp_dir().join(format!("strata-sparse-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create dir");

        let src = base.join("src.img");
        let dst = base.join("dst.img");
        let virtual_len = 16 * 1024 * 1024;
        {
            let mut f = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&src)
                .expect("create src");
            f.set_len(virtual_len).expect("set len");
            f.seek(SeekFrom::Start(1024 + 0x38)).expect("seek magic");
            f.write_all(&0xEF53_u16.to_le_bytes()).expect("write magic");
            f.seek(SeekFrom::Start(12 * 1024 * 1024))
                .expect("seek tail");
            f.write_all(b"strata-sparse-marker").expect("write marker");
        }

        assert!(validate_ext4_magic(&src).expect("magic check"));
        let (copied, virt) = sparse_clone_image(&src, &dst, None).expect("sparse clone");
        assert_eq!(virt, virtual_len);
        assert!(
            copied < 2 * 1024 * 1024,
            "expected sparse clone copied={copied}"
        );
        assert!(validate_ext4_magic(&dst).expect("cloned magic"));

        let mut check = File::open(&dst).expect("open dst");
        check
            .seek(SeekFrom::Start(12 * 1024 * 1024))
            .expect("seek dst");
        let mut marker = [0u8; 20];
        check.read_exact(&mut marker).expect("read marker");
        assert_eq!(&marker, b"strata-sparse-marker");

        let _ = fs::remove_dir_all(&base);
    }
}
