//! Headless streaming command and Python task runner reusable by RPC clients and Phase 2 WebUI.

use std::collections::VecDeque;
use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use strata_protocol::EventEnvelope;
use strata_protocol::rpc::{
    TaskCancelParams, TaskExitPayload, TaskLogsParams, TaskLogsResult, TaskOutputPayload,
    TaskRunParams, TaskRunResult, TaskStream, TaskSummary, error_codes, events,
};
use strata_runtime::{EnterSpec, spec_pipe};
use strata_sys::pidfd::PidFd;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::manager::ContainerManager;
use crate::sideveil_seam::{EventHub, PluginEventSink};

const MAX_RETAINED_TASKS: usize = 32;
const MAX_TASK_OUTPUT_LINES: usize = 256;

#[derive(Debug)]
struct TrackedTask {
    summary: TaskSummary,
    pidfd: Option<Arc<PidFd>>,
    output: VecDeque<TaskOutputPayload>,
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
    pub(crate) async fn run_task(
        &self,
        params: TaskRunParams,
    ) -> Result<TaskRunResult, (i32, String)> {
        if params.command.is_empty() {
            return Err((
                error_codes::INVALID_PARAMS,
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
        };

        let spec_fd =
            spec_pipe::create_spec_pipe(&spec).map_err(|e| (e.rpc_code(), e.to_string()))?;
        let (err_read, err_write) =
            spec_pipe::create_exec_error_pipe().map_err(|e| (e.rpc_code(), e.to_string()))?;

        let mut child = Command::new(&self.self_exe)
            .arg("__exec")
            .arg("--spec-fd")
            .arg(spec_fd.as_raw_fd().to_string())
            .arg("--err-fd")
            .arg(err_write.as_raw_fd().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                (
                    error_codes::INTERNAL_ERROR,
                    format!("spawn task helper: {e}"),
                )
            })?;

        drop(spec_fd);
        drop(err_write);

        let exec_check = tokio::task::spawn_blocking(move || spec_pipe::read_exec_error(err_read))
            .await
            .map_err(|e| (error_codes::INTERNAL_ERROR, format!("task exec check: {e}")))?
            .map_err(|e| (e.rpc_code(), e.to_string()))?;

        if let Some(err_msg) = exec_check {
            let _ = child.wait().await;
            return Err((error_codes::INTERNAL_ERROR, err_msg));
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
        if let Some(stdout) = child.stdout.take() {
            spawn_stream_reader(
                stdout,
                TaskStream::Stdout,
                task_id.clone(),
                container_id.clone(),
                Arc::clone(&seq),
                Arc::clone(&self.tasks),
                Arc::clone(&self.event_hub),
            );
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_stream_reader(
                stderr,
                TaskStream::Stderr,
                task_id.clone(),
                container_id.clone(),
                Arc::clone(&seq),
                Arc::clone(&self.tasks),
                Arc::clone(&self.event_hub),
            );
        }

        let tasks_ref = Arc::clone(&self.tasks);
        let hub_ref = Arc::clone(&self.event_hub);
        let tid = task_id.clone();
        let cid = container_id.clone();
        let timeout_secs = params.timeout_secs;

        tokio::spawn(async move {
            let mut timed_out = false;
            let status = if let Some(secs) = timeout_secs {
                match tokio::time::timeout(Duration::from_secs(secs.max(1)), child.wait()).await {
                    Ok(res) => res.ok(),
                    Err(_) => {
                        timed_out = true;
                        if let Some(pfd) = &pidfd {
                            let _ = pfd.send_signal(libc::SIGTERM);
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            let _ = pfd.send_signal(libc::SIGKILL);
                        } else {
                            let _ = child.kill().await;
                        }
                        child.wait().await.ok()
                    }
                }
            } else {
                child.wait().await.ok()
            };

            let exit_code = status.and_then(|s| s.code()).unwrap_or(-1);
            let signal = status.and_then(|s| s.signal());

            {
                let mut guard = tasks_ref.lock().await;
                if let Some(entry) = guard.iter_mut().find(|t| t.summary.task_id == tid) {
                    entry.summary.running = false;
                    entry.summary.exit_code = Some(exit_code);
                    entry.pidfd = None;
                }
            }

            let payload = serde_json::to_value(TaskExitPayload {
                task_id: tid,
                container: cid,
                exit_code,
                signal,
                timed_out,
            })
            .ok();
            hub_ref.emit(EventEnvelope::new(events::TASK_EXIT, payload));
        });

        Ok(TaskRunResult {
            task_id,
            container: container_id,
            pid,
        })
    }

    /// Cancels a running task via `pidfd_send_signal(SIGTERM)`.
    pub(crate) async fn cancel_task(
        &self,
        params: TaskCancelParams,
    ) -> Result<bool, (i32, String)> {
        let pidfd = {
            let guard = self.tasks.lock().await;
            let Some(entry) = guard.iter().find(|t| t.summary.task_id == params.task_id) else {
                return Err((
                    error_codes::TASK_NOT_FOUND,
                    format!("task '{}' not found", params.task_id),
                ));
            };
            entry.pidfd.clone()
        };
        if let Some(pfd) = pidfd {
            let sent = pfd
                .send_signal(libc::SIGTERM)
                .map_err(|e| (error_codes::INTERNAL_ERROR, e.to_string()))?;
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
    pub(crate) async fn task_logs(
        &self,
        params: TaskLogsParams,
    ) -> Result<TaskLogsResult, (i32, String)> {
        let guard = self.tasks.lock().await;
        let Some(entry) = guard.iter().find(|t| t.summary.task_id == params.task_id) else {
            return Err((
                error_codes::TASK_NOT_FOUND,
                format!("task '{}' not found", params.task_id),
            ));
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

fn spawn_stream_reader<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
    stream: TaskStream,
    task_id: String,
    container: String,
    seq: Arc<AtomicU64>,
    tasks: Arc<Mutex<VecDeque<TrackedTask>>>,
    event_hub: Arc<EventHub>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let next_seq = seq.fetch_add(1, Ordering::Relaxed);
            let item = TaskOutputPayload {
                task_id: task_id.clone(),
                container: container.clone(),
                stream,
                chunk: line,
                seq: next_seq,
            };
            {
                let mut guard = tasks.lock().await;
                if let Some(entry) = guard.iter_mut().find(|t| t.summary.task_id == task_id) {
                    if entry.output.len() >= MAX_TASK_OUTPUT_LINES {
                        entry.output.pop_front();
                    }
                    entry.output.push_back(item.clone());
                }
            }
            let payload = serde_json::to_value(&item).ok();
            event_hub.emit(EventEnvelope::new(events::TASK_OUTPUT, payload));
        }
    });
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
