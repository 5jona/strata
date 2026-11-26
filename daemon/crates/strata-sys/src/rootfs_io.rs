//! Root-anchored (`openat2` / `*at` + `O_NOFOLLOW`) filesystem operations for container rootfs trees.
//! Prevents malicious symlinks inside an untrusted container rootfs from escaping onto the Android host.

mod raw;
mod tree;

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{SysError, SysResult};
use raw::{
    collect_rel_components, dup_fd, open_child_create_excl, open_child_dir_nofollow,
    open_child_file_ro_nofollow, os_to_cstring,
};

static ROOT_TMP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Anchored handle (`O_DIRECTORY | O_CLOEXEC`) to a container `mount_dir`.
#[derive(Debug)]
pub struct RootDir {
    fd: OwnedFd,
}

impl RootDir {
    /// Opens `root_path` as an anchored directory descriptor.
    pub fn open(root_path: &Path) -> SysResult<Self> {
        let c_path = os_to_cstring(root_path.as_os_str())?;
        // SAFETY: `c_path` is NUL-terminated and flags open a directory safely with `O_CLOEXEC`.
        let raw_fd = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if raw_fd < 0 {
            return Err(SysError::path_os("open root dir", root_path));
        }
        // SAFETY: `raw_fd` is a newly allocated valid file descriptor.
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        Ok(Self { fd })
    }

    /// Returns `true` if `rel_path` exists as a real directory (never traversing symlinks).
    #[must_use]
    pub fn is_dir(&self, rel_path: &Path) -> bool {
        self.open_dir_existing(rel_path).is_ok()
    }

    /// Creates `rel_path` recursively inside the root without traversing any symlink, setting `mode`.
    pub fn ensure_dir(&self, rel_path: &Path, mode: u32) -> SysResult<OwnedFd> {
        let comps = collect_rel_components(rel_path)?;
        let mut cur = dup_fd(self.fd.as_raw_fd())?;
        for seg in comps {
            let c_seg = os_to_cstring(seg)?;
            // SAFETY: `cur` is a valid directory fd and `c_seg` is a NUL-terminated component name.
            let mk =
                unsafe { libc::mkdirat(cur.as_raw_fd(), c_seg.as_ptr(), mode as libc::mode_t) };
            if mk < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::EEXIST) {
                    return Err(SysError::path_io("mkdirat", rel_path, err));
                }
            }
            cur = open_child_dir_nofollow(cur.as_raw_fd(), &c_seg, rel_path)?;
        }
        // SAFETY: `cur` is a real directory fd (opened with `O_DIRECTORY | O_NOFOLLOW`).
        let _ = unsafe { libc::fchmod(cur.as_raw_fd(), (mode & 0o1777) as libc::mode_t) };
        Ok(cur)
    }

    /// Reads a regular file at `rel_path` without traversing any symlink in ancestors or leaf.
    pub fn read_file(&self, rel_path: &Path) -> SysResult<Vec<u8>> {
        let (parent, name) = self.open_existing_parent_and_name(rel_path)?;
        let fd = open_child_file_ro_nofollow(parent.as_raw_fd(), &name, rel_path)?;
        let mut file = File::from(fd);
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| SysError::path_io("read rooted file", rel_path, e))?;
        Ok(buf)
    }

    /// Atomically writes `bytes` to `rel_path` inside the root via `O_CREAT|O_EXCL|O_NOFOLLOW` +
    /// `fchmod` + `fsync` + `renameat` (replacing any leaf symlink without following it).
    pub fn write_atomic(&self, rel_path: &Path, bytes: &[u8], mode: u32) -> SysResult<()> {
        let (parent, name) = self.ensure_parent_and_name(rel_path, 0o755)?;
        let seq = ROOT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp_name = format!(
            ".{}.strata.{}.{seq}.tmp",
            name.to_string_lossy(),
            std::process::id()
        );
        let c_tmp = CString::new(tmp_name.as_bytes())
            .map_err(|_| SysError::InvalidCString(tmp_name.clone()))?;

        let tmp_fd = open_child_create_excl(parent.as_raw_fd(), &c_tmp, mode, rel_path)?;
        let res = (|| -> SysResult<()> {
            let mut file = File::from(tmp_fd);
            file.write_all(bytes)
                .map_err(|e| SysError::path_io("write rooted temp", rel_path, e))?;
            file.sync_all()
                .map_err(|e| SysError::path_io("sync rooted temp", rel_path, e))?;
            // SAFETY: `file` is an open regular file fd we created with `O_CREAT|O_EXCL`.
            let ch = unsafe { libc::fchmod(file.as_raw_fd(), (mode & 0o0777) as libc::mode_t) };
            if ch < 0 {
                return Err(SysError::path_os("fchmod rooted temp", rel_path));
            }
            drop(file);
            // SAFETY: `parent` is a real directory fd; `renameat` atomically replaces leaf symlinks.
            let rn = unsafe {
                libc::renameat(
                    parent.as_raw_fd(),
                    c_tmp.as_ptr(),
                    parent.as_raw_fd(),
                    name.as_ptr(),
                )
            };
            if rn < 0 {
                return Err(SysError::path_os("renameat rooted file", rel_path));
            }
            // SAFETY: `fsync` on parent dir fd persists the atomic directory entry update.
            let _ = unsafe { libc::fsync(parent.as_raw_fd()) };
            Ok(())
        })();

        if res.is_err() {
            // SAFETY: Best-effort cleanup of temp file in `parent`.
            let _ = unsafe { libc::unlinkat(parent.as_raw_fd(), c_tmp.as_ptr(), 0) };
        }
        res
    }

    /// Writes `bytes` atomically only if existing non-symlink file content differs.
    pub fn sync_bytes_if_changed(
        &self,
        bytes: &[u8],
        rel_path: &Path,
        mode: u32,
    ) -> SysResult<bool> {
        if let Ok(existing) = self.read_file(rel_path)
            && existing == bytes
        {
            if let Ok((parent, name)) = self.open_existing_parent_and_name(rel_path)
                && let Ok(fd) = open_child_file_ro_nofollow(parent.as_raw_fd(), &name, rel_path)
            {
                // SAFETY: `fd` is an open non-symlink file descriptor.
                let _ = unsafe { libc::fchmod(fd.as_raw_fd(), (mode & 0o0777) as libc::mode_t) };
            }
            return Ok(false);
        }
        self.write_atomic(rel_path, bytes, mode)?;
        Ok(true)
    }

    /// Unlinks a file or symlink at `rel_path` without traversing symlinks in ancestor directories.
    pub fn remove_file(&self, rel_path: &Path) -> SysResult<()> {
        let (parent, name) = self.open_existing_parent_and_name(rel_path)?;
        // SAFETY: `parent` is a real directory fd and `name` is NUL-terminated.
        let rc = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::ENOENT) {
                return Err(SysError::path_io("unlinkat", rel_path, err));
            }
        }
        Ok(())
    }

    /// Opens an existing non-symlink regular file at `rel_path` with `O_RDONLY | O_NOFOLLOW | O_NONBLOCK`.
    pub fn open_file_ro_nofollow(&self, rel_path: &Path) -> SysResult<OwnedFd> {
        let (parent, name) = self.open_existing_parent_and_name(rel_path)?;
        open_child_file_ro_nofollow(parent.as_raw_fd(), &name, rel_path)
    }

    /// Opens an existing directory at `rel_path` without following any symlink along the path.
    pub fn open_dir_existing(&self, rel_path: &Path) -> SysResult<OwnedFd> {
        let comps = collect_rel_components(rel_path)?;
        let mut cur = dup_fd(self.fd.as_raw_fd())?;
        for seg in comps {
            cur = open_child_dir_nofollow(cur.as_raw_fd(), &os_to_cstring(seg)?, rel_path)?;
        }
        Ok(cur)
    }

    fn ensure_parent_and_name(
        &self,
        rel_path: &Path,
        dir_mode: u32,
    ) -> SysResult<(OwnedFd, CString)> {
        let comps = collect_rel_components(rel_path)?;
        let Some((last, parents)) = comps.split_last() else {
            return Err(SysError::InvalidArgument {
                op: "rooted path",
                detail: "empty relative path".to_owned(),
            });
        };
        let parent_fd = if parents.is_empty() {
            dup_fd(self.fd.as_raw_fd())?
        } else {
            let parent_rel: PathBuf = parents.iter().collect();
            self.ensure_dir(&parent_rel, dir_mode)?
        };
        Ok((parent_fd, os_to_cstring(last)?))
    }

    fn open_existing_parent_and_name(&self, rel_path: &Path) -> SysResult<(OwnedFd, CString)> {
        let comps = collect_rel_components(rel_path)?;
        let Some((last, parents)) = comps.split_last() else {
            return Err(SysError::InvalidArgument {
                op: "rooted path",
                detail: "empty relative path".to_owned(),
            });
        };
        let mut cur = dup_fd(self.fd.as_raw_fd())?;
        for seg in parents {
            cur = open_child_dir_nofollow(cur.as_raw_fd(), &os_to_cstring(seg)?, rel_path)?;
        }
        Ok((cur, os_to_cstring(last)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn rejects_symlink_ancestor_escape_and_replaces_leaf_symlink() {
        let base = std::env::temp_dir().join(format!("strata-rootfs-io-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("mnt");
        let outside = base.join("host_secret");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&outside).expect("outside");
        fs::write(outside.join("target.txt"), b"original").expect("seed outside");

        symlink(&outside, root.join("etc")).expect("symlink etc");
        let rd = RootDir::open(&root).expect("open root");
        assert!(
            rd.write_atomic(Path::new("etc/target.txt"), b"pwned", 0o644)
                .is_err()
        );
        assert_eq!(
            fs::read(outside.join("target.txt")).expect("read"),
            b"original"
        );

        fs::create_dir_all(root.join("run")).expect("run");
        symlink(outside.join("target.txt"), root.join("run/resolv.conf")).expect("leaf link");
        rd.sync_bytes_if_changed(b"nameserver 1.1.1.1\n", Path::new("run/resolv.conf"), 0o644)
            .expect("atomic replace");
        assert_eq!(
            fs::read(outside.join("target.txt")).expect("unchanged"),
            b"original"
        );
        assert_eq!(
            rd.read_file(Path::new("run/resolv.conf"))
                .expect("read new"),
            b"nameserver 1.1.1.1\n"
        );

        let _ = fs::remove_dir_all(&base);
    }
}
