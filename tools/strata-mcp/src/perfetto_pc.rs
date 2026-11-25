//! PC-side Perfetto trace analyzer backed by C++ `trace_processor_shell` and curated PerfettoSQL.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;

/// Curated Android PerfettoSQL query presets run on PC `trace_processor_shell`.
pub(crate) fn preset_sql(preset: &str) -> Result<&'static str, String> {
    match preset {
        "summary" => Ok("SELECT ROUND((end_ts - start_ts)/1e6, 2) AS duration_ms, \
             (SELECT COUNT(*) FROM slice) AS slice_count, \
             (SELECT COUNT(*) FROM sched) AS sched_count, \
             (SELECT COUNT(*) FROM process) AS process_count FROM trace_bounds;"),
        "ui-jank" => Ok("INCLUDE PERFETTO MODULE android.frames.timeline; \
             SELECT process_name, COUNT(*) AS total_frames, \
             SUM(CASE WHEN jank_type != 'None' AND jank_type != '' THEN 1 ELSE 0 END) AS janky_frames, \
             ROUND(MAX(dur)/1e6, 2) AS max_frame_ms \
             FROM actual_frame_timeline_slice \
             GROUP BY process_name ORDER BY janky_frames DESC LIMIT 15;"),
        "startup" => Ok("INCLUDE PERFETTO MODULE android.startup.startups; \
             SELECT package, startup_type, ROUND(dur/1e6, 2) AS dur_ms \
             FROM android_startups ORDER BY dur DESC LIMIT 15;"),
        "binder" => Ok("INCLUDE PERFETTO MODULE android.binder; \
             SELECT client_process, server_process, aidl_name, \
             COUNT(*) AS calls, ROUND(MAX(client_dur)/1e6, 2) AS max_ms \
             FROM android_binder_txns GROUP BY client_process, server_process, aidl_name \
             ORDER BY max_ms DESC LIMIT 20;"),
        "cpu" => Ok("SELECT p.name AS process, t.name AS thread, \
             ROUND(SUM(s.dur)/1e6, 2) AS cpu_ms, COUNT(*) AS switches \
             FROM sched s JOIN thread t USING(utid) LEFT JOIN process p USING(upid) \
             WHERE s.utid != 0 GROUP BY p.name, t.name ORDER BY cpu_ms DESC LIMIT 20;"),
        other => Err(format!(
            "unknown analysisPreset '{other}'; expected summary|ui-jank|startup|binder|cpu"
        )),
    }
}

/// Validates custom PerfettoSQL to ensure read-only analytical queries.
pub(crate) fn validate_perfetto_sql(sql: &str) -> Result<&str, String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("empty PerfettoSQL query".to_owned());
    }
    let upper = trimmed.to_ascii_uppercase();
    for forbidden in [
        "ATTACH ", "DETACH ", "DROP ", "DELETE ", "UPDATE ", "INSERT ",
    ] {
        if upper.contains(forbidden) {
            return Err(format!(
                "disallowed mutating SQL keyword: {}",
                forbidden.trim()
            ));
        }
    }
    Ok(trimmed)
}

/// Locates `trace_processor_shell` or `trace_processor` on PC PATH / env override.
pub(crate) fn find_trace_processor() -> Option<PathBuf> {
    if let Ok(env_bin) = std::env::var("STRATA_TRACE_PROCESSOR_BIN") {
        let p = PathBuf::from(env_bin.trim());
        if p.is_file() {
            return Some(p);
        }
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for name in ["trace_processor_shell", "trace_processor"] {
            let cand = dir.join(name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Analyzes a local `.pftrace` file on PC via C++ `trace_processor_shell` (or wire fallback).
pub(crate) async fn analyze_local_trace(
    local_path: &str,
    analysis_preset: Option<&str>,
    custom_sql: Option<&str>,
) -> Value {
    let path = Path::new(local_path);
    if !path.is_file() {
        return json!({"ok": false, "error": format!("local trace file not found: {local_path}")});
    }
    let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let sql = match custom_sql.map(str::trim).filter(|s| !s.is_empty()) {
        Some(q) => match validate_perfetto_sql(q) {
            Ok(valid) => valid.to_owned(),
            Err(e) => return json!({"ok": false, "error": e}),
        },
        None => match preset_sql(analysis_preset.unwrap_or("summary")) {
            Ok(q) => q.to_owned(),
            Err(e) => return json!({"ok": false, "error": e}),
        },
    };

    if let Some(tp_bin) = find_trace_processor() {
        let out = tokio::time::timeout(
            Duration::from_secs(45),
            Command::new(&tp_bin)
                .args(["-Q", &sql, "--output-format=json", local_path])
                .output(),
        )
        .await;
        match out {
            Ok(Ok(res)) if res.status.success() => {
                let stdout = String::from_utf8_lossy(&res.stdout);
                let parsed: Value =
                    serde_json::from_str(&stdout).unwrap_or_else(|_| json!(stdout.trim()));
                return json!({
                    "ok": true,
                    "analyzer": "trace_processor_shell",
                    "localPath": local_path,
                    "sizeBytes": size_bytes,
                    "sql": sql,
                    "result": parsed,
                });
            }
            Ok(Ok(res)) => {
                return json!({
                    "ok": false,
                    "analyzer": "trace_processor_shell",
                    "localPath": local_path,
                    "sql": sql,
                    "stderr": String::from_utf8_lossy(&res.stderr).trim().chars().take(800).collect::<String>(),
                });
            }
            Ok(Err(e)) => {
                return json!({"ok": false, "error": format!("spawn trace_processor failed: {e}")});
            }
            Err(_) => return json!({"ok": false, "error": "trace_processor timed out after 45s"}),
        }
    }

    let fallback = wire_summary_fallback(path);
    json!({
        "ok": true,
        "analyzer": "wire-fallback",
        "localPath": local_path,
        "sizeBytes": size_bytes,
        "summary": fallback,
        "hint": "Install Perfetto trace_processor_shell on PC (or set STRATA_TRACE_PROCESSOR_BIN) to enable full PerfettoSQL analysis.",
    })
}

/// Lightweight protobuf packet summary fallback when `trace_processor_shell` is not installed on PC.
pub(crate) fn wire_summary_fallback(path: &Path) -> Value {
    let Ok(data) = std::fs::read(path) else {
        return json!({"error": "failed to read trace file"});
    };
    let mut pos = 0usize;
    let mut packets = 0usize;
    let mut by_field: BTreeMap<u64, usize> = BTreeMap::new();
    while pos < data.len() && packets < 100_000 {
        let Some((tag, p1)) = read_varint(&data, pos) else {
            break;
        };
        pos = p1;
        let (field, wire) = (tag >> 3, tag & 7);
        *by_field.entry(field).or_insert(0) += 1;
        if field == 1 && wire == 2 {
            packets += 1;
        }
        pos = match wire {
            0 => read_varint(&data, pos).map_or(data.len(), |(_, p)| p),
            1 => (pos + 8).min(data.len()),
            2 => read_varint(&data, pos)
                .map_or(data.len(), |(l, p)| (p + l as usize).min(data.len())),
            5 => (pos + 4).min(data.len()),
            _ => break,
        };
    }
    json!({"packetCount": packets, "topLevelFieldCounts": by_field})
}

fn read_varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let (mut val, mut shift) = (0u64, 0u32);
    while pos < buf.len() && shift < 70 {
        let b = buf[pos];
        pos += 1;
        val |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return Some((val, pos));
        }
        shift += 7;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_perfetto_sql_and_presets() {
        assert!(
            preset_sql("ui-jank")
                .expect("ui-jank")
                .contains("actual_frame_timeline_slice")
        );
        assert!(
            preset_sql("binder")
                .expect("binder")
                .contains("android_binder_txns")
        );
        assert!(preset_sql("unknown").is_err());
        assert!(validate_perfetto_sql("SELECT * FROM slice LIMIT 5").is_ok());
        assert!(validate_perfetto_sql("DROP TABLE slice").is_err());
    }
}
