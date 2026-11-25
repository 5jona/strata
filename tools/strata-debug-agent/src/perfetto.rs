//! Bounded Android Perfetto trace capture and ring-buffer session manager.

mod config;
mod proto;

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::host::{atomic_write, run_host};
use config::{ALLOWED_PREFIX, build_config, now_secs, validate_out_path, validate_session_key};
use proto::summarize_trace_bytes;

const PERFETTO_BIN: &str = "/system/bin/perfetto";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RingSessionMeta {
    pid: u32,
    preset: String,
    #[serde(rename = "bufferKb")]
    buffer_kb: u32,
    #[serde(rename = "maxDurationSec")]
    max_duration_sec: u32,
    #[serde(rename = "remotePath")]
    remote_path: String,
    #[serde(rename = "startedAt")]
    started_at: u64,
}

fn meta_path(session_key: &str) -> String {
    format!("{ALLOWED_PREFIX}strata-perfetto-session-{session_key}.json")
}

async fn stat_remote_size(path: &str) -> u64 {
    if let Ok(m) = std::fs::metadata(path) {
        return m.len();
    }
    let res = run_host(
        &["/system/bin/stat", "-c", "%s", path],
        None,
        Duration::from_secs(5),
    )
    .await;
    res.stdout_text().trim().parse().unwrap_or(0)
}

/// Captures a bounded one-shot Perfetto trace into `/data/local/tmp/`.
pub(crate) async fn capture(
    preset: &str,
    duration_sec: u32,
    buffer_kb: u32,
    out_path: Option<&str>,
) -> Value {
    let dur = duration_sec.clamp(1, 15);
    let buf = buffer_kb.clamp(1024, 32_768);
    let cfg = match build_config(preset, dur, buf) {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let remote = match validate_out_path(out_path, preset) {
        Ok(p) => p,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let res = run_host(
        &[PERFETTO_BIN, "--txt", "-c", "-", "-o", &remote],
        Some(cfg.as_bytes()),
        Duration::from_secs(u64::from(dur) + 15),
    )
    .await;
    let size = if res.ok() {
        stat_remote_size(&remote).await
    } else {
        0
    };
    let ok = res.ok() && size > 0;
    let summary = if ok {
        Some(summarize_trace_file(&remote).await)
    } else {
        None
    };
    json!({
        "ok": ok, "action": "capture", "preset": preset, "durationSec": dur, "bufferKb": buf,
        "remotePath": remote, "sizeBytes": size, "summary": summary,
        "stderr": res.stderr_text().trim().chars().take(600).collect::<String>(),
    })
}

/// Starts a background in-memory ring-buffer Perfetto session.
pub(crate) async fn start_ring(
    preset: &str,
    buffer_kb: u32,
    max_duration_sec: u32,
    session_key: &str,
    out_path: Option<&str>,
) -> Value {
    let key = match validate_session_key(session_key) {
        Ok(k) => k,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let dur = max_duration_sec.clamp(60, 3600);
    let buf = buffer_kb.clamp(1024, 32_768);
    let cfg = match build_config(preset, dur, buf) {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let remote = match validate_out_path(out_path, preset) {
        Ok(p) => p,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let cfg_path = format!("{ALLOWED_PREFIX}strata-perfetto-cfg-{key}.pbtxt");
    let _ = atomic_write(Path::new(&cfg_path), cfg.as_bytes());
    let launch = format!(
        "nohup {PERFETTO_BIN} --txt -c '{cfg_path}' -o '{remote}' </dev/null >/dev/null 2>&1 & echo $!"
    );
    let res = run_host(
        &["/system/bin/sh", "-c", &launch],
        None,
        Duration::from_secs(8),
    )
    .await;
    let pid = res
        .stdout_text()
        .lines()
        .find_map(|l| l.trim().parse::<u32>().ok())
        .filter(|&p| p > 1);
    let Some(pid) = pid else {
        return json!({"ok": false, "action": "start", "error": "failed to spawn background perfetto"});
    };
    let meta = RingSessionMeta {
        pid,
        preset: preset.to_owned(),
        buffer_kb: buf,
        max_duration_sec: dur,
        remote_path: remote.clone(),
        started_at: now_secs(),
    };
    if let Ok(bytes) = serde_json::to_vec(&meta) {
        let _ = atomic_write(Path::new(&meta_path(key)), &bytes);
    }
    json!({"ok": true, "action": "start", "sessionKey": key, "pid": pid, "preset": preset, "remotePath": remote})
}

/// Checks whether a background Perfetto ring-buffer session is active.
pub(crate) async fn ring_status(session_key: &str) -> Value {
    let key = match validate_session_key(session_key) {
        Ok(k) => k,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let meta: Option<RingSessionMeta> = std::fs::read_to_string(meta_path(key))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let Some(m) = meta else {
        return json!({"ok": true, "action": "status", "active": false, "sessionKey": key});
    };
    let pid_s = m.pid.to_string();
    let chk = run_host(
        &[
            "/system/bin/sh",
            "-c",
            r#"kill -0 "$1" 2>/dev/null"#,
            "_",
            &pid_s,
        ],
        None,
        Duration::from_secs(4),
    )
    .await;
    json!({"ok": true, "action": "status", "active": chk.ok(), "sessionKey": key, "meta": m})
}

/// Stops a ring session (flushing trace to disk) and optionally restarts it (`snapshot`).
pub(crate) async fn stop_or_snapshot(session_key: &str, restart: bool) -> Value {
    let key = match validate_session_key(session_key) {
        Ok(k) => k,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let mpath = meta_path(key);
    let meta: Option<RingSessionMeta> = std::fs::read_to_string(&mpath)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let Some(m) = meta else {
        return json!({"ok": false, "error": format!("no active session metadata for '{key}'")});
    };
    let pid_s = m.pid.to_string();
    let _ = run_host(
        &["/system/bin/sh", "-c", r#"kill -INT "$1" 2>/dev/null; for i in 1 2 3 4 5 6 7 8 9 10; do kill -0 "$1" 2>/dev/null || exit 0; sleep 0.3; done; kill -TERM "$1" 2>/dev/null || true"#, "_", &pid_s],
        None,
        Duration::from_secs(10),
    )
    .await;
    let _ = std::fs::remove_file(&mpath);
    let size = stat_remote_size(&m.remote_path).await;
    let ok = size > 0;
    let summary = if ok {
        Some(summarize_trace_file(&m.remote_path).await)
    } else {
        None
    };
    let mut out = json!({
        "ok": ok, "action": if restart { "snapshot" } else { "stop" }, "sessionKey": key,
        "remotePath": m.remote_path, "sizeBytes": size, "summary": summary,
    });
    if restart {
        let next = start_ring(&m.preset, m.buffer_kb, m.max_duration_sec, key, None).await;
        out["restarted"] = next["ok"].clone();
        out["nextRemotePath"] = next["remotePath"].clone();
    }
    out
}

async fn summarize_trace_file(path: &str) -> Value {
    if let Ok(bytes) = std::fs::read(path) {
        let mut s = summarize_trace_bytes(&bytes, 100_000);
        s["sizeBytes"] = json!(bytes.len());
        return s;
    }
    let res = run_host(&["/system/bin/cat", path], None, Duration::from_secs(20)).await;
    if res.ok() && !res.stdout.is_empty() {
        let mut s = summarize_trace_bytes(&res.stdout, 100_000);
        s["sizeBytes"] = json!(res.stdout.len());
        return s;
    }
    json!({"ok": false, "error": format!("trace unreadable: {path}")})
}

/// Summarizes an existing `.pftrace` file on-device (lightweight metadata; deep SQL runs on PC).
pub(crate) async fn info_trace(path: &str) -> Value {
    if path.is_empty() || path.contains("..") {
        return json!({"ok": false, "error": "invalid trace path"});
    }
    let mut out = summarize_trace_file(path).await;
    out["path"] = json!(path);
    out
}
