//! Stream readers, timeout-enforcing child waiter, and timestamp helpers for `TaskRunner`.

use std::collections::VecDeque;
use std::os::unix::process::ExitStatusExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use strata_protocol::EventEnvelope;
use strata_protocol::rpc::{TaskExitPayload, TaskOutputPayload, TaskStream, events};
use strata_sys::pidfd::PidFd;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::sideveil_seam::{EventHub, PluginEventSink};
use crate::task_runner::{MAX_TASK_OUTPUT_LINES, TrackedTask};

const STREAM_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn spawn_stream_reader<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
    stream: TaskStream,
    task_id: String,
    container: String,
    seq: Arc<AtomicU64>,
    tasks: Arc<Mutex<VecDeque<TrackedTask>>>,
    event_hub: Arc<EventHub>,
) -> JoinHandle<()> {
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
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_task_waiter(
    mut child: Child,
    pidfd: Option<Arc<PidFd>>,
    stdout_task: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<()>>,
    task_id: String,
    container_id: String,
    timeout_secs: Option<u64>,
    tasks: Arc<Mutex<VecDeque<TrackedTask>>>,
    event_hub: Arc<EventHub>,
) {
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

        // Drain stdout/stderr reader tasks before emitting `task.exit` so final output lines
        // are always buffered and delivered ahead of the exit notification.
        if let Some(handle) = stdout_task {
            let _ = tokio::time::timeout(STREAM_DRAIN_TIMEOUT, handle).await;
        }
        if let Some(handle) = stderr_task {
            let _ = tokio::time::timeout(STREAM_DRAIN_TIMEOUT, handle).await;
        }

        let exit_code = status.and_then(|s| s.code()).unwrap_or(-1);
        let signal = status.and_then(|s| s.signal());

        {
            let mut guard = tasks.lock().await;
            if let Some(entry) = guard.iter_mut().find(|t| t.summary.task_id == task_id) {
                entry.summary.running = false;
                entry.summary.exit_code = Some(exit_code);
                entry.pidfd = None;
            }
        }

        let payload = serde_json::to_value(TaskExitPayload {
            task_id,
            container: container_id,
            exit_code,
            signal,
            timed_out,
        })
        .ok();
        event_hub.emit(EventEnvelope::new(events::TASK_EXIT, payload));
    });
}

pub(super) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use strata_protocol::rpc::TaskSummary;
    use tokio::process::Command;

    #[tokio::test]
    async fn joins_stream_readers_before_emitting_task_exit() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "echo out-line; echo err-line >&2"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sh");

        let tasks = Arc::new(Mutex::new(VecDeque::from([TrackedTask {
            summary: TaskSummary {
                task_id: "task-1".to_owned(),
                container: "main".to_owned(),
                pid: child.id().unwrap_or(0),
                command: vec!["/bin/sh".to_owned()],
                running: true,
                exit_code: None,
                started_at_ms: 0,
            },
            pidfd: None,
            output: VecDeque::new(),
        }])));
        let hub = Arc::new(EventHub::new());
        let mut sub = hub.subscribe();
        let seq = Arc::new(AtomicU64::new(1));

        let stdout_task = child.stdout.take().map(|r| {
            spawn_stream_reader(
                r,
                TaskStream::Stdout,
                "task-1".to_owned(),
                "main".to_owned(),
                Arc::clone(&seq),
                Arc::clone(&tasks),
                Arc::clone(&hub),
            )
        });
        let stderr_task = child.stderr.take().map(|r| {
            spawn_stream_reader(
                r,
                TaskStream::Stderr,
                "task-1".to_owned(),
                "main".to_owned(),
                Arc::clone(&seq),
                Arc::clone(&tasks),
                Arc::clone(&hub),
            )
        });

        spawn_task_waiter(
            child,
            None,
            stdout_task,
            stderr_task,
            "task-1".to_owned(),
            "main".to_owned(),
            Some(5),
            Arc::clone(&tasks),
            Arc::clone(&hub),
        );

        let exit_env = tokio::time::timeout(Duration::from_secs(3), sub.reliable.recv())
            .await
            .expect("wait task.exit")
            .expect("receive task.exit");
        assert_eq!(exit_env.event, events::TASK_EXIT);

        let guard = tasks.lock().await;
        let chunks: Vec<&str> = guard[0].output.iter().map(|o| o.chunk.as_str()).collect();
        assert!(chunks.contains(&"out-line"), "missing stdout: {chunks:?}");
        assert!(chunks.contains(&"err-line"), "missing stderr: {chunks:?}");
    }
}
