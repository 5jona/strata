//! Low-level `openat2` (`RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS`) and `*at` helpers for `RootDir`.

use std::ffi::{CString, OsStr};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

use crate::error::{SysError, SysResult};

/// Linux universal `SYS_openat2` syscall number on both `x86_64` and `aarch64`.
const SYS_OPENAT2: libc::c_long = 437;
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

pub(super) fn collect_rel_components(rel_path: &Path) -> SysResult<Vec<&OsStr>> {
    let mut comps = Vec::new();
    for comp in rel_path.components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(seg) => comps.push(seg),
            Component::RootDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(SysError::InvalidArgument {
                    op: "rooted path",
                    detail: format!("disallowed component in '{}'", rel_path.display()),
                });
            }
        }
    }
    Ok(comps)
}

fn openat2_or_fallback(
    dirfd: RawFd,
    name: &CString,
    flags: libc::c_int,
    mode: u32,
) -> io::Result<OwnedFd> {
    let how = OpenHow {
        flags: (flags | libc::O_CLOEXEC) as u64,
        mode: u64::from(mode),
        resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
    };
    // SAFETY: `how` matches Linux `struct open_how` ABI and `name` is NUL-terminated.
    let rc = unsafe {
        libc::syscall(
            SYS_OPENAT2,
            dirfd,
            name.as_ptr(),
            &how as *const OpenHow,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if rc >= 0 {
        // SAFETY: `rc` is a newly opened valid file descriptor.
        return Ok(unsafe { OwnedFd::from_raw_fd(rc as RawFd) });
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() != Some(libc::ENOSYS) {
        return Err(err);
    }
    // SAFETY: Fallback single-component `openat` + `O_NOFOLLOW` when kernel lacks `openat2`.
    let fb = unsafe {
        libc::openat(
            dirfd,
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode as libc::mode_t,
        )
    };
    if fb < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fb` is a newly opened valid file descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(fb) })
}

pub(super) fn open_child_dir_nofollow(
    dirfd: RawFd,
    name: &CString,
    rel_path: &Path,
) -> SysResult<OwnedFd> {
    openat2_or_fallback(
        dirfd,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        0,
    )
    .map_err(|e| SysError::path_io("open rooted dir", rel_path, e))
}

pub(super) fn open_child_file_ro_nofollow(
    dirfd: RawFd,
    name: &CString,
    rel_path: &Path,
) -> SysResult<OwnedFd> {
    let fd = openat2_or_fallback(
        dirfd,
        name,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        0,
    )
    .map_err(|e| SysError::path_io("open rooted file", rel_path, e))?;
    // SAFETY: `libc::stat` is a plain C struct safe to zero-initialize before `fstat`.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `fd` is valid and `stat` is writable `libc::stat`.
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } < 0 {
        return Err(SysError::path_os("fstat rooted file", rel_path));
    }
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return Err(SysError::InvalidArgument {
            op: "rooted file",
            detail: format!("'{}' is not a regular file", rel_path.display()),
        });
    }
    Ok(fd)
}

pub(super) fn open_child_create_excl(
    dirfd: RawFd,
    name: &CString,
    mode: u32,
    rel_path: &Path,
) -> SysResult<OwnedFd> {
    openat2_or_fallback(
        dirfd,
        name,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW,
        mode & 0o0777,
    )
    .map_err(|e| SysError::path_io("create rooted temp", rel_path, e))
}

pub(super) fn dup_fd(fd: RawFd) -> SysResult<OwnedFd> {
    // SAFETY: `F_DUPFD_CLOEXEC` duplicates an open fd with `FD_CLOEXEC` set atomically.
    let dup = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if dup < 0 {
        return Err(SysError::last_os("fcntl(F_DUPFD_CLOEXEC)"));
    }
    // SAFETY: `dup` is a newly allocated valid file descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(dup) })
}

pub(super) fn os_to_cstring(s: &OsStr) -> SysResult<CString> {
    CString::new(s.as_bytes())
        .map_err(|_| SysError::InvalidCString(s.to_string_lossy().into_owned()))
}
