//! Zero-polling `inotify` wrapper for watching atomic configuration updates.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::error::{SysError, SysResult};

/// Non-blocking `inotify` descriptor suitable for wrapping in Tokio `AsyncFd`.
#[derive(Debug)]
pub struct InotifyWatcher {
    fd: OwnedFd,
}

impl InotifyWatcher {
    /// Creates an `inotify` instance (`IN_NONBLOCK | IN_CLOEXEC`) watching `dir` for
    /// atomic writes (`IN_CLOSE_WRITE | IN_MOVED_TO | IN_CREATE | IN_DELETE`).
    pub fn watch_dir(dir: &Path) -> SysResult<Self> {
        // SAFETY: `inotify_init1` takes only flag bits and returns a fresh fd or -1.
        let raw = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if raw < 0 {
            return Err(SysError::last_os("inotify_init1"));
        }
        // SAFETY: `raw` is a newly created valid file descriptor owned exclusively by `fd`.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };

        let c_dir = CString::new(dir.as_os_str().as_bytes())
            .map_err(|_| SysError::InvalidCString(dir.to_string_lossy().into_owned()))?;
        let mask = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE | libc::IN_DELETE;
        // SAFETY: `fd` is an open inotify descriptor and `c_dir` is a valid NUL-terminated path.
        let wd = unsafe { libc::inotify_add_watch(fd.as_raw_fd(), c_dir.as_ptr(), mask) };
        if wd < 0 {
            return Err(SysError::path_os("inotify_add_watch", dir));
        }
        Ok(Self { fd })
    }

    /// Drains all queued inotify events, returning `true` if at least one event was read.
    pub fn drain_events(&self) -> io::Result<bool> {
        let mut buf = [0_u8; 4096];
        let mut any = false;
        loop {
            // SAFETY: `self.fd` is a valid non-blocking inotify fd and `buf` is writable storage.
            let n = unsafe {
                libc::read(
                    self.fd.as_raw_fd(),
                    buf.as_mut_ptr().cast::<libc::c_void>(),
                    buf.len(),
                )
            };
            if n > 0 {
                any = true;
                continue;
            }
            if n == 0 {
                return Ok(any);
            }
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                return Ok(any);
            }
            return Err(err);
        }
    }
}

impl AsRawFd for InotifyWatcher {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl AsFd for InotifyWatcher {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
