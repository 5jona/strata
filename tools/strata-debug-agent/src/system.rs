//! L5 read-only kernel diagnostics: pstore ramoops, dmesg, tracefs, and bpftrace presets.

pub(crate) mod android;
pub(crate) mod lsposed;
pub(crate) mod parsers;

use std::path::Path;
use std::time::Duration;

use regex::Regex;
use serde_json::{Value, json};

use crate::host::{host_exec_available, run_host, run_local, tail_chars};
pub(crate) use android::{inspect_ksu, sqlite_inspect};
pub(crate) use lsposed::inspect_lsposed;
use parsers::parse_pstore_text;

const L5_HINT: &str = "Enable read-only L5 mounts via: strata config set-l5-diag on";

/// Reads pstore ramoops files directly or via `host-exec` and highlights OEM watchdog/panic hits.
pub(crate) async fn collect_pstore(pstore_dir: &str) -> Value {
    let dir = Path::new(pstore_dir);
    let mut local_files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("console-ramoops") || name.starts_with("dmesg-ramoops") {
                local_files.push(entry.path());
            }
        }
    }
    local_files.sort();
    if !local_files.is_empty() {
        let mut map = serde_json::Map::new();
        for p in local_files.iter().take(4) {
            let key = p.file_name().map_or_else(
                || "ramoops".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            );
            match std::fs::read_to_string(p) {
                Ok(txt) => {
                    map.insert(key, parse_pstore_text(&txt, 40));
                }
                Err(e) => {
                    map.insert(key, json!({"error": e.to_string()}));
                }
            }
        }
        return json!({"ok": true, "available": true, "source": "local", "files": map});
    }

    if host_exec_available() {
        let cmd = format!(
            "ls {0}/console-ramoops* {0}/dmesg-ramoops* 2>/dev/null || true",
            shell_quote(pstore_dir)
        );
        let res = run_host(
            &["/system/bin/sh", "-c", &cmd],
            None,
            Duration::from_secs(6),
        )
        .await;
        let files: Vec<String> = res
            .stdout_text()
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .take(4)
            .map(str::to_owned)
            .collect();
        if !files.is_empty() {
            let mut map = serde_json::Map::new();
            for rf in files {
                let cat = run_host(&["/system/bin/cat", &rf], None, Duration::from_secs(6)).await;
                let key = Path::new(&rf)
                    .file_name()
                    .map_or(rf.clone(), |n| n.to_string_lossy().into_owned());
                map.insert(key, parse_pstore_text(&cat.stdout_text(), 40));
            }
            return json!({"ok": true, "available": true, "source": "host-exec", "files": map});
        }
    }

    json!({"ok": true, "available": false, "reason": format!("no ramoops files in {pstore_dir}"), "files": {}})
}

/// Collects kernel `dmesg` ring buffer lines with optional regex filter.
pub(crate) async fn collect_dmesg(limit: usize, filter: Option<&str>) -> Value {
    let res = if host_exec_available() {
        run_host(&["/system/bin/dmesg"], None, Duration::from_secs(8)).await
    } else {
        run_local("dmesg", &[], None, Duration::from_secs(8)).await
    };
    if !res.ok() && res.stdout.is_empty() {
        return json!({"ok": true, "available": false, "reason": res.stderr_text(), "lines": []});
    }
    let re = match filter {
        Some(pat) if !pat.is_empty() => match Regex::new(pat) {
            Ok(r) => Some(r),
            Err(e) => return json!({"ok": false, "error": format!("invalid filter regex: {e}")}),
        },
        _ => None,
    };
    let stdout = res.stdout_text();
    let parsed = parse_pstore_text(&stdout, 20);
    let mut lines: Vec<&str> = stdout
        .lines()
        .filter(|ln| re.as_ref().is_none_or(|r| r.is_match(ln)))
        .collect();
    if limit > 0 && lines.len() > limit {
        lines.drain(0..lines.len() - limit);
    }
    json!({"ok": true, "available": true, "count": lines.len(), "watchdogHits": parsed["watchdogHits"], "lines": lines})
}

/// Inspects read-only tracefs and bpffs availability.
pub(crate) fn inspect_tracing(tracing_dir: &str, limit: usize) -> Value {
    let tdir = Path::new(tracing_dir);
    let btf = Path::new("/sys/kernel/btf/vmlinux").exists();
    let tracefs = tdir.join("current_tracer").exists();
    let bpffs = std::fs::read_to_string("/proc/mounts")
        .map(|m| {
            m.lines()
                .any(|l| l.contains(" /sys/fs/bpf ") && l.contains(" bpf "))
        })
        .unwrap_or(false);
    let bpftrace =
        Path::new("/usr/bin/bpftrace").exists() || Path::new("/usr/local/bin/bpftrace").exists();

    let caps =
        json!({"btfVmlinux": btf, "tracefs": tracefs, "bpffsMounted": bpffs, "bpftrace": bpftrace});
    if !tracefs {
        return json!({"ok": true, "available": false, "reason": format!("tracefs not mounted at {tracing_dir}; {L5_HINT}"), "capabilities": caps});
    }
    let current_tracer = std::fs::read_to_string(tdir.join("current_tracer")).unwrap_or_default();
    let tracing_on = std::fs::read_to_string(tdir.join("tracing_on")).unwrap_or_default();
    let trace_raw = std::fs::read_to_string(tdir.join("trace")).unwrap_or_default();
    let mut tail: Vec<&str> = trace_raw
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if limit > 0 && tail.len() > limit {
        tail.drain(0..tail.len() - limit);
    }
    json!({
        "ok": true, "available": true, "tracingDir": tracing_dir,
        "tracingOn": tracing_on.trim() == "1", "currentTracer": current_tracer.trim(),
        "traceTail": tail, "capabilities": caps,
    })
}

/// Runs a bounded read-only `bpftrace` kernel probe preset (`guard-audit`, `net-uid`, `sched-lat`).
pub(crate) async fn run_kernel_probe(preset: &str, duration_sec: u64) -> Value {
    let prog = match preset {
        "guard-audit" => {
            "tracepoint:syscalls:sys_enter_setgid,tracepoint:syscalls:sys_enter_setresgid,tracepoint:syscalls:sys_enter_execve { @[probe, comm, uid] = count(); }"
        }
        "net-uid" => {
            "kprobe:tcp_v4_connect,kprobe:tcp_v6_connect,kprobe:tcp_retransmit_skb { @[func, uid, comm] = count(); }"
        }
        "sched-lat" => {
            "tracepoint:sched:sched_wakeup { @q[args->pid] = nsecs; } tracepoint:sched:sched_switch /@q[args->next_pid]/ { @us = hist((nsecs - @q[args->next_pid]) / 1000); delete(@q[args->next_pid]); }"
        }
        other => return json!({"ok": false, "error": format!("unknown preset '{other}'")}),
    };
    let dur = duration_sec.clamp(1, 30);
    if !Path::new("/sys/kernel/tracing/current_tracer").exists() {
        return json!({"ok": true, "available": false, "preset": preset, "reason": format!("tracefs unavailable; {L5_HINT}")});
    }
    let script = format!("{prog} interval:s:{dur} {{ exit(); }}");
    let res = run_local(
        "bpftrace",
        &["-e", &script],
        None,
        Duration::from_secs(dur + 10),
    )
    .await;
    if res.code == 127 {
        return json!({"ok": true, "available": false, "preset": preset, "reason": "bpftrace not installed; run setup-container.sh --debug-profile"});
    }
    json!({
        "ok": res.ok(), "available": true, "backend": "bpftrace", "preset": preset,
        "durationSec": dur, "output": tail_chars(res.stdout_text().trim(), 4000),
        "stderr": tail_chars(res.stderr_text().trim(), 1000),
    })
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
