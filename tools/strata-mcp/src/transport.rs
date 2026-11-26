//! Three-tier async transport (`ssh` -> `adb-ssh` -> `adb-host`), dynamic port discovery, and `serve-rpc` pool.

mod adb_forward;
mod adb_transfer;
pub(crate) mod container_probe;
mod exec;
mod policy;
pub(crate) mod routing;
pub(crate) mod rpc_pool;
mod transfer;

use std::time::Duration;

use serde_json::{Value, json};

use crate::devices::DeviceRegistry;
pub(crate) use adb_forward::{
    adb_host_shell_command, argv_to_shell, ensure_adb_forward, invalidate_adb_forward, shell_quote,
};
pub(crate) use container_probe::{
    build_container_required_error, invalidate_container_probe, probe_containers_via_adb,
};
pub(crate) use exec::extract_trailing_json;
use exec::{annotate_degraded, run_adb_host_once, run_ssh_once};
pub(crate) use policy::{RetryPolicy, RouteRequirement};
use policy::{RouteRunner, SshAttempt, execute_routes_outcome};
pub(crate) use routing::{TargetOverride, TransportRoute, build_routes};
pub(crate) use transfer::{open_ssh_port_forward, transfer_pull, transfer_push};

const MAX_TIMEOUT_SEC: f64 = 120.0;

/// Executes an idempotent argv command (auto-degrades to `adb-host` for host-capable tools).
pub(crate) async fn ssh_exec_argv(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    argv: &[String],
    timeout_sec: f64,
) -> Value {
    ssh_exec_argv_with_req(
        registry,
        target,
        argv,
        timeout_sec,
        RetryPolicy::Idempotent,
        RouteRequirement::HostOrContainer,
        "command",
    )
    .await
}

/// Executes a side-effectful argv command without risking duplicate execution on mid-flight drop.
pub(crate) async fn ssh_exec_argv_once(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    argv: &[String],
    timeout_sec: f64,
) -> Value {
    ssh_exec_argv_with_req(
        registry,
        target,
        argv,
        timeout_sec,
        RetryPolicy::SideEffect,
        RouteRequirement::HostOrContainer,
        "command",
    )
    .await
}

/// Executes an argv command with explicit retry policy and container/host route requirement.
pub(crate) async fn ssh_exec_argv_with_req(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    argv: &[String],
    timeout_sec: f64,
    policy: RetryPolicy,
    requirement: RouteRequirement,
    tool_label: &str,
) -> Value {
    let remote_cmd = argv_to_shell(argv);
    let host_cmd = adb_host_shell_command(argv);
    let rpc_sub_argv = if argv.first().map(String::as_str) == Some("strata-debug") && argv.len() > 1
    {
        Some(&argv[1..])
    } else {
        None
    };
    execute_three_tier(
        registry,
        target,
        &remote_cmd,
        &host_cmd,
        rpc_sub_argv,
        timeout_sec,
        policy,
        requirement,
        tool_label,
    )
    .await
}

/// Executes a raw shell command string over three-tier transport.
pub(crate) async fn ssh_exec_raw_with_policy(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    remote_cmd: &str,
    timeout_sec: f64,
    policy: RetryPolicy,
) -> Value {
    execute_three_tier(
        registry,
        target,
        remote_cmd,
        remote_cmd,
        None,
        timeout_sec,
        policy,
        RouteRequirement::HostOrContainer,
        "raw-shell",
    )
    .await
}

/// Invalidates container probe cache and persistent `serve-rpc` sessions for a target device.
pub(crate) async fn invalidate_device_caches(registry: &DeviceRegistry, target: &TargetOverride) {
    if let Ok(routes) = build_routes(registry, target).await {
        for r in &routes {
            rpc_pool::invalidate_rpc_session(&r.session_key()).await;
            if let Some(ser) = r.adb_serial.as_deref() {
                invalidate_container_probe(Some(ser)).await;
                rpc_pool::invalidate_rpc_sessions_matching(ser).await;
                if let Some(lp) = r.local_port {
                    invalidate_adb_forward(ser, lp);
                }
            }
        }
    } else {
        invalidate_container_probe(None).await;
    }
}

struct TransportRunner<'a> {
    remote_cmd: &'a str,
    host_cmd: &'a str,
    rpc_sub_argv: Option<&'a [String]>,
    policy: RetryPolicy,
    timeout: Duration,
}

impl RouteRunner for TransportRunner<'_> {
    async fn ensure_forward(&self, route: &TransportRoute) -> Result<(), String> {
        if let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port) {
            ensure_adb_forward(ser, lp, route.remote_ssh_port).await
        } else {
            Ok(())
        }
    }

    async fn run_once(&self, route: &TransportRoute) -> SshAttempt {
        if let Some(sub_argv) = self.rpc_sub_argv {
            match rpc_pool::call_serve_rpc(route, sub_argv, self.timeout).await {
                Ok(payload) => {
                    return SshAttempt {
                        exit_code: 0,
                        stdout_empty: false,
                        has_json: true,
                        stderr: String::new(),
                        payload,
                    };
                }
                Err(rpc_pool::RpcCallError::InFlight(err))
                    if self.policy == RetryPolicy::SideEffect =>
                {
                    return SshAttempt {
                        exit_code: 124,
                        stdout_empty: true,
                        has_json: false,
                        stderr: err.clone(),
                        payload: json!({"ok": false, "rc": 124, "route": route.kind, "error": err}),
                    };
                }
                Err(_) => {}
            }
        }
        if route.kind == "adb-host" {
            run_adb_host_once(route, self.host_cmd, self.timeout).await
        } else {
            run_ssh_once(route, self.remote_cmd, self.timeout).await
        }
    }

    async fn reset_control(&self, route: &TransportRoute) {
        rpc_pool::invalidate_rpc_session(&route.session_key()).await;
        if route.kind == "adb-ssh"
            && let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port)
        {
            invalidate_adb_forward(ser, lp);
        }
        routing::reset_control_socket(route).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_three_tier(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    remote_cmd: &str,
    host_cmd: &str,
    rpc_sub_argv: Option<&[String]>,
    timeout_sec: f64,
    policy: RetryPolicy,
    requirement: RouteRequirement,
    tool_label: &str,
) -> Value {
    let routes = match build_routes(registry, target).await {
        Ok(r) => r,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let eff = Duration::from_secs_f64(timeout_sec.clamp(1.0, MAX_TIMEOUT_SEC));
    let runner = TransportRunner {
        remote_cmd,
        host_cmd,
        rpc_sub_argv,
        policy,
        timeout: eff,
    };

    let ssh_routes: Vec<TransportRoute> = routes
        .iter()
        .filter(|r| r.kind != "adb-host")
        .cloned()
        .collect();
    let adb_host = routes.iter().find(|r| r.kind == "adb-host").cloned();
    let adb_ssh = routes.iter().find(|r| r.kind == "adb-ssh").cloned();

    let mut last_payload = json!({"ok": false, "rc": 255, "error": "no transport route available"});
    if !ssh_routes.is_empty() {
        let outcome = execute_routes_outcome(&ssh_routes, policy, &runner).await;
        if !outcome.exhausted_transport {
            return outcome.payload;
        }
        last_payload = outcome.payload;
    }

    let Some(host_route) = adb_host else {
        return last_payload;
    };
    let Some(serial) = host_route.adb_serial.as_deref() else {
        return last_payload;
    };

    let probe = probe_containers_via_adb(serial, false).await;
    if let Some(real_port) = probe.active_ssh_port
        && let Some(base_adb_ssh) = adb_ssh
        && base_adb_ssh.remote_ssh_port != real_port
    {
        if let Some(lp) = base_adb_ssh.local_port {
            invalidate_adb_forward(serial, lp);
        }
        let refreshed = base_adb_ssh.with_remote_port(real_port);
        let retry = execute_routes_outcome(&[refreshed], policy, &runner).await;
        if !retry.exhausted_transport {
            return retry.payload;
        }
    }

    if requirement == RouteRequirement::ContainerOnly {
        return build_container_required_error(tool_label, &probe);
    }

    let reason = probe
        .degraded_reason
        .unwrap_or_else(|| "SSH/container transport unavailable; degraded to adb-host".to_owned());
    let attempt = runner.run_once(&host_route).await;
    annotate_degraded(attempt.payload, &reason)
}
