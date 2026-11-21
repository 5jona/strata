//! Linux namespace isolation (`unshare`, `setns`, `sethostname`, and `/proc/<pid>/ns/*` inspection).

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

use crate::error::{SysError, SysResult};

const CLONE_NEWTIME: libc::c_int = 0x0000_0080;

/// Unshares mount, UTS, cgroup, and (when supported by kernel) time namespaces.
/// Never requires `CLONE_NEWPID`, `CLONE_NEWIPC`, or `CLONE_NEWUSER`.
/// Returns `Ok(true)` if `CLONE_NEWTIME` succeeded, or `Ok(false)` if degraded without it.
pub fn unshare_container_namespaces() -> SysResult<bool> {
    let base = libc::CLONE_NEWNS | libc::CLONE_NEWUTS | libc::CLONE_NEWCGROUP;
    // SAFETY: `unshare(2)` is safe to call in a single-threaded container setup process.
    let rc = unsafe { libc::unshare(base | CLONE_NEWTIME) };
    if rc == 0 {
        return Ok(true);
    }

    let err = io::Error::last_os_error();
    if matches!(
        err.raw_os_error(),
        Some(libc::EINVAL | libc::EPERM | libc::ENOSYS)
    ) {
        // SAFETY: Retrying `unshare(2)` without `CLONE_NEWTIME` on kernels lacking time ns support.
        let retry_rc = unsafe { libc::unshare(base) };
        if retry_rc == 0 {
            return Ok(false);
        }
        return Err(SysError::last_os("unshare(NEWNS|NEWUTS|NEWCGROUP)"));
    }

    Err(SysError::Io {
        op: "unshare(NEWNS|NEWUTS|NEWCGROUP|NEWTIME)",
        source: err,
    })
}

/// Sets the UTS namespace hostname using `SYS_sethostname` (uniform across glibc and bionic).
pub fn set_hostname(hostname: &str) -> SysResult<()> {
    let bytes = hostname.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err(SysError::InvalidArgument {
            op: "sethostname",
            detail: format!("hostname length {} out of range 1..=64", bytes.len()),
        });
    }
    // SAFETY: `bytes.as_ptr()` is valid for `bytes.len()` bytes across `SYS_sethostname`.
    let rc = unsafe { libc::syscall(libc::SYS_sethostname, bytes.as_ptr(), bytes.len()) };
    if rc < 0 {
        return Err(SysError::last_os("sethostname"));
    }
    Ok(())
}

/// Enters an existing container process's UTS, cgroup, optional time, and mount namespaces.
/// Caller must be single-threaded so `setns(CLONE_NEWNS)` succeeds.
pub fn enter_container_namespaces(target_pid: u32) -> SysResult<()> {
    if target_pid <= 1 {
        return Err(SysError::InvalidArgument {
            op: "enter_container_namespaces",
            detail: format!("refusing to enter invalid target pid {target_pid}"),
        });
    }

    let open_ns = |kind: &str| -> SysResult<File> {
        let path = format!("/proc/{target_pid}/ns/{kind}");
        File::open(&path).map_err(|e| SysError::path_io("open ns", path, e))
    };

    // Open all namespace descriptors before modifying any namespace of the calling process.
    let uts_fd = open_ns("uts")?;
    let cgroup_fd = open_ns("cgroup")?;
    let time_fd = File::open(format!("/proc/{target_pid}/ns/time")).ok();
    let mnt_fd = open_ns("mnt")?;

    call_setns(&uts_fd, libc::CLONE_NEWUTS, "setns(uts)")?;
    call_setns(&cgroup_fd, libc::CLONE_NEWCGROUP, "setns(cgroup)")?;
    if let Some(tf) = &time_fd {
        let _ = call_setns(tf, CLONE_NEWTIME, "setns(time)");
    }
    call_setns(&mnt_fd, libc::CLONE_NEWNS, "setns(mnt)")?;
    Ok(())
}

/// Reads the numeric inode identifier from `/proc/<pid>/ns/<kind>`.
pub fn read_ns_inode(pid: u32, kind: &str) -> SysResult<u64> {
    let path = format!("/proc/{pid}/ns/{kind}");
    let target = std::fs::read_link(&path).map_err(|e| SysError::path_io("readlink", &path, e))?;
    let target_str = target.to_string_lossy();
    parse_ns_link(&target_str).ok_or_else(|| SysError::InvalidArgument {
        op: "read_ns_inode",
        detail: format!("unexpected namespace link '{target_str}' at {path}"),
    })
}

/// Parses Linux namespace symlink targets such as `"mnt:[4026532199]"` into their numeric inode.
#[must_use]
pub fn parse_ns_link(raw: &str) -> Option<u64> {
    let start = raw.find(":[")?;
    let rest = &raw[start + 2..];
    let end = rest.find(']')?;
    rest[..end].parse::<u64>().ok()
}

/// Checks whether a process currently exists via `kill(pid, 0)`.
#[must_use]
pub fn pid_exists(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: `kill(pid, 0)` only checks existence/permissions and sends no signal.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Sends a POSIX signal to a PID. Returns `Ok(false)` if the process no longer exists (`ESRCH`).
pub fn send_signal(pid: u32, sig: libc::c_int) -> SysResult<bool> {
    if pid <= 1 {
        return Err(SysError::InvalidArgument {
            op: "kill",
            detail: format!("refusing to signal pid {pid}"),
        });
    }
    // SAFETY: `pid > 1` is validated above and `sig` is a POSIX signal number.
    let rc = unsafe { libc::kill(pid as libc::pid_t, sig) };
    if rc == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        return Ok(false);
    }
    Err(SysError::Io {
        op: "kill",
        source: err,
    })
}

/// Reads PID 1's mount namespace inode to guarantee container process scanners never match host.
pub fn host_mount_ns_inode() -> SysResult<u64> {
    read_ns_inode(1, "mnt")
}

/// Returns whether a directory path exists and is accessible.
#[must_use]
pub fn path_is_dir(path: &Path) -> bool {
    path.is_dir()
}

fn call_setns(file: &File, nstype: libc::c_int, op: &'static str) -> SysResult<()> {
    // SAFETY: `file` is a valid open `/proc/<pid>/ns/*` descriptor matching `nstype`.
    let rc = unsafe { libc::setns(file.as_raw_fd(), nstype) };
    if rc < 0 {
        return Err(SysError::last_os(op));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_namespace_symlink_inodes() {
        assert_eq!(parse_ns_link("mnt:[4026531841]"), Some(4_026_531_841));
        assert_eq!(parse_ns_link("uts:[4026531838]"), Some(4_026_531_838));
        assert_eq!(parse_ns_link("invalid"), None);
        assert_eq!(parse_ns_link("mnt:[]"), None);
    }
}
