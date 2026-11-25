//! L2a incremental logcat cursor manager with atomic watermark persistence.

mod parser;

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::host::{atomic_write, run_host};
use parser::{extract_crashes, filter_lines, fnv1a_hex, parse_threadtime_line};

pub(crate) const DEFAULT_CURSOR_PATH: &str = "/tmp/strata-debug-logcat.cursor";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CursorState {
    ts: String,
    #[serde(rename = "lastDigest", default)]
    last_digest: String,
}

async fn probe_latest_watermark() -> CursorState {
    let tail = run_host(
        &["/system/bin/logcat", "-d", "-v", "threadtime", "-t", "1"],
        None,
        Duration::from_secs(8),
    )
    .await;
    for line in tail.stdout_text().lines().rev() {
        if let Some(parsed) = parse_threadtime_line(line) {
            return CursorState {
                ts: parsed.ts,
                last_digest: fnv1a_hex(line),
            };
        }
    }
    let dt = run_host(
        &["/system/bin/date", "+%m-%d %H:%M:%S.000"],
        None,
        Duration::from_secs(4),
    )
    .await;
    let ts = dt.stdout_text().trim().to_owned();
    CursorState {
        ts: if ts.is_empty() {
            "01-01 00:00:00.000".to_owned()
        } else {
            ts
        },
        last_digest: String::new(),
    }
}

/// Records the current logcat watermark atomically at `cursor_path`.
pub(crate) async fn mark_cursor(cursor_path: &str) -> Value {
    let state = probe_latest_watermark().await;
    match serde_json::to_vec(&state)
        .map_err(|e| std::io::Error::other(e.to_string()))
        .and_then(|bytes| atomic_write(Path::new(cursor_path), &bytes))
    {
        Ok(()) => json!({"ok": true, "marked": state}),
        Err(err) => json!({"ok": false, "error": format!("failed to persist cursor: {err}")}),
    }
}

/// Reads incremental logcat entries since cursor, advancing cursor unless `no_advance` is set.
pub(crate) async fn read_incremental(
    cursor_path: &str,
    priority: &str,
    tag: Option<&str>,
    pid: Option<u32>,
    limit: usize,
    advance: bool,
) -> Value {
    let (state, warning) = match std::fs::read_to_string(cursor_path) {
        Ok(raw) => match serde_json::from_str::<CursorState>(&raw) {
            Ok(s) if !s.ts.trim().is_empty() => (Some(s), None),
            Ok(_) | Err(_) => (
                None,
                Some(format!(
                    "corrupt cursor file '{cursor_path}'; falling back to recent tail"
                )),
            ),
        },
        Err(_) => (
            None,
            Some(format!(
                "cursor file '{cursor_path}' not found; falling back to recent tail"
            )),
        ),
    };

    let tail_str = (limit.saturating_mul(4).max(200)).to_string();
    let res = if let Some(st) = state.as_ref() {
        let cmd = ["/system/bin/logcat", "-d", "-v", "threadtime", "-T", &st.ts];
        run_host(&cmd, None, Duration::from_secs(10)).await
    } else {
        let cmd = [
            "/system/bin/logcat",
            "-d",
            "-v",
            "threadtime",
            "-t",
            &tail_str,
        ];
        run_host(&cmd, None, Duration::from_secs(10)).await
    };

    let stdout = res.stdout_text();
    let lines: Vec<&str> = stdout.lines().collect();
    let last_digest = state.as_ref().map_or("", |s| s.last_digest.as_str());
    let (entries, latest_ts, latest_digest) =
        filter_lines(&lines, priority, tag, pid, last_digest, limit);

    if res.ok() && advance && !latest_ts.is_empty() {
        let next = CursorState {
            ts: latest_ts.clone(),
            last_digest: latest_digest,
        };
        if let Ok(bytes) = serde_json::to_vec(&next) {
            let _ = atomic_write(Path::new(cursor_path), &bytes);
        }
    }

    let crashes = extract_crashes(&entries, 20);
    let cursor_out = if !latest_ts.is_empty() {
        latest_ts
    } else {
        state.map_or_else(String::new, |s| s.ts)
    };
    let mut out =
        json!({"ok": res.ok(), "cursor": cursor_out, "count": entries.len(), "entries": entries});
    if !crashes.is_empty() {
        out["crashes"] = json!(crashes);
    }
    if let Some(w) = warning {
        out["warning"] = json!(w);
    }
    if !res.ok() {
        out["error"] = json!(res.stderr_text().chars().take(400).collect::<String>());
    }
    out
}
