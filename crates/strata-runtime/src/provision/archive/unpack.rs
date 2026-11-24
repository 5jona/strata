//! Safe tar entry unpacking with Zip-Slip, symlink-ancestor, hardlink, and setuid hardening.

use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{RuntimeError, RuntimeResult};

/// Unpacks tar entries into `dest_dir`, rejecting traversal/symlink escapes, stripping setuid/setgid,
/// and skipping `/dev/*` and device nodes.
pub(super) fn unpack_tar_safe<R: Read>(
    archive: &mut tar::Archive<R>,
    dest_dir: &Path,
    cancel: &AtomicBool,
) -> RuntimeResult<u64> {
    let entries = archive.entries().map_err(|source| RuntimeError::Io {
        op: "read tar entries",
        source,
    })?;
    let mut extracted = 0u64;

    for entry_res in entries {
        if cancel.load(Ordering::Relaxed) {
            return Err(RuntimeError::InvalidArgument(
                "archive extraction cancelled".to_owned(),
            ));
        }
        let mut entry = entry_res.map_err(|source| RuntimeError::Io {
            op: "read tar entry",
            source,
        })?;
        let raw_path = entry
            .path()
            .map_err(|source| RuntimeError::Io {
                op: "read tar entry path",
                source,
            })?
            .into_owned();
        let Some(rel_path) = sanitize_tar_rel_path(&raw_path)? else {
            continue;
        };
        if rel_path.starts_with("dev") {
            continue;
        }
        let entry_type = entry.header().entry_type();
        if matches!(
            entry_type,
            tar::EntryType::Char | tar::EntryType::Block | tar::EntryType::Fifo
        ) {
            continue;
        }

        let target = dest_dir.join(&rel_path);
        ensure_no_symlink_in_ancestors(dest_dir, &rel_path)?;

        let mode = entry.header().mode().ok();
        match entry_type {
            tar::EntryType::Directory => {
                if let Ok(meta) = fs::symlink_metadata(&target)
                    && meta.file_type().is_symlink()
                {
                    fs::remove_file(&target).map_err(|source| RuntimeError::Io {
                        op: "remove symlink before tar dir",
                        source,
                    })?;
                }
                fs::create_dir_all(&target).map_err(|source| RuntimeError::Io {
                    op: "create tar dir",
                    source,
                })?;
                if let Some(m) = mode {
                    set_dir_mode_nofollow(&target, m & 0o1777);
                }
            }
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|source| RuntimeError::Io {
                        op: "create tar file parent",
                        source,
                    })?;
                }
                if fs::symlink_metadata(&target).is_ok() {
                    let _ = fs::remove_file(&target);
                }
                let mut out = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&target)
                    .map_err(|source| RuntimeError::Io {
                        op: "create tar file",
                        source,
                    })?;
                io::copy(&mut entry, &mut out).map_err(|source| RuntimeError::Io {
                    op: "write tar file",
                    source,
                })?;
                if let Some(m) = mode {
                    // Strip setuid/setgid/sticky bits on extracted files to prevent root-setuid promotion.
                    let _ = out.set_permissions(fs::Permissions::from_mode(m & 0o0777));
                }
            }
            tar::EntryType::Symlink => {
                if let Some(link_target) = entry.link_name().ok().flatten() {
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(|source| RuntimeError::Io {
                            op: "create symlink parent",
                            source,
                        })?;
                    }
                    let _ = fs::remove_file(&target);
                    symlink(&*link_target, &target).map_err(|source| RuntimeError::Io {
                        op: "create tar symlink",
                        source,
                    })?;
                }
            }
            tar::EntryType::Link => {
                if let Some(link_target) = entry.link_name().ok().flatten() {
                    let Some(safe_link) = sanitize_tar_rel_path(&link_target)? else {
                        continue;
                    };
                    ensure_no_symlink_in_ancestors(dest_dir, &safe_link)?;
                    let src = dest_dir.join(&safe_link);
                    let src_meta =
                        fs::symlink_metadata(&src).map_err(|source| RuntimeError::Io {
                            op: "stat tar hardlink source",
                            source,
                        })?;
                    if src_meta.file_type().is_symlink() || !src_meta.is_file() {
                        return Err(RuntimeError::InvalidArgument(format!(
                            "rejected tar hardlink to non-regular or symlink target '{}'",
                            safe_link.display()
                        )));
                    }
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(|source| RuntimeError::Io {
                            op: "create hardlink parent",
                            source,
                        })?;
                    }
                    let _ = fs::remove_file(&target);
                    fs::hard_link(&src, &target).map_err(|source| RuntimeError::Io {
                        op: "create tar hardlink",
                        source,
                    })?;
                }
            }
            _ => continue,
        }
        extracted += 1;
    }

    Ok(extracted)
}

fn set_dir_mode_nofollow(dir_path: &Path, mode: u32) {
    if let Ok(dir) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(dir_path)
    {
        // SAFETY: `dir` is an open directory fd guaranteed not to be a symlink.
        let _ = unsafe { libc::fchmod(dir.as_raw_fd(), mode as libc::mode_t) };
    }
}

fn sanitize_tar_rel_path(path: &Path) -> RuntimeResult<Option<PathBuf>> {
    let mut clean = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(seg) => clean.push(seg),
            Component::ParentDir | Component::Prefix(_) => {
                return Err(RuntimeError::InvalidArgument(format!(
                    "rejected unsafe archive path '{}'",
                    path.display()
                )));
            }
        }
    }
    if clean.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(clean))
    }
}

fn ensure_no_symlink_in_ancestors(dest_dir: &Path, rel_path: &Path) -> RuntimeResult<()> {
    let mut cur = dest_dir.to_path_buf();
    let Some(parent) = rel_path.parent() else {
        return Ok(());
    };
    for comp in parent.components() {
        if let Component::Normal(seg) = comp {
            cur.push(seg);
            if let Ok(meta) = fs::symlink_metadata(&cur)
                && meta.file_type().is_symlink()
            {
                return Err(RuntimeError::InvalidArgument(format!(
                    "rejected archive entry traversing symlink ancestor '{}'",
                    rel_path.display()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn strips_setuid_replaces_dir_symlink_and_blocks_symlink_hardlink() {
        let base = std::env::temp_dir().join(format!("strata-unpack-sec-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dest = base.join("dest");
        let outside = base.join("outside");
        fs::create_dir_all(&dest).expect("dest");
        fs::create_dir_all(&outside).expect("outside");

        // 1. Archive with setuid file + symlink replaced by directory entry.
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            let mut sym = tar::Header::new_gnu();
            sym.set_entry_type(tar::EntryType::Symlink);
            sym.set_size(0);
            sym.set_cksum();
            b.append_link(&mut sym, "sub", &outside)
                .expect("append sym");

            let mut dir_hdr = tar::Header::new_gnu();
            dir_hdr.set_entry_type(tar::EntryType::Directory);
            dir_hdr.set_mode(0o4755);
            dir_hdr.set_size(0);
            dir_hdr.set_cksum();
            b.append_data(&mut dir_hdr, "sub", io::empty())
                .expect("dir");

            let mut file_hdr = tar::Header::new_gnu();
            file_hdr.set_mode(0o6755);
            file_hdr.set_size(4);
            file_hdr.set_cksum();
            b.append_data(&mut file_hdr, "sub/bin", &b"test"[..])
                .expect("file");
            b.finish().expect("finish");
        }

        let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
        let cancel = AtomicBool::new(false);
        unpack_tar_safe(&mut archive, &dest, &cancel).expect("unpack");

        // `sub` must be a real directory (not symlink), outside dir untouched, setuid stripped.
        assert!(
            fs::symlink_metadata(dest.join("sub"))
                .expect("sub")
                .is_dir()
        );
        assert!(!outside.join("bin").exists());
        let file_mode = fs::symlink_metadata(dest.join("sub/bin"))
            .expect("bin")
            .permissions()
            .mode();
        assert_eq!(file_mode & 0o7000, 0, "setuid/setgid must be stripped");

        // 2. Archive attempting to hardlink to a symlink must be rejected.
        let mut bad_tar = Vec::new();
        {
            let mut b = tar::Builder::new(&mut bad_tar);
            let mut sym = tar::Header::new_gnu();
            sym.set_entry_type(tar::EntryType::Symlink);
            sym.set_size(0);
            sym.set_cksum();
            b.append_link(&mut sym, "escape_link", outside.join("x"))
                .expect("append sym");

            let mut hl = tar::Header::new_gnu();
            hl.set_entry_type(tar::EntryType::Link);
            hl.set_size(0);
            hl.set_cksum();
            b.append_link(&mut hl, "hard", "escape_link").expect("hl");
            b.finish().expect("finish");
        }
        let mut bad_archive = tar::Archive::new(Cursor::new(bad_tar));
        assert!(unpack_tar_safe(&mut bad_archive, &dest, &cancel).is_err());

        let _ = fs::remove_dir_all(&base);
    }
}
