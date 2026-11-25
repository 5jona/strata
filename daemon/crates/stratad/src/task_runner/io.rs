//! Bounded-memory stream readers, timeout-enforcing child waiter, and timestamp helpers for `TaskRunner`.

use std::collections::VecDeque;
use std::os::unix::process::ExitStatusExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use strata_protocol::EventEnvelope;
use strata_protocol::rpc::{TaskExitPayload, TaskOutputPayload, TaskStream, events};
use strata_sys::pidfd::PidFd;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::sideveil_seam::{EventHub, PluginEventSink};
use crate::task_runner::{MAX_TASK_OUTPUT_LINES, TrackedTask};

const STREAM_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const MAX_STREAM_LINE_BYTES: usize = 4096;
pub(crate) type TaskExitHook = Box<dyn FnOnce(i32, bool) + Send>;

/// Reads a single `\n`-delimited line while capping RAM allocation at `max_bytes`.
/// Any excess bytes beyond `max_bytes` up to the next `\n` (or EOF) are drained without buffering.
pub(crate) async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<String>> {
    let mut kept = Vec::new();
    let mut saw_any = false;
    let mut truncated = false;

    loop {
        let avail = reader.fill_buf().await?;
        if avail.is_empty() {
            break;
        }
        saw_any = true;
        let (consume, found_nl) = match avail.iter().position(|&b| b == b'\n') {
            Some(pos) => (pos + 1, true),
            None => (avail.len(), false),
        };
        let payload = if found_nl {
            &avail[..consume - 1]
        } else {
            &avail[..consume]
        };
        if kept.len() < max_bytes {
            let room = max_bytes - kept.len();
            let take = payload.len().min(room);
            kept.extend_from_slice(&payload[..take]);
            if take < payload.len() {
                truncated = true;
            }
        } else if !payload.is_empty() {
            truncated = true;
        }
        reader.consume(consume);
        if found_nl {
            break;
        }
    }

    if !saw_any {
        return Ok(None);
    }
    if kept.last() == Some(&b'\r') {
        kept.pop();
    }
    let mut text = String::from_utf8_lossy(&kept).into_owned();
    if truncated {
        text.push_str("...");
    }
    Ok(Some(text))
}

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
        let mut buf_reader = BufReader::new(reader);
        while let Ok(Some(line)) = read_bounded_line(&mut buf_reader, MAX_STREAM_LINE_BYTES).await {
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
    on_exit: Option<TaskExitHook>,
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

        // Drain stdout/stderr reader tasks with timeout and abort if an orphan holds the pipe open.
        if let Some(mut handle) = stdout_task
            && tokio::time::timeout(STREAM_DRAIN_TIMEOUT, &mut handle)
                .await
                .is_err()
        {
            handle.abort();
        }
        if let Some(mut handle) = stderr_task
            && tokio::time::timeout(STREAM_DRAIN_TIMEOUT, &mut handle)
                .await
                .is_err()
        {
            handle.abort();
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
        if let Some(cb) = on_exit {
            cb(exit_code, timed_out);
        }
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
    use std::io::Cursor;
    use std::process::Stdio;
    use strata_protocol::rpc::TaskSummary;
    use tokio::process::Command;

    #[tokio::test]
    async fn bounded_line_reader_caps_unterminated_megabyte_stream_without_oom() {
        let mut huge = vec![b'A'; 128 * 1024];
        huge.extend_from_slice(b"\nsecond-line\r\n");
        let mut reader = BufReader::new(Cursor::new(huge));

        let first = read_bounded_line(&mut reader, 64)
            .await
            .expect("read first")
            .expect("first line");
        assert_eq!(first.len(), 64 + 3);
        assert!(first.ends_with("..."));

        let second = read_bounded_line(&mut reader, 64)
            .await
            .expect("read second")
            .expect("second line");
        assert_eq!(second, "second-line");

        assert_eq!(read_bounded_line(&mut reader, 64).await.expect("eof"), None);

        // Also test stream with NO newline at all.
        let no_nl = vec![b'Z'; 32 * 1024];
        let mut reader2 = BufReader::new(Cursor::new(no_nl));
        let capped = read_bounded_line(&mut reader2, 128)
            .await
            .expect("read no_nl")
            .expect("line");
        assert_eq!(capped.len(), 128 + 3);
        assert!(capped.ends_with("..."));
    }

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
            None,
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
