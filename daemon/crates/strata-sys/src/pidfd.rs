//! RAII `pidfd_open` and `pidfd_send_signal` wrappers for zero-polling process supervision.

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};

use crate::error::{SysError, SysResult};

const PIDFD_NONBLOCK: libc::c_uint = libc::O_NONBLOCK as libc::c_uint;

/// Owned Linux `pidfd` descriptor compatible with epoll / Tokio `AsyncFd`.
#[derive(Debug)]
pub struct PidFd {
    fd: OwnedFd,
    pid: u32,
}

impl PidFd {
    /// Opens a non-blocking `pidfd` referencing `pid` via `SYS_pidfd_open`.
    pub fn open(pid: u32) -> SysResult<Self> {
        if pid == 0 {
            return Err(SysError::InvalidArgument {
                op: "pidfd_open",
                detail: "pid must be non-zero".to_owned(),
            });
        }
        // SAFETY: `SYS_pidfd_open` creates a new non-blocking fd for `pid` or returns -1.
        let raw = unsafe {
            libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, PIDFD_NONBLOCK) as libc::c_int
        };
        if raw < 0 {
            return Err(SysError::last_os("pidfd_open"));
        }
        // SAFETY: `raw` is a newly created valid file descriptor owned exclusively by `Self`.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(Self { fd, pid })
    }

    /// Returns the PID associated with this descriptor.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Sends a signal via `SYS_pidfd_send_signal` without PID reuse races.
    /// Returns `Ok(false)` if the target process has already exited (`ESRCH`).
    pub fn send_signal(&self, sig: libc::c_int) -> SysResult<bool> {
        // SAFETY: `self.fd` is a valid pidfd; passing null `siginfo` and 0 flags is kernel-defined.
        let rc = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                sig,
                std::ptr::null::<libc::siginfo_t>(),
                0_u32,
            )
        };
        if rc == 0 {
            return Ok(true);
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        Err(SysError::Io {
            op: "pidfd_send_signal",
            source: err,
        })
    }
}

impl AsRawFd for PidFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl AsFd for PidFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
