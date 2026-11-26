//! Retry and failover policy separating idempotent queries from side-effectful device actions.

use serde_json::{Value, json};

use super::routing::TransportRoute;

/// Failover/retry policy separating read-only queries from side-effectful device actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryPolicy {
    /// Read-only / idempotent commands: failover on transport error (rc 255) or timeout (rc 124)
    /// when no structured JSON output was received.
    Idempotent,
    /// Side-effectful commands (gesture, tap, exec, app mutations, observe actions):
    /// failover ONLY on guaranteed pre-execution SSH connection errors (rc 255, empty stdout,
    /// known connect/auth/kex failure without mid-flight drop markers).
    SideEffect,
}

#[derive(Debug, Clone)]
pub(super) struct SshAttempt {
    pub(super) exit_code: i32,
    pub(super) stdout_empty: bool,
    pub(super) has_json: bool,
    pub(super) stderr: String,
    pub(super) payload: Value,
}

pub(super) fn is_pre_exec_ssh_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    if [
        "broken pipe",
        "read from master failed",
        "shared connection to",
        "killed by signal",
    ]
    .iter()
    .any(|ambig| lower.contains(ambig))
    {
        return false;
    }
    [
        "connection refused",
        "connection timed out",
        "operation timed out",
        "no route to host",
        "network is unreachable",
        "could not resolve",
        "temporary failure in name resolution",
        "name or service not known",
        "permission denied",
        "host key verification failed",
        "kex_exchange_identification",
        "banner exchange",
        "mux_client_hello_exchange",
        "mux_client_request_alive",
        "mux_client_request_session: session open refused",
        "control socket connect",
        "controlsocket",
        "unix_listener",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(super) fn should_failover(attempt: &SshAttempt, policy: RetryPolicy) -> bool {
    if attempt.has_json || attempt.exit_code == 0 {
        return false;
    }
    match attempt.exit_code {
        255 => match policy {
            RetryPolicy::Idempotent => true,
            RetryPolicy::SideEffect => {
                attempt.stdout_empty && is_pre_exec_ssh_error(&attempt.stderr)
            }
        },
        124 => matches!(policy, RetryPolicy::Idempotent),
        _ => false,
    }
}

fn should_reset_control_socket(attempt: &SshAttempt) -> bool {
    !attempt.has_json && matches!(attempt.exit_code, 124 | 255)
}

pub(super) trait RouteRunner {
    async fn ensure_forward(&self, route: &TransportRoute) -> Result<(), String>;
    async fn run_once(&self, route: &TransportRoute) -> SshAttempt;
    async fn reset_control(&self, route: &TransportRoute);
}

pub(super) async fn execute_routes_with_policy<R: RouteRunner>(
    routes: &[TransportRoute],
    policy: RetryPolicy,
    runner: &R,
) -> Value {
    let mut last = json!({"ok": false, "rc": 255, "error": "no transport route available"});

    for route in routes {
        if route.kind == "adb-ssh"
            && let Err(err) = runner.ensure_forward(route).await
        {
            last = json!({"ok": false, "rc": 255, "route": route.kind, "error": err});
            continue;
        }

        let mut attempt = runner.run_once(route).await;
        if route.kind == "adb-ssh" && attempt.exit_code == 255 && should_failover(&attempt, policy)
        {
            runner.reset_control(route).await;
            if runner.ensure_forward(route).await.is_ok() {
                attempt = runner.run_once(route).await;
            }
        }
        let failover = should_failover(&attempt, policy);
        let reset_needed = should_reset_control_socket(&attempt);
        last = attempt.payload;
        if !failover {
            if reset_needed {
                runner.reset_control(route).await;
            }
            return last;
        }
        runner.reset_control(route).await;
    }
    last
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn test_route(kind: &'static str) -> TransportRoute {
        TransportRoute {
            kind,
            host: if kind == "adb-ssh" {
                "127.0.0.1".to_owned()
            } else {
                "192.0.2.10".to_owned()
            },
            port: Some(if kind == "adb-ssh" { 12222 } else { 2222 }),
            user: Some("root".to_owned()),
            control_path: PathBuf::from(format!("/tmp/test-ctrl-{kind}")),
            known_hosts_path: PathBuf::from("/tmp/test-kh"),
            adb_serial: (kind == "adb-ssh").then(|| "SER123".to_owned()),
            local_port: (kind == "adb-ssh").then_some(12222),
            remote_ssh_port: 2222,
        }
    }

    #[test]
    fn failover_never_retries_successful_json_or_ambiguous_side_effects() {
        let ok_json = SshAttempt {
            exit_code: 0,
            stdout_empty: false,
            has_json: true,
            stderr: String::new(),
            payload: json!({"ok": true, "gesture": "swipe", "route": "ssh"}),
        };
        assert!(!should_failover(&ok_json, RetryPolicy::Idempotent));
        assert!(!should_failover(&ok_json, RetryPolicy::SideEffect));

        let timeout = SshAttempt {
            exit_code: 124,
            stdout_empty: true,
            has_json: false,
            stderr: "ssh timed out after 25s".to_owned(),
            payload: json!({"ok": false, "rc": 124}),
        };
        assert!(should_failover(&timeout, RetryPolicy::Idempotent));
        assert!(!should_failover(&timeout, RetryPolicy::SideEffect));

        let connect_refused = SshAttempt {
            exit_code: 255,
            stdout_empty: true,
            has_json: false,
            stderr: "ssh: connect to host 192.0.2.10 port 2222: Connection refused".to_owned(),
            payload: json!({"ok": false, "rc": 255}),
        };
        assert!(should_failover(&connect_refused, RetryPolicy::Idempotent));
        assert!(should_failover(&connect_refused, RetryPolicy::SideEffect));

        for ambiguous_err in [
            "client_loop: send disconnect: Broken pipe",
            "mux_client_request_session: read from master failed: Broken pipe",
            "Shared connection to 192.0.2.10 closed.",
            "host-exec: /tmp/missing: No such file or directory",
        ] {
            let attempt = SshAttempt {
                exit_code: 255,
                stdout_empty: true,
                has_json: false,
                stderr: ambiguous_err.to_owned(),
                payload: json!({"ok": false, "rc": 255}),
            };
            assert!(should_failover(&attempt, RetryPolicy::Idempotent));
            assert!(
                !should_failover(&attempt, RetryPolicy::SideEffect),
                "must not retry side effect on: {ambiguous_err}"
            );
        }
    }

    enum MockMode {
        JsonWithoutRc,
        Timeout,
        PreExecRefusedOnSsh,
    }

    struct MockRunner {
        mode: MockMode,
        calls: AtomicUsize,
        resets: AtomicUsize,
    }

    impl MockRunner {
        fn new(mode: MockMode) -> Self {
            Self {
                mode,
                calls: AtomicUsize::new(0),
                resets: AtomicUsize::new(0),
            }
        }
    }

    impl RouteRunner for MockRunner {
        async fn ensure_forward(&self, _route: &TransportRoute) -> Result<(), String> {
            Ok(())
        }

        async fn run_once(&self, route: &TransportRoute) -> SshAttempt {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            match self.mode {
                MockMode::JsonWithoutRc => {
                    assert_eq!(n, 0);
                    SshAttempt {
                        exit_code: 0,
                        stdout_empty: false,
                        has_json: true,
                        stderr: String::new(),
                        payload: json!({"ok": true, "gesture": "swipe", "route": route.kind}),
                    }
                }
                MockMode::Timeout => SshAttempt {
                    exit_code: 124,
                    stdout_empty: true,
                    has_json: false,
                    stderr: "ssh timed out".to_owned(),
                    payload: json!({"ok": false, "rc": 124, "route": route.kind}),
                },
                MockMode::PreExecRefusedOnSsh if route.kind == "ssh" => SshAttempt {
                    exit_code: 255,
                    stdout_empty: true,
                    has_json: false,
                    stderr: "ssh: connect to host 192.0.2.10 port 2222: Connection refused"
                        .to_owned(),
                    payload: json!({"ok": false, "rc": 255, "route": route.kind}),
                },
                MockMode::PreExecRefusedOnSsh => SshAttempt {
                    exit_code: 0,
                    stdout_empty: false,
                    has_json: true,
                    stderr: String::new(),
                    payload: json!({"ok": true, "gesture": "swipe", "route": route.kind}),
                },
            }
        }

        async fn reset_control(&self, _route: &TransportRoute) {
            self.resets.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn side_effect_route_loop_stops_on_json_without_rc_and_never_double_fires() {
        let routes = [test_route("ssh"), test_route("adb-ssh")];

        // 1. JSON response without `rc` (like `strata-debug gesture`) must NOT failover to adb-ssh.
        let r1 = MockRunner::new(MockMode::JsonWithoutRc);
        let res = execute_routes_with_policy(&routes, RetryPolicy::SideEffect, &r1).await;
        assert_eq!(r1.calls.load(Ordering::SeqCst), 1);
        assert_eq!(res["route"], json!("ssh"));

        // 2. Timeout (rc 124) on side-effect must stop immediately without adb-ssh retry.
        let r2 = MockRunner::new(MockMode::Timeout);
        let res_to = execute_routes_with_policy(&routes, RetryPolicy::SideEffect, &r2).await;
        assert_eq!(r2.calls.load(Ordering::SeqCst), 1);
        assert_eq!(r2.resets.load(Ordering::SeqCst), 1);
        assert_eq!(res_to["route"], json!("ssh"));

        // 3. Pre-exec Connection refused on ssh DOES failover once to adb-ssh for SideEffect.
        let r3 = MockRunner::new(MockMode::PreExecRefusedOnSsh);
        let res_fo = execute_routes_with_policy(&routes, RetryPolicy::SideEffect, &r3).await;
        assert_eq!(r3.calls.load(Ordering::SeqCst), 2);
        assert_eq!(res_fo["route"], json!("adb-ssh"));
    }
}
