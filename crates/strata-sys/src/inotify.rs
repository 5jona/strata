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

    /// Drains all queued inotify events, returning `true` if any event targeted `target_name`
    /// (ignoring `.tmp` scratch files) or if an `IN_Q_OVERFLOW` event occurred.
    pub fn drain_events_for(&self, target_name: &str) -> io::Result<bool> {
        let mut buf = [0_u8; 4096];
        let mut matched = false;
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
                if buffer_matches_target(&buf[..n as usize], target_name) {
                    matched = true;
                }
                continue;
            }
            if n == 0 {
                return Ok(matched);
            }
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                return Ok(matched);
            }
            return Err(err);
        }
    }
}

/// Parses raw Linux `inotify_event` records and returns `true` when any event targets `target_name`
/// (ignoring `.tmp` files) or indicates `IN_Q_OVERFLOW`.
fn buffer_matches_target(buf: &[u8], target_name: &str) -> bool {
    let hdr_size = std::mem::size_of::<libc::inotify_event>();
    let mut offset = 0usize;
    while offset + hdr_size <= buf.len() {
        // SAFETY: `offset + hdr_size <= buf.len()` and `read_unaligned` allows arbitrary alignment.
        let ev = unsafe {
            buf.as_ptr()
                .add(offset)
                .cast::<libc::inotify_event>()
                .read_unaligned()
        };
        offset += hdr_size;
        let name_len = ev.len as usize;
        if offset.saturating_add(name_len) > buf.len() {
            break;
        }
        let raw_name = &buf[offset..offset + name_len];
        offset += name_len;

        if ev.mask & libc::IN_Q_OVERFLOW != 0 {
            return true;
        }
        let end = raw_name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(raw_name.len());
        if let Ok(name) = std::str::from_utf8(&raw_name[..end])
            && !name.ends_with(".tmp")
            && name == target_name
        {
            return true;
        }
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_event(mask: u32, name: &[u8]) -> Vec<u8> {
        let mut padded = name.to_vec();
        if !padded.is_empty() {
            padded.push(0);
            while !padded.len().is_multiple_of(4) {
                padded.push(0);
            }
        }
        let ev = libc::inotify_event {
            wd: 1,
            mask,
            cookie: 0,
            len: padded.len() as u32,
        };
        let hdr_size = std::mem::size_of::<libc::inotify_event>();
        let mut out = vec![0u8; hdr_size];
        // SAFETY: `ev` is POD and `out` has exact length `hdr_size`.
        unsafe {
            std::ptr::copy_nonoverlapping((&raw const ev).cast::<u8>(), out.as_mut_ptr(), hdr_size);
        }
        out.extend_from_slice(&padded);
        out
    }

    #[test]
    fn filters_tmp_and_unrelated_events_and_matches_target() {
        let mut buf = encode_event(libc::IN_CLOSE_WRITE, b".config.json.strata.1.1.tmp");
        buf.extend_from_slice(&encode_event(libc::IN_CREATE, b"other.json"));
        assert!(!buffer_matches_target(&buf, "config.json"));

        buf.extend_from_slice(&encode_event(libc::IN_MOVED_TO, b"config.json"));
        assert!(buffer_matches_target(&buf, "config.json"));

        let overflow = encode_event(libc::IN_Q_OVERFLOW, b"");
        assert!(buffer_matches_target(&overflow, "config.json"));
    }
}
