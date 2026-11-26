//! Binary PTY WebSocket frame codec (`0x00`-`0x03`), watermark backpressure tracker, and 128 KiB ring buffer.

use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use strata_sys::{namespace, pty};

pub(crate) const TAG_DATA: u8 = 0x00;
pub(crate) const TAG_RESIZE: u8 = 0x01;
pub(crate) const TAG_ACK: u8 = 0x02;
pub(crate) const TAG_EXIT: u8 = 0x03;

pub(crate) const HIGH_WATERMARK_BYTES: usize = 256 * 1024;
pub(crate) const LOW_WATERMARK_BYTES: usize = 64 * 1024;
const RING_BUFFER_CAP_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct PtyResizePayload {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct PtyExitPayload {
    pub(crate) code: i32,
    pub(crate) signal: Option<i32>,
}

/// Decoded client-to-server binary PTY WebSocket frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClientPtyFrame {
    Data(Vec<u8>),
    Resize(PtyResizePayload),
    Ack(u32),
}

/// Decodes a binary WebSocket frame from the client (`0x00` data, `0x01` resize, `0x02` u32 BE ACK).
pub(crate) fn decode_client_frame(raw: &[u8]) -> Result<ClientPtyFrame, &'static str> {
    let Some((&tag, body)) = raw.split_first() else {
        return Err("empty PTY frame");
    };
    match tag {
        TAG_DATA => Ok(ClientPtyFrame::Data(body.to_vec())),
        TAG_RESIZE => {
            let parsed: PtyResizePayload =
                serde_json::from_slice(body).map_err(|_| "invalid PTY resize JSON")?;
            Ok(ClientPtyFrame::Resize(parsed))
        }
        TAG_ACK => {
            if body.len() != 4 {
                return Err("PTY ack payload must be 4 bytes");
            }
            let bytes = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
            Ok(ClientPtyFrame::Ack(bytes))
        }
        _ => Err("unknown PTY frame tag"),
    }
}

/// Encodes a server-to-client binary PTY data frame (`0x00` + payload).
#[must_use]
pub(crate) fn encode_data_frame(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + data.len());
    out.push(TAG_DATA);
    out.extend_from_slice(data);
    out
}

/// Encodes a server-to-client binary PTY exit frame (`0x03` + JSON).
#[must_use]
pub(crate) fn encode_exit_frame(payload: &PtyExitPayload) -> Vec<u8> {
    let json =
        serde_json::to_vec(payload).unwrap_or_else(|_| b"{\"code\":1,\"signal\":null}".to_vec());
    let mut out = Vec::with_capacity(1 + json.len());
    out.push(TAG_EXIT);
    out.extend_from_slice(&json);
    out
}

/// Watermark backpressure tracker pausing PTY reads at 256 KiB and resuming at <= 64 KiB.
#[derive(Debug, Default)]
pub(crate) struct WatermarkFlow {
    in_flight: usize,
    paused: bool,
}

impl WatermarkFlow {
    pub(crate) fn on_sent(&mut self, bytes: usize) {
        self.in_flight = self.in_flight.saturating_add(bytes);
        if self.in_flight >= HIGH_WATERMARK_BYTES {
            self.paused = true;
        }
    }

    pub(crate) fn on_ack(&mut self, bytes: u32) -> bool {
        let was_paused = self.paused;
        self.in_flight = self.in_flight.saturating_sub(bytes as usize);
        if self.paused && self.in_flight <= LOW_WATERMARK_BYTES {
            self.paused = false;
        }
        was_paused && !self.paused
    }

    #[must_use]
    pub(crate) fn is_paused(&self) -> bool {
        self.paused
    }
}

/// Bounded 128 KiB byte ring buffer retaining recent PTY output across short disconnects.
#[derive(Debug, Default)]
pub(crate) struct OutputRing {
    buf: VecDeque<u8>,
}

impl OutputRing {
    pub(crate) fn push(&mut self, data: &[u8]) {
        if data.len() >= RING_BUFFER_CAP_BYTES {
            self.buf.clear();
            self.buf.extend(&data[data.len() - RING_BUFFER_CAP_BYTES..]);
            return;
        }
        let excess = (self.buf.len() + data.len()).saturating_sub(RING_BUFFER_CAP_BYTES);
        for _ in 0..excess {
            self.buf.pop_front();
        }
        self.buf.extend(data);
    }

    #[must_use]
    pub(crate) fn snapshot(&self) -> Vec<u8> {
        self.buf.iter().copied().collect()
    }
}

pub(super) fn read_nonblocking(fd: std::os::fd::RawFd, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: `fd` is a valid non-blocking PTY master fd and `buf` is valid for `buf.len()`.
    let rc = unsafe { libc::read(fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len()) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc as usize)
    }
}

pub(super) fn write_nonblocking(fd: std::os::fd::RawFd, buf: &[u8]) -> io::Result<usize> {
    // SAFETY: `fd` is a valid non-blocking PTY master fd and `buf` is valid for `buf.len()`.
    let rc = unsafe { libc::write(fd, buf.as_ptr().cast::<libc::c_void>(), buf.len()) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc as usize)
    }
}

pub(super) fn terminate_pty_group(pgid: u32) {
    if pgid <= 1 || !namespace::pid_exists(pgid) {
        return;
    }
    let _ = pty::signal_process_group(pgid, libc::SIGHUP);
    if wait_pid_gone(pgid, Duration::from_millis(400)) {
        return;
    }
    let _ = pty::signal_process_group(pgid, libc::SIGTERM);
    if wait_pid_gone(pgid, Duration::from_millis(1200)) {
        return;
    }
    let _ = pty::signal_process_group(pgid, libc::SIGKILL);
}

fn wait_pid_gone(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !namespace::pid_exists(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    !namespace::pid_exists(pid)
}

/// Builds the container login shell invocation for `auto|bash|zsh|fish|nu` PTY requests.
#[must_use]
pub(crate) fn build_pty_shell_command(shell_pref: Option<&str>) -> Vec<String> {
    let normalized = match shell_pref.map(str::trim) {
        Some("bash") => "bash",
        Some("zsh") => "zsh",
        Some("fish") => "fish",
        Some("nu" | "nushell") => "nu",
        _ => "auto",
    };

    let script = if normalized == "auto" {
        concat!(
            "shell=$(awk -F: '$1==\"root\"{print $7; exit}' /etc/passwd 2>/dev/null || true); ",
            "if [ -n \"$shell\" ] && [ -x \"$shell\" ]; then ",
            "export SHELL=\"$shell\"; ",
            "case \"$(basename \"$shell\")\" in nu) exec \"$shell\" --login ;; *) exec \"$shell\" -l ;; esac; ",
            "fi; ",
            "if [ -x /bin/bash ]; then export SHELL=/bin/bash; exec /bin/bash -l; fi; ",
            "export SHELL=/bin/sh; exec /bin/sh -l"
        )
        .to_owned()
    } else {
        let login_flag = if normalized == "nu" { "--login" } else { "-l" };
        format!(
            concat!(
                "target=\"\"; ",
                "for d in /root/.local/share/mise/shims /root/.local/bin /opt/strata/mise/shims /opt/strata/bin /usr/local/bin /usr/bin /bin; do ",
                "if [ -x \"$d/{sh}\" ]; then target=\"$d/{sh}\"; break; fi; ",
                "done; ",
                "if [ -z \"$target\" ]; then target=$(command -v {sh} 2>/dev/null || true); fi; ",
                "if [ -n \"$target\" ] && [ -x \"$target\" ]; then export SHELL=\"$target\"; exec \"$target\" {flag}; fi; ",
                "printf '[strata] requested shell \"{sh}\" is not installed; falling back to default shell.\\r\\n' >&2; ",
                "if [ -x /bin/bash ]; then export SHELL=/bin/bash; exec /bin/bash -l; fi; ",
                "export SHELL=/bin/sh; exec /bin/sh -l"
            ),
            sh = normalized,
            flag = login_flag
        )
    };

    vec!["/bin/sh".to_owned(), "-c".to_owned(), script]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_safe_pty_shell_commands_for_auto_and_modern_shells() {
        let auto = build_pty_shell_command(None);
        assert_eq!(auto[0], "/bin/sh");
        assert!(auto[2].contains("/etc/passwd"));

        let zsh = build_pty_shell_command(Some("zsh"));
        assert!(zsh[2].contains("/root/.local/share/mise/shims"));
        assert!(zsh[2].contains("exec \"$target\" -l"));

        let nu = build_pty_shell_command(Some("nushell"));
        assert!(nu[2].contains("exec \"$target\" --login"));

        let unknown = build_pty_shell_command(Some("zsh;rm -rf /"));
        assert!(unknown[2].contains("/etc/passwd"));
    }

    #[test]
    fn encodes_and_decodes_binary_pty_frames_and_watermark_backpressure() {
        assert_eq!(
            decode_client_frame(&encode_data_frame(b"ls -la\n")),
            Ok(ClientPtyFrame::Data(b"ls -la\n".to_vec()))
        );
        let resize_raw = [vec![TAG_RESIZE], br#"{"cols":120,"rows":40}"#.to_vec()].concat();
        assert_eq!(
            decode_client_frame(&resize_raw),
            Ok(ClientPtyFrame::Resize(PtyResizePayload {
                cols: 120,
                rows: 40
            }))
        );
        let mut ack_raw = vec![TAG_ACK];
        ack_raw.extend_from_slice(&4096_u32.to_be_bytes());
        assert_eq!(decode_client_frame(&ack_raw), Ok(ClientPtyFrame::Ack(4096)));

        let exit_raw = encode_exit_frame(&PtyExitPayload {
            code: 0,
            signal: None,
        });
        assert_eq!(exit_raw[0], TAG_EXIT);

        let mut flow = WatermarkFlow::default();
        flow.on_sent(200 * 1024);
        assert!(!flow.is_paused());
        flow.on_sent(64 * 1024);
        assert!(flow.is_paused());
        assert!(!flow.on_ack(100 * 1024));
        assert!(flow.is_paused());
        assert!(flow.on_ack(120 * 1024));
        assert!(!flow.is_paused());
    }
}
