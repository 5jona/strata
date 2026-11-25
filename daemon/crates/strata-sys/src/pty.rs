//! Minimal audited Linux/Android PTY, session/process-group, and CSPRNG wrappers.

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use crate::error::{SysError, SysResult};

/// Linux universal `_IOR('T', 0x30, unsigned int)` ioctl for reading PTY slave index.
/// Defined explicitly because `libc` omits `TIOCGPTN` on `aarch64-linux-android`.
const TIOCGPTN_IOCTL: libc::c_ulong = 0x8004_5430;

/// Owned master/slave PTY pair allocated from `/dev/ptmx`.
#[derive(Debug)]
pub struct PtyPair {
    pub master: OwnedFd,
    pub slave: OwnedFd,
    pub slave_path: PathBuf,
}

/// Allocates a new Unix98 PTY pair with non-blocking master and initial terminal window size.
pub fn open_pty(cols: u16, rows: u16) -> SysResult<PtyPair> {
    // SAFETY: `posix_openpt` returns a new master fd or -1 on error.
    let master_raw = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if master_raw < 0 {
        return Err(SysError::last_os("posix_openpt"));
    }
    // SAFETY: `master_raw` is a newly opened valid file descriptor.
    let master = unsafe { OwnedFd::from_raw_fd(master_raw) };

    // SAFETY: `grantpt` and `unlockpt` operate on a valid master PTY descriptor.
    if unsafe { libc::grantpt(master.as_raw_fd()) } < 0 {
        return Err(SysError::last_os("grantpt"));
    }
    // SAFETY: `unlockpt` unlocks the slave PTY associated with `master`.
    if unsafe { libc::unlockpt(master.as_raw_fd()) } < 0 {
        return Err(SysError::last_os("unlockpt"));
    }

    let mut pty_num: libc::c_uint = 0;
    // SAFETY: `TIOCGPTN` writes the unsigned int slave index into `pty_num`.
    let rc = unsafe {
        libc::ioctl(
            master.as_raw_fd(),
            TIOCGPTN_IOCTL as _,
            &mut pty_num as *mut libc::c_uint,
        )
    };
    if rc < 0 {
        return Err(SysError::last_os("ioctl(TIOCGPTN)"));
    }

    let slave_path = PathBuf::from(format!("/dev/pts/{pty_num}"));
    let c_slave = CString::new(slave_path.as_os_str().as_bytes())
        .map_err(|_| SysError::InvalidCString(slave_path.to_string_lossy().into_owned()))?;
    // SAFETY: `c_slave` is NUL-terminated; opening with O_NOCTTY|O_CLOEXEC avoids stealing daemon ctty or leaking fd.
    let slave_raw = unsafe {
        libc::open(
            c_slave.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    if slave_raw < 0 {
        return Err(SysError::path_io(
            "open pty slave",
            &slave_path,
            io::Error::last_os_error(),
        ));
    }
    // SAFETY: `slave_raw` is a newly opened valid file descriptor.
    let slave = unsafe { OwnedFd::from_raw_fd(slave_raw) };

    set_winsize(master.as_raw_fd(), cols, rows)?;
    set_nonblocking(master.as_raw_fd())?;

    Ok(PtyPair {
        master,
        slave,
        slave_path,
    })
}

/// Applies terminal dimensions (`TIOCSWINSZ`) to a PTY descriptor.
pub fn set_winsize(fd: RawFd, cols: u16, rows: u16) -> SysResult<()> {
    let ws = libc::winsize {
        ws_row: rows.max(1),
        ws_col: cols.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `ws` is a valid `libc::winsize` passed to `TIOCSWINSZ`.
    let rc = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ as _, &ws as *const libc::winsize) };
    if rc < 0 {
        return Err(SysError::last_os("ioctl(TIOCSWINSZ)"));
    }
    Ok(())
}

/// Sets `O_NONBLOCK` on a file descriptor (used for Tokio `AsyncFd` integration).
pub fn set_nonblocking(fd: RawFd) -> SysResult<()> {
    // SAFETY: `F_GETFL` reads descriptor status flags without pointer dereference.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(SysError::last_os("fcntl(F_GETFL)"));
    }
    // SAFETY: `F_SETFL` updates status flags with `O_NONBLOCK` enabled.
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        return Err(SysError::last_os("fcntl(F_SETFL, O_NONBLOCK)"));
    }
    Ok(())
}

/// Starts a new POSIX session (`setsid`) so the calling process becomes a session/process-group leader.
pub fn create_new_session() -> SysResult<u32> {
    // SAFETY: `setsid(2)` has no pointer arguments and creates a new session if not already a leader.
    let sid = unsafe { libc::setsid() };
    if sid < 0 {
        return Err(SysError::last_os("setsid"));
    }
    Ok(sid as u32)
}

/// Creates a new session and acquires `tty_fd` as the process's controlling terminal (`TIOCSCTTY`).
pub fn setup_controlling_tty(tty_fd: RawFd) -> SysResult<()> {
    let _ = create_new_session()?;
    // SAFETY: `TIOCSCTTY` with arg `0` attaches `tty_fd` as controlling terminal of the new session.
    let rc = unsafe { libc::ioctl(tty_fd, libc::TIOCSCTTY as _, 0 as libc::c_int) };
    if rc < 0 {
        return Err(SysError::last_os("ioctl(TIOCSCTTY)"));
    }
    Ok(())
}

/// Sends a POSIX signal to an entire process group (`kill(-pgid, sig)`).
/// Returns `Ok(false)` when no processes remain in the group (`ESRCH`).
pub fn signal_process_group(pgid: u32, sig: libc::c_int) -> SysResult<bool> {
    if pgid <= 1 {
        return Err(SysError::InvalidArgument {
            op: "killpg",
            detail: format!("refusing to signal process group {pgid}"),
        });
    }
    let neg_pgid = -(pgid as libc::pid_t);
    // SAFETY: `pgid > 1` is validated above and `sig` is a POSIX signal number.
    let rc = unsafe { libc::kill(neg_pgid, sig) };
    if rc == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        return Ok(false);
    }
    Err(SysError::Io {
        op: "killpg",
        source: err,
    })
}

/// Fills `buf` with cryptographically secure random bytes via `SYS_getrandom` with `/dev/urandom` fallback.
pub fn fill_random_bytes(buf: &mut [u8]) -> SysResult<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let slice = &mut buf[filled..];
        // SAFETY: `slice.as_mut_ptr()` is valid for `slice.len()` bytes.
        let rc = unsafe {
            libc::syscall(
                libc::SYS_getrandom,
                slice.as_mut_ptr(),
                slice.len(),
                0 as libc::c_uint,
            )
        };
        if rc > 0 {
            filled += rc as usize;
            continue;
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        if matches!(err.raw_os_error(), Some(libc::ENOSYS | libc::EPERM)) {
            let mut urandom = File::open("/dev/urandom")
                .map_err(|e| SysError::path_io("open", "/dev/urandom", e))?;
            urandom
                .read_exact(slice)
                .map_err(|e| SysError::path_io("read", "/dev/urandom", e))?;
            return Ok(());
        }
        return Err(SysError::Io {
            op: "getrandom",
            source: err,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_pty_sets_winsize_and_generates_random_bytes() {
        let pair = open_pty(120, 40).expect("open pty");
        assert!(pair.slave_path.to_string_lossy().starts_with("/dev/pts/"));
        set_winsize(pair.master.as_raw_fd(), 80, 24).expect("resize pty");

        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        fill_random_bytes(&mut a).expect("random a");
        fill_random_bytes(&mut b).expect("random b");
        assert_ne!(a, b);
    }

    #[test]
    fn refuses_signaling_invalid_process_groups() {
        assert!(signal_process_group(0, libc::SIGTERM).is_err());
        assert!(signal_process_group(1, libc::SIGTERM).is_err());
    }
}
