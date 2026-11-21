//! Inherited pipe helpers for passing launch specs without cmdline exposure and exec-sync errors.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{RuntimeError, RuntimeResult};

const MAX_INLINE_SPEC_BYTES: usize = 49_152;

/// Creates a pipe whose read end is inheritable across `exec`, writes JSON-encoded `spec`
/// into the write end, closes the write end, and returns the read `OwnedFd`.
pub fn create_spec_pipe<T: Serialize>(spec: &T) -> RuntimeResult<OwnedFd> {
    let bytes = serde_json::to_vec(spec)?;
    if bytes.len() > MAX_INLINE_SPEC_BYTES {
        return Err(RuntimeError::InvalidArgument(format!(
            "launch spec size {} exceeds inline pipe limit {MAX_INLINE_SPEC_BYTES}",
            bytes.len()
        )));
    }

    let (read_fd, write_fd) = create_pipe_pair(false, true)?;
    let mut writer = File::from(write_fd);
    writer
        .write_all(&bytes)
        .map_err(|source| RuntimeError::Io {
            op: "write spec pipe",
            source,
        })?;
    drop(writer);
    Ok(read_fd)
}

/// Creates a `(parent_read_cloexec, child_write_inheritable)` pipe used to report pre-`execve`
/// stage-2 failures immediately and detect successful `execve` via `FD_CLOEXEC` EOF.
pub fn create_exec_error_pipe() -> RuntimeResult<(OwnedFd, OwnedFd)> {
    create_pipe_pair(true, false)
}

/// Reads and deserializes a JSON spec from an inherited pipe descriptor, closing the fd.
pub fn read_spec_fd<T: DeserializeOwned>(raw_fd: RawFd) -> RuntimeResult<T> {
    if raw_fd < 0 {
        return Err(RuntimeError::InvalidArgument(format!(
            "invalid spec fd {raw_fd}"
        )));
    }
    // SAFETY: `raw_fd` is an exclusively inherited pipe read descriptor passed by our parent.
    let mut reader = unsafe { File::from_raw_fd(raw_fd) };
    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .map_err(|source| RuntimeError::Io {
            op: "read spec pipe",
            source,
        })?;
    Ok(serde_json::from_slice(&buf)?)
}

/// Marks `raw_fd` as `FD_CLOEXEC` so successful `execve` closes it atomically.
pub fn mark_cloexec(raw_fd: RawFd) -> RuntimeResult<()> {
    // SAFETY: `fcntl(F_GETFD)` / `fcntl(F_SETFD)` on an open descriptor is safe.
    let flags = unsafe { libc::fcntl(raw_fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(RuntimeError::Io {
            op: "fcntl(F_GETFD)",
            source: std::io::Error::last_os_error(),
        });
    }
    // SAFETY: Updating descriptor flags with `FD_CLOEXEC`.
    let rc = unsafe { libc::fcntl(raw_fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
    if rc < 0 {
        return Err(RuntimeError::Io {
            op: "fcntl(F_SETFD)",
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(())
}

/// Writes an error message into `err_fd` (if provided) before exiting a failed helper process.
pub fn report_exec_error(err_fd: Option<RawFd>, message: &str) {
    let Some(fd) = err_fd else {
        return;
    };
    if fd < 0 {
        return;
    }
    // SAFETY: `fd` is the inherited error pipe write end owned by this failing helper process.
    let mut file = unsafe { File::from_raw_fd(fd) };
    let _ = file.write_all(message.as_bytes());
    let _ = file.flush();
}

/// Reads any pre-`execve` error string from `read_fd` until EOF; empty string means `execve` succeeded.
pub fn read_exec_error(read_fd: OwnedFd) -> RuntimeResult<Option<String>> {
    let mut reader = File::from(read_fd);
    let mut text = String::new();
    reader
        .read_to_string(&mut text)
        .map_err(|source| RuntimeError::Io {
            op: "read exec error pipe",
            source,
        })?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

fn create_pipe_pair(read_cloexec: bool, write_cloexec: bool) -> RuntimeResult<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a writable 2-element array for `pipe2`.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc < 0 {
        return Err(RuntimeError::Io {
            op: "pipe2",
            source: std::io::Error::last_os_error(),
        });
    }
    // SAFETY: Both descriptors were newly allocated by `pipe2`.
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: Both descriptors were newly allocated by `pipe2`.
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

    if !read_cloexec {
        clear_cloexec(read_fd.as_raw_fd())?;
    }
    if !write_cloexec {
        clear_cloexec(write_fd.as_raw_fd())?;
    }
    Ok((read_fd, write_fd))
}

fn clear_cloexec(raw_fd: RawFd) -> RuntimeResult<()> {
    // SAFETY: `raw_fd` is an open pipe descriptor owned by the caller.
    let flags = unsafe { libc::fcntl(raw_fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(RuntimeError::Io {
            op: "fcntl(F_GETFD)",
            source: std::io::Error::last_os_error(),
        });
    }
    // SAFETY: Clearing `FD_CLOEXEC` so child helper inherits this pipe end.
    let rc = unsafe { libc::fcntl(raw_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) };
    if rc < 0 {
        return Err(RuntimeError::Io {
            op: "fcntl(F_SETFD)",
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(())
}
