use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::container_probe::{ContainerProbe, ContainerSummary};
use super::execute_three_tier_on_routes;
use super::policy::{RetryPolicy, RouteRequirement, RouteRunner, SshAttempt};
use super::routing::TransportRoute;

fn sample_routes() -> Vec<TransportRoute> {
    vec![
        TransportRoute {
            kind: "ssh",
            host: "192.0.2.10".to_owned(),
            port: Some(2222),
            user: Some("root".to_owned()),
            control_path: PathBuf::from("/tmp/test-ssh-ctrl"),
            known_hosts_path: PathBuf::from("/tmp/test-kh"),
            adb_serial: None,
            local_port: None,
            remote_ssh_port: 2222,
        },
        TransportRoute {
            kind: "adb-ssh",
            host: "127.0.0.1".to_owned(),
            port: Some(12222),
            user: Some("root".to_owned()),
            control_path: PathBuf::from("/tmp/test-adb-ctrl"),
            known_hosts_path: PathBuf::from("/tmp/test-kh"),
            adb_serial: Some("SYNTH_SER_1".to_owned()),
            local_port: Some(12222),
            remote_ssh_port: 2222,
        },
        TransportRoute {
            kind: "adb-host",
            host: "adb-host".to_owned(),
            port: None,
            user: None,
            control_path: PathBuf::new(),
            known_hosts_path: PathBuf::from("/tmp/test-kh"),
            adb_serial: Some("SYNTH_SER_1".to_owned()),
            local_port: None,
            remote_ssh_port: 2222,
        },
    ]
}

fn stopped_probe() -> ContainerProbe {
    ContainerProbe {
        daemon_online: true,
        containers: vec![ContainerSummary {
            id: "main".to_owned(),
            is_default: true,
            phase: "stopped".to_owned(),
            ssh_enabled: true,
            ssh_port: 2222,
        }],
        active_ssh_port: None,
        degraded_reason: Some("all Strata containers are stopped".to_owned()),
    }
}

enum SimFailure {
    PreConnectEof,
    InFlightEof,
}

struct MockThreeTierRunner {
    failure: SimFailure,
    cached: Option<ContainerProbe>,
    probe: ContainerProbe,
    ssh_calls: AtomicUsize,
    host_calls: AtomicUsize,
}

impl MockThreeTierRunner {
    fn new(failure: SimFailure, cached: Option<ContainerProbe>, probe: ContainerProbe) -> Self {
        Self {
            failure,
            cached,
            probe,
            ssh_calls: AtomicUsize::new(0),
            host_calls: AtomicUsize::new(0),
        }
    }
}

impl RouteRunner for MockThreeTierRunner {
    async fn ensure_forward(&self, _route: &TransportRoute) -> Result<(), String> {
        Ok(())
    }

    async fn run_once(&self, route: &TransportRoute) -> SshAttempt {
        if route.kind == "adb-host" {
            self.host_calls.fetch_add(1, Ordering::SeqCst);
            return SshAttempt {
                exit_code: 0,
                stdout_empty: false,
                has_json: true,
                pre_exec_failed: false,
                stderr: String::new(),
                payload: json!({"ok": true, "rc": 0, "route": "adb-host"}),
            };
        }
        self.ssh_calls.fetch_add(1, Ordering::SeqCst);
        match self.failure {
            SimFailure::PreConnectEof => SshAttempt {
                exit_code: 255,
                stdout_empty: true,
                has_json: false,
                pre_exec_failed: true,
                stderr: "serve-rpc pipe closed (EOF)".to_owned(),
                payload: json!({"ok": false, "rc": 255, "route": route.kind, "error": "serve-rpc pipe closed (EOF)"}),
            },
            SimFailure::InFlightEof => SshAttempt {
                exit_code: 124,
                stdout_empty: true,
                has_json: false,
                pre_exec_failed: false,
                stderr: "serve-rpc pipe closed (EOF)".to_owned(),
                payload: json!({"ok": false, "rc": 124, "route": route.kind, "error": "serve-rpc pipe closed (EOF)"}),
            },
        }
    }

    async fn reset_control(&self, _route: &TransportRoute) {}

    async fn cached_probe(&self, _serial: &str) -> Option<ContainerProbe> {
        self.cached.clone()
    }

    async fn probe_containers(&self, _serial: &str) -> ContainerProbe {
        self.probe.clone()
    }
}

#[tokio::test]
async fn pre_connect_eof_allows_side_effect_failover_to_adb_host_and_container_diagnostics() {
    let routes = sample_routes();

    let host_runner = MockThreeTierRunner::new(SimFailure::PreConnectEof, None, stopped_probe());
    let host_res: Value = execute_three_tier_on_routes(
        &routes,
        RetryPolicy::SideEffect,
        RouteRequirement::HostOrContainer,
        "device_exec",
        &host_runner,
    )
    .await;
    assert_eq!(host_res["ok"], json!(true));
    assert_eq!(host_res["route"], json!("adb-host"));
    assert_eq!(
        host_res["degradedReason"],
        json!("all Strata containers are stopped")
    );
    assert_eq!(host_runner.host_calls.load(Ordering::SeqCst), 1);

    let container_runner =
        MockThreeTierRunner::new(SimFailure::PreConnectEof, None, stopped_probe());
    let container_res: Value = execute_three_tier_on_routes(
        &routes,
        RetryPolicy::SideEffect,
        RouteRequirement::ContainerOnly,
        "device_exec(scope='container')",
        &container_runner,
    )
    .await;
    assert_eq!(container_res["ok"], json!(false));
    assert_eq!(container_res["daemonOnline"], json!(true));
    assert_eq!(
        container_res["degradedReason"],
        json!("all Strata containers are stopped")
    );
    assert_eq!(container_res["containers"][0]["id"], json!("main"));
    assert_eq!(container_runner.host_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn in_flight_eof_stops_side_effect_without_retry() {
    let routes = sample_routes();
    let runner = MockThreeTierRunner::new(SimFailure::InFlightEof, None, stopped_probe());
    let res = execute_three_tier_on_routes(
        &routes,
        RetryPolicy::SideEffect,
        RouteRequirement::HostOrContainer,
        "observe_step",
        &runner,
    )
    .await;
    assert_eq!(res["ok"], json!(false));
    assert_eq!(res["rc"], json!(124));
    assert_eq!(res["route"], json!("ssh"));
    assert_eq!(runner.ssh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(runner.host_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cached_zero_container_probe_short_circuits_ssh_for_host_and_container_only() {
    let routes = sample_routes();
    let cached = Some(stopped_probe());

    let host_runner =
        MockThreeTierRunner::new(SimFailure::PreConnectEof, cached.clone(), stopped_probe());
    let host_res = execute_three_tier_on_routes(
        &routes,
        RetryPolicy::SideEffect,
        RouteRequirement::HostOrContainer,
        "device_exec",
        &host_runner,
    )
    .await;
    assert_eq!(host_res["ok"], json!(true));
    assert_eq!(host_res["route"], json!("adb-host"));
    assert_eq!(host_runner.ssh_calls.load(Ordering::SeqCst), 0);
    assert_eq!(host_runner.host_calls.load(Ordering::SeqCst), 1);

    let container_runner =
        MockThreeTierRunner::new(SimFailure::PreConnectEof, cached, stopped_probe());
    let container_res = execute_three_tier_on_routes(
        &routes,
        RetryPolicy::SideEffect,
        RouteRequirement::ContainerOnly,
        "device_exec(scope='container')",
        &container_runner,
    )
    .await;
    assert_eq!(container_res["ok"], json!(false));
    assert_eq!(container_res["daemonOnline"], json!(true));
    assert_eq!(container_runner.ssh_calls.load(Ordering::SeqCst), 0);
    assert_eq!(container_runner.host_calls.load(Ordering::SeqCst), 0);
}
