//! Async SSH ControlMaster execution and ADB-SSH failover loop.

pub(crate) mod routing;
mod transfer;

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;

use crate::devices::DeviceRegistry;
pub(crate) use routing::{
    TargetOverride, TransportRoute, argv_to_shell, build_routes, shell_quote,
};
pub(crate) use transfer::{open_ssh_port_forward, transfer_pull, transfer_push};

const MAX_TIMEOUT_SEC: f64 = 120.0;

/// Executes an argv command remotely via SSH ControlMaster with ADB-SSH auto-failover.
pub(crate) async fn ssh_exec_argv(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    argv: &[String],
    timeout_sec: f64,
) -> Value {
    let remote_cmd = argv_to_shell(argv);
    ssh_exec_raw(registry, target, &remote_cmd, timeout_sec).await
}

/// Executes a remote command string over SSH with candidate route failover and JSON extraction.
pub(crate) async fn ssh_exec_raw(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    remote_cmd: &str,
    timeout_sec: f64,
) -> Value {
    let routes = match build_routes(registry, target).await {
        Ok(r) => r,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let eff = Duration::from_secs_f64(timeout_sec.clamp(1.0, MAX_TIMEOUT_SEC));
    let mut last = json!({"ok": false, "rc": 255, "error": "no transport route available"});

    for (idx, route) in routes.iter().enumerate() {
        if route.kind == "adb-ssh"
            && let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port)
            && let Err(err) = routing::ensure_adb_forward(ser, lp, route.remote_ssh_port).await
        {
            last = json!({"ok": false, "rc": 255, "route": route.kind, "error": err});
            continue;
        }

        let mut res = run_ssh_once(route, remote_cmd, eff).await;
        if route.kind == "adb-ssh"
            && res["rc"].as_i64() == Some(255)
            && let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port)
        {
            routing::reset_control_socket(route).await;
            if routing::ensure_adb_forward(ser, lp, route.remote_ssh_port)
                .await
                .is_ok()
            {
                res = run_ssh_once(route, remote_cmd, eff).await;
            }
        }
        let rc = res["rc"].as_i64().unwrap_or(255);
        last = res;
        if rc != 124 && rc != 255 {
            return last;
        }
        if idx + 1 < routes.len() {
            routing::reset_control_socket(route).await;
        }
    }
    last
}

async fn run_ssh_once(route: &TransportRoute, remote_cmd: &str, timeout: Duration) -> Value {
    let mut args = route.base_ssh_args();
    args.push(remote_cmd.to_owned());
    let mut cmd = Command::new("ssh");
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return json!({"ok": false, "rc": 127, "route": route.kind, "error": format!("spawn ssh failed: {e}")});
        }
    };

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let rc = out.status.code().unwrap_or(1);
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            if let Some(parsed) = extract_trailing_json(&stdout) {
                if let Value::Object(mut obj) = parsed {
                    obj.entry("route").or_insert_with(|| json!(route.kind));
                    return Value::Object(obj);
                }
                return json!({"ok": rc == 0, "rc": rc, "route": route.kind, "data": parsed});
            }
            json!({"ok": rc == 0, "rc": rc, "route": route.kind, "stdout": stdout.trim(), "stderr": stderr})
        }
        Ok(Err(e)) => {
            json!({"ok": false, "rc": 1, "route": route.kind, "error": format!("ssh wait failed: {e}")})
        }
        Err(_) => {
            json!({"ok": false, "rc": 124, "route": route.kind, "error": format!("ssh timed out after {}s", timeout.as_secs())})
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
    fn extracts_trailing_json_past_ssh_banner() {
        let raw = "Warning: Permanently added host\n{\"ok\":true,\"count\":3}\n";
        let v = extract_trailing_json(raw).expect("parsed");
        assert_eq!(v["count"], json!(3));
    }
}
