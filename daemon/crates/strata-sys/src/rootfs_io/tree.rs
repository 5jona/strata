//! Symlink-safe rooted directory inspection and recursive cleanup (`openat2` / `fstatat` / `unlinkat`).

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd, RawFd};
use std::path::Path;

use crate::error::{SysError, SysResult};
use crate::rootfs_io::RootDir;
use crate::rootfs_io::raw::{dup_fd, open_child_dir_nofollow};

const MAX_TREE_DEPTH: usize = 64;

struct DirStream(*mut libc::DIR);

impl DirStream {
    fn from_owned_fd(fd: OwnedFd, rel_path: &Path) -> SysResult<Self> {
        let raw = fd.into_raw_fd();
        // SAFETY: `fdopendir` takes ownership of `raw` on success; on failure we close `raw`.
        let ptr = unsafe { libc::fdopendir(raw) };
        if ptr.is_null() {
            let err = io::Error::last_os_error();
            // SAFETY: `raw` is still owned by us because `fdopendir` failed.
            unsafe {
                libc::close(raw);
            }
            return Err(SysError::path_io("fdopendir", rel_path, err));
        }
        Ok(Self(ptr))
    }

    fn next_entry_name(&mut self) -> Option<CString> {
        loop {
            // SAFETY: `self.0` is a valid non-null `DIR*` stream owned exclusively by `self`.
            let ent = unsafe { libc::readdir(self.0) };
            if ent.is_null() {
                return None;
            }
            // SAFETY: `d_name` is a NUL-terminated C string within the dirent returned by `readdir`.
            let c_name = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) };
            let bytes = c_name.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            if let Ok(owned) = CString::new(bytes) {
                return Some(owned);
            }
        }
    }
}

impl Drop for DirStream {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid `DIR*` created by `fdopendir`; `closedir` closes its fd too.
        unsafe {
            libc::closedir(self.0);
        }
    }
}

fn fstatat_nofollow(dirfd: RawFd, name: &CStr) -> io::Result<libc::stat> {
    // SAFETY: `libc::stat` is a plain C struct safe to zero-initialize.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `dirfd` is a valid open directory fd, `name` is NUL-terminated, and `st` is writable.
    let rc = unsafe { libc::fstatat(dirfd, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(st)
}

fn dir_size_fd(dirfd: RawFd, rel_path: &Path, depth: usize) -> u64 {
    if depth >= MAX_TREE_DEPTH {
        return 0;
    }
    let Ok(dup) = dup_fd(dirfd) else {
        return 0;
    };
    let Ok(mut stream) = DirStream::from_owned_fd(dup, rel_path) else {
        return 0;
    };
    let mut total: u64 = 0;
    while let Some(name) = stream.next_entry_name() {
        let Ok(st) = fstatat_nofollow(dirfd, &name) else {
            continue;
        };
        match st.st_mode & libc::S_IFMT {
            libc::S_IFREG => {
                if let Ok(sz) = u64::try_from(st.st_size) {
                    total = total.saturating_add(sz);
                }
            }
            libc::S_IFDIR => {
                if let Ok(child_fd) = open_child_dir_nofollow(dirfd, &name, rel_path) {
                    total = total.saturating_add(dir_size_fd(
                        child_fd.as_raw_fd(),
                        rel_path,
                        depth + 1,
                    ));
                }
            }
            _ => {}
        }
    }
    total
}

fn clear_dir_fd(dirfd: RawFd, rel_path: &Path, depth: usize) -> SysResult<()> {
    if depth >= MAX_TREE_DEPTH {
        return Ok(());
    }
    let dup = dup_fd(dirfd)?;
    let mut stream = DirStream::from_owned_fd(dup, rel_path)?;
    let mut names = Vec::new();
    while let Some(name) = stream.next_entry_name() {
        names.push(name);
    }
    drop(stream);

    for name in names {
        let Ok(st) = fstatat_nofollow(dirfd, &name) else {
            continue;
        };
        if (st.st_mode & libc::S_IFMT) == libc::S_IFDIR {
            if let Ok(child_fd) = open_child_dir_nofollow(dirfd, &name, rel_path) {
                let _ = clear_dir_fd(child_fd.as_raw_fd(), rel_path, depth + 1);
            }
            // SAFETY: `dirfd` is a verified directory fd and `name` is a single component.
            let _ = unsafe { libc::unlinkat(dirfd, name.as_ptr(), libc::AT_REMOVEDIR) };
        } else {
            // SAFETY: `dirfd` is a verified directory fd and `name` is a non-directory entry (file/symlink).
            let _ = unsafe { libc::unlinkat(dirfd, name.as_ptr(), 0) };
        }
    }
    Ok(())
}

impl RootDir {
    /// Returns `true` if `rel_path` is a real directory inside the root with at least one entry.
    #[must_use]
    pub fn dir_has_entries(&self, rel_path: &Path) -> bool {
        let Ok(dir_fd) = self.open_dir_existing(rel_path) else {
            return false;
        };
        let Ok(mut stream) = DirStream::from_owned_fd(dir_fd, rel_path) else {
            return false;
        };
        stream.next_entry_name().is_some()
    }

    /// Recursively computes total regular-file bytes under `rel_path` without following symlinks.
    #[must_use]
    pub fn dir_size_bytes(&self, rel_path: &Path) -> u64 {
        let Ok(dir_fd) = self.open_dir_existing(rel_path) else {
            return 0;
        };
        dir_size_fd(dir_fd.as_raw_fd(), rel_path, 0)
    }

    /// Recursively removes all entries inside `rel_path` (keeping `rel_path` itself) without
    /// following any symlinks in ancestor directories or child entries.
    pub fn clear_dir_contents(&self, rel_path: &Path) -> SysResult<()> {
        let dir_fd = match self.open_dir_existing(rel_path) {
            Ok(fd) => fd,
            Err(SysError::PathIo { source, .. })
                if matches!(
                    source.raw_os_error(),
                    Some(libc::ENOENT | libc::ENOTDIR | libc::ELOOP | libc::EXDEV)
                ) =>
            {
                return Ok(());
            }
            Err(err) => return Err(err),
        };
        clear_dir_fd(dir_fd.as_raw_fd(), rel_path, 0)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use crate::rootfs_io::RootDir;

    #[test]
    fn tree_ops_never_follow_ancestor_or_child_symlinks_outside_root() {
        let base = std::env::temp_dir().join(format!("strata-tree-io-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("mnt");
        let outside = base.join("host_victim");
        fs::create_dir_all(root.join("root/.cache/mise/sub")).expect("create mise cache");
        fs::create_dir_all(&outside).expect("create outside");
        fs::write(outside.join("keep.txt"), b"must-not-be-deleted").expect("seed outside");
        fs::write(root.join("root/.cache/mise/sub/a.bin"), vec![0u8; 1024]).expect("seed cache");
        symlink(&outside, root.join("root/.cache/mise/escape")).expect("child symlink");
        symlink(&outside, root.join("root/.cache/uv")).expect("ancestor symlink");

        let rd = RootDir::open(&root).expect("open root");
        assert!(rd.dir_has_entries(Path::new("root/.cache/mise")));
        assert!(!rd.dir_has_entries(Path::new("root/.cache/uv")));
        assert_eq!(rd.dir_size_bytes(Path::new("root/.cache/mise")), 1024);
        assert_eq!(rd.dir_size_bytes(Path::new("root/.cache/uv")), 0);

        rd.clear_dir_contents(Path::new("root/.cache/uv"))
            .expect("uv symlink skip");
        rd.clear_dir_contents(Path::new("root/.cache/mise"))
            .expect("mise clear");

        assert!(!rd.dir_has_entries(Path::new("root/.cache/mise")));
        assert_eq!(rd.dir_size_bytes(Path::new("root/.cache/mise")), 0);
        assert_eq!(
            fs::read(outside.join("keep.txt")).expect("outside intact"),
            b"must-not-be-deleted"
        );

        let _ = fs::remove_dir_all(&base);
    }
}
