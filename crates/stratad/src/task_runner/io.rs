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

use crate::sideveil_seam::{EventHub, PluginEventSink};
use crate::task_runner::{MAX_TASK_OUTPUT_LINES, TrackedTask};

pub(super) fn spawn_stream_reader<R: AsyncRead + Unpin + Send + 'static>(
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

pub(super) fn spawn_task_waiter(
    mut child: Child,
    pidfd: Option<Arc<PidFd>>,
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
