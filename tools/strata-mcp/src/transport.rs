//! Async SSH ControlMaster execution and ADB-SSH failover loop.

mod policy;
pub(crate) mod routing;
mod transfer;

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;

use crate::devices::DeviceRegistry;
pub(crate) use policy::RetryPolicy;
use policy::{RouteRunner, SshAttempt, execute_routes_with_policy};
pub(crate) use routing::{
    TargetOverride, TransportRoute, argv_to_shell, build_routes, shell_quote,
};
pub(crate) use transfer::{open_ssh_port_forward, transfer_pull, transfer_push};

const MAX_TIMEOUT_SEC: f64 = 120.0;

/// Executes an idempotent argv command remotely via SSH ControlMaster with ADB-SSH auto-failover.
pub(crate) async fn ssh_exec_argv(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    argv: &[String],
    timeout_sec: f64,
) -> Value {
    let remote_cmd = argv_to_shell(argv);
    ssh_exec_raw_with_policy(
        registry,
        target,
        &remote_cmd,
        timeout_sec,
        RetryPolicy::Idempotent,
    )
    .await
}

/// Executes a side-effectful argv command without risking duplicate execution on timeout or mid-flight drop.
pub(crate) async fn ssh_exec_argv_once(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    argv: &[String],
    timeout_sec: f64,
) -> Value {
    let remote_cmd = argv_to_shell(argv);
    ssh_exec_raw_with_policy(
        registry,
        target,
        &remote_cmd,
        timeout_sec,
        RetryPolicy::SideEffect,
    )
    .await
}

struct SshRunner<'a> {
    remote_cmd: &'a str,
    timeout: Duration,
}

impl RouteRunner for SshRunner<'_> {
    async fn ensure_forward(&self, route: &TransportRoute) -> Result<(), String> {
        if let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port) {
            routing::ensure_adb_forward(ser, lp, route.remote_ssh_port).await
        } else {
            Ok(())
        }
    }

    async fn run_once(&self, route: &TransportRoute) -> SshAttempt {
        run_ssh_once(route, self.remote_cmd, self.timeout).await
    }

    async fn reset_control(&self, route: &TransportRoute) {
        routing::reset_control_socket(route).await;
    }
}

/// Executes a remote command string over SSH with candidate route failover and JSON extraction.
pub(crate) async fn ssh_exec_raw_with_policy(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    remote_cmd: &str,
    timeout_sec: f64,
    policy: RetryPolicy,
) -> Value {
    let routes = match build_routes(registry, target).await {
        Ok(r) => r,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let eff = Duration::from_secs_f64(timeout_sec.clamp(1.0, MAX_TIMEOUT_SEC));
    let runner = SshRunner {
        remote_cmd,
        timeout: eff,
    };
    execute_routes_with_policy(&routes, policy, &runner).await
}

async fn run_ssh_once(route: &TransportRoute, remote_cmd: &str, timeout: Duration) -> SshAttempt {
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
            let err = format!("spawn ssh failed: {e}");
            return SshAttempt {
                exit_code: 127,
                stdout_empty: true,
                has_json: false,
                stderr: err.clone(),
                payload: json!({"ok": false, "rc": 127, "route": route.kind, "error": err}),
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
                    obj.entry("route").or_insert_with(|| json!(route.kind));
                    Value::Object(obj)
                } else {
                    json!({"ok": rc == 0, "rc": rc, "route": route.kind, "data": parsed})
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
                payload: json!({"ok": rc == 0, "rc": rc, "route": route.kind, "stdout": stdout.trim(), "stderr": stderr}),
            }
        }
        Ok(Err(e)) => {
            let err = format!("ssh wait failed: {e}");
            SshAttempt {
                exit_code: 1,
                stdout_empty: true,
                has_json: false,
                stderr: err.clone(),
                payload: json!({"ok": false, "rc": 1, "route": route.kind, "error": err}),
            }
        }
        Err(_) => {
            let err = format!("ssh timed out after {}s", timeout.as_secs());
            SshAttempt {
                exit_code: 124,
                stdout_empty: true,
                has_json: false,
                stderr: err.clone(),
                payload: json!({"ok": false, "rc": 124, "route": route.kind, "error": err}),
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
    fn extracts_trailing_json_past_ssh_banner() {
        let raw = "Warning: Permanently added host\n{\"ok\":true,\"count\":3}\n";
        let v = extract_trailing_json(raw).expect("parsed");
        assert_eq!(v["count"], json!(3));
    }
}
