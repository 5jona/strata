//! One-shot `ssh` / `adb-host` process execution and trailing JSON payload extraction.

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;

use super::adb_forward::wrap_su_command;
use super::policy::SshAttempt;
use super::routing::TransportRoute;

pub(super) fn annotate_degraded(mut payload: Value, reason: &str) -> Value {
    if let Value::Object(ref mut obj) = payload {
        obj.insert("route".to_owned(), json!("adb-host"));
        obj.insert("degradedReason".to_owned(), json!(reason));
    }
    payload
}

pub(super) async fn run_ssh_once(
    route: &TransportRoute,
    remote_cmd: &str,
    timeout: Duration,
) -> SshAttempt {
    let mut args = route.base_ssh_args();
    args.push(remote_cmd.to_owned());
    run_process_attempt("ssh", &args, route.kind, timeout).await
}

pub(super) async fn run_adb_host_once(
    route: &TransportRoute,
    host_cmd: &str,
    timeout: Duration,
) -> SshAttempt {
    let Some(serial) = route.adb_serial.as_deref() else {
        let err = "adb-host route missing ADB serial".to_owned();
        return SshAttempt {
            exit_code: 255,
            stdout_empty: true,
            has_json: false,
            stderr: err.clone(),
            payload: json!({"ok": false, "rc": 255, "route": "adb-host", "error": err}),
        };
    };
    let args = vec![
        "-s".to_owned(),
        serial.to_owned(),
        "shell".to_owned(),
        wrap_su_command(host_cmd),
    ];
    run_process_attempt("adb", &args, "adb-host", timeout).await
}

async fn run_process_attempt(
    program: &str,
    args: &[String],
    route_kind: &'static str,
    timeout: Duration,
) -> SshAttempt {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let err = format!("spawn {program} failed: {e}");
            return SshAttempt {
                exit_code: 127,
                stdout_empty: true,
                has_json: false,
                stderr: err.clone(),
                payload: json!({"ok": false, "rc": 127, "route": route_kind, "error": err}),
            };
        }
    };

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let rc = out.status.code().unwrap_or(1);
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let stdout_empty = stdout.trim().is_empty();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            if let Some(parsed) = extract_trailing_json(&stdout) {
                let payload = if let Value::Object(mut obj) = parsed {
                    obj.entry("route").or_insert_with(|| json!(route_kind));
                    Value::Object(obj)
                } else {
                    json!({"ok": rc == 0, "rc": rc, "route": route_kind, "data": parsed})
                };
                return SshAttempt {
                    exit_code: rc,
                    stdout_empty,
                    has_json: true,
                    stderr,
                    payload,
                };
            }
            SshAttempt {
                exit_code: rc,
                stdout_empty,
                has_json: false,
                stderr: stderr.clone(),
                payload: json!({"ok": rc == 0, "rc": rc, "route": route_kind, "stdout": stdout.trim(), "stderr": stderr}),
            }
        }
        Ok(Err(e)) => {
            let err = format!("{program} wait failed: {e}");
            SshAttempt {
                exit_code: 1,
                stdout_empty: true,
                has_json: false,
                stderr: err.clone(),
                payload: json!({"ok": false, "rc": 1, "route": route_kind, "error": err}),
            }
        }
        Err(_) => {
            let err = format!("{program} timed out after {}s", timeout.as_secs());
            SshAttempt {
                exit_code: 124,
                stdout_empty: true,
                has_json: false,
                stderr: err.clone(),
                payload: json!({"ok": false, "rc": 124, "route": route_kind, "error": err}),
            }
        }
    }
}

/// Parses stdout as JSON, tolerating leading SSH banners before a trailing JSON object/array.
pub(crate) fn extract_trailing_json(stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Some(v);
    }
    if !(trimmed.ends_with('}') || trimmed.ends_with(']')) {
        return None;
    }
    for (idx, ch) in trimmed.char_indices() {
        if (ch == '{' || ch == '[')
            && let Ok(v) = serde_json::from_str::<Value>(&trimmed[idx..])
        {
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_trailing_json_past_ssh_banner_and_annotates_degraded() {
        let raw = "Warning: Permanently added host\n{\"ok\":true,\"count\":3}\n";
        let v = extract_trailing_json(raw).expect("parsed");
        assert_eq!(v["count"], json!(3));

        let degraded = annotate_degraded(v, "all Strata containers are stopped");
        assert_eq!(degraded["route"], json!("adb-host"));
        assert_eq!(
            degraded["degradedReason"],
            json!("all Strata containers are stopped")
        );
    }
}
