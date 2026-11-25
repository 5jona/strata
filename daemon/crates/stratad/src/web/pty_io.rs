//! Non-blocking `AsyncFd` PTY master I/O loop with kernel-level watermark backpressure,
//! detach grace timeout, and `-pgid` process reaper.

use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::process::ExitStatusExt;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use strata_sys::pidfd::PidFd;
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;

use crate::web::pty_codec::{
    PtyExitPayload, read_nonblocking, terminate_pty_group, write_nonblocking,
};
use crate::web::pty_session::{PtyRegistry, PtySession};

const DETACH_GRACE_SECS: u64 = 5 * 60;

/// Newtype implementing `AsRawFd` over a shared `Arc<OwnedFd>` for `AsyncFd`.
pub(super) struct SharedMasterFd(pub(super) Arc<OwnedFd>);

impl AsRawFd for SharedMasterFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

pub(super) fn spawn_pty_io_and_reaper(
    registry: Arc<PtyRegistry>,
    session: Arc<PtySession>,
    master: SharedMasterFd,
    mut child: tokio::process::Child,
    pidfd: PidFd,
    mut stdin_rx: mpsc::Receiver<Vec<u8>>,
) {
    tokio::spawn(async move {
        // SAFETY: `master` owns an `Arc<OwnedFd>` keeping the non-blocking PTY master alive for `AsyncFd`.
        let Ok(async_master) = (unsafe { AsyncFd::register(master) }) else {
            session.terminate_now();
            let _ = child.wait().await;
            registry.remove_session(&session.session_id).await;
            return;
        };
        // SAFETY: `pidfd` is exclusively owned for exit notification.
        let async_pidfd = unsafe { AsyncFd::register(pidfd) }.ok();

        let mut buf = vec![0u8; 16 * 1024];
        let mut pending_write: VecDeque<u8> = VecDeque::new();
        let mut grace_tick = tokio::time::interval(Duration::from_secs(15));

        loop {
            if session.closed.load(Ordering::Acquire) {
                break;
            }
            let paused = session.is_read_paused();
            tokio::select! {
                _ = session.read_resume_notify.notified() => {}
                read_res = async_master.readable(), if !paused => {
                    let Ok(mut guard) = read_res else { break };
                    match read_nonblocking(guard.get_inner().as_raw_fd(), &mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let chunk = buf[..n].to_vec();
                            session.ring.lock().await.push(&chunk);
                            let _ = session.out_tx.send(chunk);
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            guard.clear_ready();
                        }
                        Err(_) => break,
                    }
                }
                maybe_in = stdin_rx.recv(), if pending_write.len() < 64 * 1024 => {
                    if let Some(bytes) = maybe_in {
                        pending_write.extend(bytes);
                    } else {
                        break;
                    }
                }
                write_res = async_master.writable(), if !pending_write.is_empty() => {
                    let Ok(mut guard) = write_res else { break };
                    let (a, _) = pending_write.as_slices();
                    match write_nonblocking(guard.get_inner().as_raw_fd(), a) {
                        Ok(n) if n > 0 => {
                            pending_write.drain(..n);
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            guard.clear_ready();
                        }
                        _ => break,
                    }
                }
                _ = async {
                    if let Some(pfd) = &async_pidfd {
                        let _ = pfd.readable().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    while let Ok(n) = read_nonblocking(async_master.get_ref().as_raw_fd(), &mut buf) {
                        if n == 0 { break; }
                        let chunk = buf[..n].to_vec();
                        session.ring.lock().await.push(&chunk);
                        let _ = session.out_tx.send(chunk);
                    }
                    break;
                }
                _ = grace_tick.tick() => {
                    if !session.attached.load(Ordering::Acquire)
                        && let Some(since) = *session.detached_at.lock().await
                        && since.elapsed() >= Duration::from_secs(DETACH_GRACE_SECS)
                    {
                        break;
                    }
                }
            }
        }

        session.closed.store(true, Ordering::Release);
        drop(async_master);
        let pid = session.pid;
        let _ = tokio::task::spawn_blocking(move || terminate_pty_group(pid)).await;
        let payload = match child.wait().await {
            Ok(st) => PtyExitPayload {
                code: st.code().unwrap_or(-1),
                signal: st.signal(),
            },
            Err(_) => PtyExitPayload {
                code: -1,
                signal: None,
            },
        };
        *session.exit_state.write().await = Some(payload);
        session.exit_notify.notify_waiters();
        registry.remove_session(&session.session_id).await;
    });
}
