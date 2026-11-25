//! Live detachable PTY session registry, `Arc<OwnedFd>` master ownership,
//! attach-generation handoff, and `-pgid` cleanup.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use strata_protocol::CapProfile;
use strata_protocol::constants::DEFAULT_CONTAINER_WORKDIR;
use strata_runtime::{EnterSpec, spec_pipe};
use strata_sys::{pidfd::PidFd, pty};
use tokio::process::Command;
use tokio::sync::{Mutex, Notify, RwLock, broadcast, mpsc};

use crate::wakelock::{WakeLockGuard, WakeLockManager};
use crate::web::pty_codec::terminate_pty_group;
pub(crate) use crate::web::pty_codec::{
    ClientPtyFrame, OutputRing, PtyExitPayload, WatermarkFlow, decode_client_frame,
    encode_data_frame, encode_exit_frame,
};
use crate::web::pty_io::{SharedMasterFd, spawn_pty_io_and_reaper};

const MAX_PTY_SESSIONS: usize = 16;

/// Live detachable PTY session inside a container.
pub(crate) struct PtySession {
    pub(crate) session_id: String,
    pub(crate) container_id: String,
    pub(crate) pid: u32,
    pub(crate) stdin_tx: mpsc::Sender<Vec<u8>>,
    master: Arc<OwnedFd>,
    pub(crate) out_tx: broadcast::Sender<Vec<u8>>,
    pub(crate) ring: Mutex<OutputRing>,
    pub(crate) exit_state: RwLock<Option<PtyExitPayload>>,
    pub(crate) exit_notify: Notify,
    pub(crate) attached: AtomicBool,
    pub(crate) attach_gen: AtomicU64,
    pub(crate) attach_notify: Notify,
    pub(crate) detached_at: Mutex<Option<Instant>>,
    read_paused: AtomicBool,
    pub(super) read_resume_notify: Notify,
    pub(crate) closed: AtomicBool,
    _wakelock: WakeLockGuard,
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtySession")
            .field("session_id", &self.session_id)
            .field("container_id", &self.container_id)
            .field("pid", &self.pid)
            .finish()
    }
}

impl PtySession {
    pub(crate) fn resize(&self, cols: u16, rows: u16) {
        if !self.closed.load(Ordering::Acquire) {
            let _ = pty::set_winsize(self.master.as_raw_fd(), cols, rows);
        }
    }

    pub(crate) async fn mark_attached(&self) -> u64 {
        let generation = self.attach_gen.fetch_add(1, Ordering::AcqRel) + 1;
        self.attached.store(true, Ordering::Release);
        *self.detached_at.lock().await = None;
        self.set_read_paused(false);
        self.attach_notify.notify_waiters();
        generation
    }

    pub(crate) async fn mark_detached(&self, my_gen: u64) {
        if self.attach_gen.load(Ordering::Acquire) == my_gen {
            self.attached.store(false, Ordering::Release);
            *self.detached_at.lock().await = Some(Instant::now());
            self.set_read_paused(false);
        }
    }

    pub(crate) fn is_current_generation(&self, my_gen: u64) -> bool {
        self.attach_gen.load(Ordering::Acquire) == my_gen
    }

    pub(crate) fn set_read_paused(&self, paused: bool) {
        let prev = self.read_paused.swap(paused, Ordering::AcqRel);
        if prev && !paused {
            self.read_resume_notify.notify_one();
        }
    }

    pub(crate) fn is_read_paused(&self) -> bool {
        self.read_paused.load(Ordering::Acquire)
    }

    pub(crate) fn terminate_now(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.read_resume_notify.notify_one();
            let pgid = self.pid;
            tokio::task::spawn_blocking(move || terminate_pty_group(pgid));
        }
    }
}

/// Registry owning active and recently detached PTY sessions across all containers.
#[derive(Debug)]
pub(crate) struct PtyRegistry {
    next_id: AtomicU64,
    sessions: RwLock<HashMap<String, Arc<PtySession>>>,
    wakelock: Arc<WakeLockManager>,
}

impl PtyRegistry {
    #[must_use]
    pub(crate) fn new(wakelock: Arc<WakeLockManager>) -> Self {
        Self {
            next_id: AtomicU64::new(1),
            sessions: RwLock::new(HashMap::new()),
            wakelock,
        }
    }

    pub(crate) async fn active_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    pub(crate) async fn get(&self, session_id: &str) -> Option<Arc<PtySession>> {
        self.sessions.read().await.get(session_id).cloned()
    }

    pub(super) async fn remove_session(&self, session_id: &str) {
        self.sessions.write().await.remove(session_id);
    }

    /// Explicitly terminates and removes one PTY session by ID.
    pub(crate) async fn kill_session(&self, session_id: &str) -> bool {
        let removed = self.sessions.write().await.remove(session_id);
        if let Some(sess) = removed {
            sess.terminate_now();
            true
        } else {
            false
        }
    }

    /// Terminates every PTY session across all containers (e.g. on token rotation).
    pub(crate) async fn kill_all(&self) -> usize {
        let all: Vec<Arc<PtySession>> = {
            let mut map = self.sessions.write().await;
            map.drain().map(|(_, s)| s).collect()
        };
        let count = all.len();
        for sess in all {
            sess.terminate_now();
        }
        count
    }

    /// Terminates every PTY session belonging to `container_id` (invoked before teardown).
    pub(crate) async fn close_container_sessions(&self, container_id: &str) {
        let to_kill: Vec<Arc<PtySession>> = {
            let mut map = self.sessions.write().await;
            let keys: Vec<String> = map
                .iter()
                .filter(|(_, s)| s.container_id == container_id)
                .map(|(k, _)| k.clone())
                .collect();
            keys.into_iter().filter_map(|k| map.remove(&k)).collect()
        };
        for sess in to_kill {
            sess.terminate_now();
        }
    }

    /// Evicts the oldest unattached (`!attached`) PTY session to free a slot for a new session.
    async fn evict_oldest_detached(&self) -> bool {
        let candidates: Vec<(String, Arc<PtySession>)> = {
            let map = self.sessions.read().await;
            map.iter()
                .filter(|(_, s)| !s.attached.load(Ordering::Acquire))
                .map(|(k, v)| (k.clone(), Arc::clone(v)))
                .collect()
        };
        let mut oldest: Option<(String, Instant)> = None;
        for (id, sess) in candidates {
            let since = sess.detached_at.lock().await.unwrap_or_else(Instant::now);
            if oldest.as_ref().is_none_or(|(_, prev)| since < *prev) {
                oldest = Some((id, since));
            }
        }
        if let Some((evict_id, _)) = oldest {
            self.kill_session(&evict_id).await
        } else {
            false
        }
    }

    /// Spawns a new interactive PTY session inside a running container via `stratad __exec`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_session(
        self: &Arc<Self>,
        self_exe: &Path,
        container_id: String,
        ns_pid: u32,
        cap_profile: CapProfile,
        cols: u16,
        rows: u16,
        shell: Option<Vec<String>>,
    ) -> Result<Arc<PtySession>, String> {
        if self.sessions.read().await.len() >= MAX_PTY_SESSIONS
            && !self.evict_oldest_detached().await
        {
            return Err(format!(
                "maximum concurrent PTY sessions ({MAX_PTY_SESSIONS}) reached"
            ));
        }

        let pair = pty::open_pty(cols.max(20), rows.max(5)).map_err(|e| e.to_string())?;
        let slave_in = pair.slave;
        let slave_out = slave_in
            .try_clone()
            .map_err(|e| format!("dup pty slave stdout: {e}"))?;
        let slave_err = slave_in
            .try_clone()
            .map_err(|e| format!("dup pty slave stderr: {e}"))?;

        let command = shell.unwrap_or_else(|| {
            vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "if [ -x /bin/bash ]; then exec /bin/bash -l; else exec /bin/sh -l; fi".to_owned(),
            ]
        });
        let spec = EnterSpec {
            container_id: container_id.clone(),
            ns_pid,
            cap_profile,
            command,
            workdir: Some(DEFAULT_CONTAINER_WORKDIR.to_owned()),
            term: Some("xterm-256color".to_owned()),
            env: std::collections::BTreeMap::new(),
            pty_ctty: true,
            new_session: true,
        };

        let spec_fd = spec_pipe::create_spec_pipe(&spec).map_err(|e| e.to_string())?;
        let (err_read, err_write) =
            spec_pipe::create_exec_error_pipe().map_err(|e| e.to_string())?;
        let spec_raw = spec_fd.as_raw_fd();
        let err_raw = err_write.as_raw_fd();

        let mut cmd = Command::new(self_exe);
        cmd.arg("__exec")
            .arg("--spec-fd")
            .arg(spec_raw.to_string())
            .arg("--err-fd")
            .arg(err_raw.to_string())
            .stdin(Stdio::from(slave_in))
            .stdout(Stdio::from(slave_out))
            .stderr(Stdio::from(slave_err));
        // SAFETY: Clears `FD_CLOEXEC` only for this child's pipe fds via async-signal-safe `fcntl`.
        unsafe {
            cmd.pre_exec(move || spec_pipe::clear_cloexec_pre_exec(&[spec_raw, err_raw]));
        }
        let mut child = cmd.spawn().map_err(|e| format!("spawn pty helper: {e}"))?;

        drop(spec_fd);
        drop(err_write);

        let exec_res =
            tokio::task::spawn_blocking(move || spec_pipe::read_exec_error(err_read)).await;
        match exec_res {
            Ok(Ok(None)) => {}
            Ok(Ok(Some(msg))) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(msg);
            }
            Ok(Err(e)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(e.to_string());
            }
            Err(e) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(format!("pty exec check join: {e}"));
            }
        }

        let pid = child.id().unwrap_or(0);
        let pidfd = PidFd::open(pid).map_err(|e| format!("pidfd_open({pid}): {e}"))?;
        let session_id = format!("pty-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (stdin_tx, stdin_rx) = mpsc::channel::<Vec<u8>>(64);
        let (out_tx, _) = broadcast::channel::<Vec<u8>>(128);
        let master = Arc::new(pair.master);

        let session = Arc::new(PtySession {
            session_id: session_id.clone(),
            container_id,
            pid,
            stdin_tx,
            master: Arc::clone(&master),
            out_tx,
            ring: Mutex::new(OutputRing::default()),
            exit_state: RwLock::new(None),
            exit_notify: Notify::new(),
            attached: AtomicBool::new(false),
            attach_gen: AtomicU64::new(0),
            attach_notify: Notify::new(),
            detached_at: Mutex::new(Some(Instant::now())),
            read_paused: AtomicBool::new(false),
            read_resume_notify: Notify::new(),
            closed: AtomicBool::new(false),
            _wakelock: self.wakelock.acquire(),
        });

        {
            let mut map = self.sessions.write().await;
            if map.len() >= MAX_PTY_SESSIONS {
                drop(map);
                session.terminate_now();
                let _ = child.wait().await;
                return Err(format!(
                    "maximum concurrent PTY sessions ({MAX_PTY_SESSIONS}) reached"
                ));
            }
            map.insert(session_id.clone(), Arc::clone(&session));
        }

        spawn_pty_io_and_reaper(
            Arc::clone(self),
            Arc::clone(&session),
            SharedMasterFd(master),
            child,
            pidfd,
            stdin_rx,
        );
        Ok(session)
    }
}
