//! Headless streaming command and Python task runner reusable by RPC clients and the SideVeil WebUI.

pub(crate) mod io;

use std::collections::VecDeque;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use strata_protocol::rpc::{
    TaskCancelParams, TaskLogsParams, TaskLogsResult, TaskOutputPayload, TaskRunParams,
    TaskRunResult, TaskStream, TaskSummary,
};
use strata_runtime::{EnterSpec, spec_pipe};
use strata_sys::pidfd::PidFd;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::error::{DaemonError, DaemonResult};
use crate::manager::ContainerManager;
use crate::sideveil_seam::EventHub;
use crate::task_runner::io::{now_unix_ms, spawn_stream_reader, spawn_task_waiter};

const MAX_RETAINED_TASKS: usize = 32;
pub(super) const MAX_TASK_OUTPUT_LINES: usize = 256;

#[derive(Debug)]
pub(super) struct TrackedTask {
    pub(super) summary: TaskSummary,
    pub(super) pidfd: Option<Arc<PidFd>>,
    pub(super) output: VecDeque<TaskOutputPayload>,
}

/// Manages daemon-hosted streaming commands inside running containers.
#[derive(Debug)]
pub(crate) struct TaskRunner {
    manager: Arc<ContainerManager>,
    event_hub: Arc<EventHub>,
    self_exe: PathBuf,
    next_id: AtomicU64,
    tasks: Arc<Mutex<VecDeque<TrackedTask>>>,
}

impl TaskRunner {
    #[must_use]
    pub(crate) fn new(
        manager: Arc<ContainerManager>,
        event_hub: Arc<EventHub>,
        self_exe: PathBuf,
    ) -> Self {
        Self {
            manager,
            event_hub,
            self_exe,
            next_id: AtomicU64::new(1),
            tasks: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Spawns a headless command inside a running container and streams `task.output` / `task.exit`.
    pub(crate) async fn run_task(&self, params: TaskRunParams) -> DaemonResult<TaskRunResult> {
        if params.command.is_empty() {
            return Err(DaemonError::InvalidParams(
                "task command must not be empty".to_owned(),
            ));
        }
        let (container_id, ns_pid, cfg) = self
            .manager
            .running_target(params.container.as_deref())
            .await?;

        let spec = EnterSpec {
            container_id: container_id.clone(),
            ns_pid,
            cap_profile: cfg.cap_profile(),
            command: params.command.clone(),
            workdir: params.cwd.clone(),
            term: Some("dumb".to_owned()),
            env: std::collections::BTreeMap::new(),
            pty_ctty: false,
            new_session: false,
        };

        let spec_fd = spec_pipe::create_spec_pipe(&spec)?;
        let (err_read, err_write) = spec_pipe::create_exec_error_pipe()?;
        let spec_raw = spec_fd.as_raw_fd();
        let err_raw = err_write.as_raw_fd();

        let mut cmd = Command::new(&self.self_exe);
        cmd.arg("__exec")
            .arg("--spec-fd")
            .arg(spec_raw.to_string())
            .arg("--err-fd")
            .arg(err_raw.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // SAFETY: Clears `FD_CLOEXEC` only on this child's pipe fds via async-signal-safe `fcntl`.
        unsafe {
            cmd.pre_exec(move || spec_pipe::clear_cloexec_pre_exec(&[spec_raw, err_raw]));
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| DaemonError::Internal(format!("spawn task helper: {e}")))?;

        drop(spec_fd);
        drop(err_write);

        let exec_res =
            tokio::task::spawn_blocking(move || spec_pipe::read_exec_error(err_read)).await;
        match exec_res {
            Ok(Ok(None)) => {}
            Ok(Ok(Some(err_msg))) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(DaemonError::Internal(err_msg));
            }
            Ok(Err(e)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(e.into());
            }
            Err(e) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(DaemonError::join("task exec check", e));
            }
        }

        let pid = child.id().unwrap_or(0);
        let pidfd = (pid > 0)
            .then(|| PidFd::open(pid).ok())
            .flatten()
            .map(Arc::new);
        let task_id = format!("task-{}", self.next_id.fetch_add(1, Ordering::Relaxed));

        let summary = TaskSummary {
            task_id: task_id.clone(),
            container: container_id.clone(),
            pid,
            command: params.command,
            running: true,
            exit_code: None,
            started_at_ms: now_unix_ms(),
        };

        {
            let mut guard = self.tasks.lock().await;
            guard.push_back(TrackedTask {
                summary,
                pidfd: pidfd.clone(),
                output: VecDeque::new(),
            });
            while guard.len() > MAX_RETAINED_TASKS {
                if let Some(idx) = guard.iter().position(|t| !t.summary.running) {
                    guard.remove(idx);
                } else {
                    break;
                }
            }
        }

        let seq = Arc::new(AtomicU64::new(1));
        let stdout_task = child.stdout.take().map(|stdout| {
            spawn_stream_reader(
                stdout,
                TaskStream::Stdout,
                task_id.clone(),
                container_id.clone(),
                Arc::clone(&seq),
                Arc::clone(&self.tasks),
                Arc::clone(&self.event_hub),
            )
        });
        let stderr_task = child.stderr.take().map(|stderr| {
            spawn_stream_reader(
                stderr,
                TaskStream::Stderr,
                task_id.clone(),
                container_id.clone(),
                Arc::clone(&seq),
                Arc::clone(&self.tasks),
                Arc::clone(&self.event_hub),
            )
        });

        spawn_task_waiter(
            child,
            pidfd,
            stdout_task,
            stderr_task,
            task_id.clone(),
            container_id.clone(),
            params.timeout_secs,
            Arc::clone(&self.tasks),
            Arc::clone(&self.event_hub),
        );

        Ok(TaskRunResult {
            task_id,
            container: container_id,
            pid,
        })
    }

    /// Cancels a running task via `pidfd_send_signal(SIGTERM)`.
    pub(crate) async fn cancel_task(&self, params: TaskCancelParams) -> DaemonResult<bool> {
        let pidfd = {
            let guard = self.tasks.lock().await;
            let Some(entry) = guard.iter().find(|t| t.summary.task_id == params.task_id) else {
                return Err(DaemonError::TaskNotFound(params.task_id));
            };
            entry.pidfd.clone()
        };
        if let Some(pfd) = pidfd {
            let sent = pfd
                .send_signal(libc::SIGTERM)
                .map_err(|e| DaemonError::Internal(e.to_string()))?;
            return Ok(sent);
        }
        Ok(false)
    }

    /// Lists active and recently completed tasks.
    pub(crate) async fn list_tasks(&self) -> Vec<TaskSummary> {
        self.tasks
            .lock()
            .await
            .iter()
            .map(|t| t.summary.clone())
            .collect()
    }

    /// Replays buffered stdout/stderr lines for a tracked task (`task.logs`).
    pub(crate) async fn task_logs(&self, params: TaskLogsParams) -> DaemonResult<TaskLogsResult> {
        let guard = self.tasks.lock().await;
        let Some(entry) = guard.iter().find(|t| t.summary.task_id == params.task_id) else {
            return Err(DaemonError::TaskNotFound(params.task_id));
        };
        let max_lines = params
            .limit
            .unwrap_or(MAX_TASK_OUTPUT_LINES)
            .clamp(1, MAX_TASK_OUTPUT_LINES);
        let skip = entry.output.len().saturating_sub(max_lines);
        let entries = entry.output.iter().skip(skip).cloned().collect();
        Ok(TaskLogsResult {
            summary: entry.summary.clone(),
            entries,
        })
    }

    /// Returns the number of currently active tasks.
    pub(crate) async fn active_count(&self) -> usize {
        self.tasks
            .lock()
            .await
            .iter()
            .filter(|t| t.summary.running)
            .count()
    }

    /// Cancels all running tasks during daemon shutdown.
    pub(crate) async fn cancel_all(&self) {
        let pidfds: Vec<Arc<PidFd>> = self
            .tasks
            .lock()
            .await
            .iter()
            .filter_map(|t| t.pidfd.clone())
            .collect();
        for pfd in pidfds {
            let _ = pfd.send_signal(libc::SIGTERM);
        }
    }
}
