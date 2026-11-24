//! Streaming `.tar.gz` export of a mounted container rootfs, skipping virtual filesystems.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::error::{RuntimeError, RuntimeResult};

/// Streams a mounted container rootfs directory as a `.tar.gz` archive into `writer`,
/// skipping virtual filesystems (`/proc`, `/sys`, `/dev`, `/run`).
pub fn write_tar_gz_directory<W: Write>(
    root_dir: &Path,
    writer: W,
    cancel: &AtomicBool,
) -> RuntimeResult<u64> {
    let encoder = GzEncoder::new(writer, Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    builder.follow_symlinks(false);
    let mut count = 0u64;
    append_dir_filtered(&mut builder, root_dir, Path::new(""), cancel, &mut count)?;
    let encoder = builder.into_inner().map_err(|source| RuntimeError::Io {
        op: "finish tar export",
        source,
    })?;
    let mut inner = encoder.finish().map_err(|source| RuntimeError::Io {
        op: "finish gzip export",
        source,
    })?;
    inner.flush().map_err(|source| RuntimeError::Io {
        op: "flush export stream",
        source,
    })?;
    Ok(count)
}

fn append_dir_filtered<W: Write>(
    builder: &mut tar::Builder<W>,
    base: &Path,
    rel: &Path,
    cancel: &AtomicBool,
    count: &mut u64,
) -> RuntimeResult<()> {
    let cur = if rel.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(rel)
    };
    let entries = fs::read_dir(&cur).map_err(|source| RuntimeError::Io {
        op: "read dir for export",
        source,
    })?;
    for entry_res in entries {
        if cancel.load(Ordering::Relaxed) {
            return Err(RuntimeError::InvalidArgument("export cancelled".to_owned()));
        }
        let entry = entry_res.map_err(|source| RuntimeError::Io {
            op: "read dir entry for export",
            source,
        })?;
        let name = entry.file_name();
        let entry_rel = rel.join(&name);
        if rel.as_os_str().is_empty()
            && matches!(name.to_str(), Some("proc" | "sys" | "dev" | "run"))
        {
            continue;
        }
        let full = base.join(&entry_rel);
        let meta = fs::symlink_metadata(&full).map_err(|source| RuntimeError::Io {
            op: "lstat export entry",
            source,
        })?;
        if meta.file_type().is_dir() {
            builder
                .append_dir(&entry_rel, &full)
                .map_err(|source| RuntimeError::Io {
                    op: "append tar dir",
                    source,
                })?;
            *count += 1;
            append_dir_filtered(builder, base, &entry_rel, cancel, count)?;
        } else if meta.file_type().is_file() || meta.file_type().is_symlink() {
            builder
                .append_path_with_name(&full, &entry_rel)
                .map_err(|source| RuntimeError::Io {
                    op: "append tar entry",
                    source,
                })?;
            *count += 1;
        }
    }
    Ok(())
}
