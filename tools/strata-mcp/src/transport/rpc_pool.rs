//! Persistent `strata-debug serve-rpc` session pool with per-route serialization and auto-reconnect.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use super::adb_forward::HOST_DEBUG_BIN;
use super::routing::TransportRoute;

const MAX_BANNER_LINES: usize = 24;

#[derive(Debug)]
struct RpcSession {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

/// Failure classification for a persistent `serve-rpc` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RpcCallError {
    /// Failed before the request could be dispatched (safe to retry/failover).
    PreExec(String),
    /// Timed out or disconnected after request dispatch (must not duplicate side effects).
    InFlight(String),
}

impl RpcSession {
    fn spawn_command(program: &str, args: &[String]) -> Result<Self, RpcCallError> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| RpcCallError::PreExec(format!("spawn serve-rpc failed: {e}")))?;
        let Some(stdin) = child.stdin.take() else {
            return Err(RpcCallError::PreExec(
                "serve-rpc stdin unavailable".to_owned(),
            ));
        };
        let Some(stdout) = child.stdout.take() else {
            return Err(RpcCallError::PreExec(
                "serve-rpc stdout unavailable".to_owned(),
            ));
        };
        Ok(Self {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    fn spawn_for_route(route: &TransportRoute) -> Result<Self, RpcCallError> {
        if route.kind == "adb-host" {
            let Some(serial) = route.adb_serial.as_deref() else {
                return Err(RpcCallError::PreExec(
                    "adb-host route missing serial".to_owned(),
                ));
            };
            let args = vec![
                "-s".to_owned(),
                serial.to_owned(),
                "shell".to_owned(),
                format!("su -c '{HOST_DEBUG_BIN} serve-rpc'"),
            ];
            return Self::spawn_command("adb", &args);
        }
        let mut args = vec!["-T".to_owned()];
        args.extend(route.base_ssh_args());
        args.push("strata-debug serve-rpc".to_owned());
        Self::spawn_command("ssh", &args)
    }

    async fn call_cli(
        &mut self,
        sub_argv: &[String],
        timeout: Duration,
    ) -> Result<Value, RpcCallError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);

        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "cli",
            "params": {"argv": sub_argv},
        });
        let mut line = serde_json::to_string(&req)
            .map_err(|e| RpcCallError::PreExec(format!("serialize rpc request: {e}")))?;
        line.push('\n');

        let write_res = tokio::time::timeout(Duration::from_secs(5), async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await
        })
        .await;
        match write_res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(RpcCallError::PreExec(format!(
                    "serve-rpc write failed: {e}"
                )));
            }
            Err(_) => {
                return Err(RpcCallError::PreExec(
                    "serve-rpc write timed out".to_owned(),
                ));
            }
        }

        let read_fut = async {
            let mut skipped = 0usize;
            let mut buf = String::new();
            loop {
                buf.clear();
                let n =
                    self.stdout.read_line(&mut buf).await.map_err(|e| {
                        RpcCallError::InFlight(format!("serve-rpc read failed: {e}"))
                    })?;
                if n == 0 {
                    return Err(RpcCallError::InFlight(
                        "serve-rpc pipe closed (EOF)".to_owned(),
                    ));
                }
                let trimmed = buf.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(resp) = serde_json::from_str::<Value>(trimmed)
                    && resp.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
                    && resp.get("id").and_then(Value::as_u64) == Some(id)
                {
                    if let Some(res) = resp.get("result") {
                        return Ok(res.clone());
                    }
                    let msg = resp
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown serve-rpc error");
                    return Err(RpcCallError::InFlight(format!("serve-rpc error: {msg}")));
                }
                skipped += 1;
                if skipped > MAX_BANNER_LINES {
                    return Err(RpcCallError::InFlight(
                        "serve-rpc produced unexpected non-JSON output".to_owned(),
                    ));
                }
            }
        };

        match tokio::time::timeout(timeout, read_fut).await {
            Ok(res) => res,
            Err(_) => Err(RpcCallError::InFlight(format!(
                "serve-rpc timed out after {}s",
                timeout.as_secs()
            ))),
        }
    }
}

type SessionSlot = Arc<Mutex<Option<RpcSession>>>;

fn session_pool() -> &'static Mutex<HashMap<String, SessionSlot>> {
    static POOL: OnceLock<Mutex<HashMap<String, SessionSlot>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn get_slot(key: &str) -> SessionSlot {
    let mut map = session_pool().lock().await;
    Arc::clone(
        map.entry(key.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(None))),
    )
}

/// Drops any cached `serve-rpc` session matching `key`.
pub(crate) async fn invalidate_rpc_session(key: &str) {
    let slot = {
        let mut map = session_pool().lock().await;
        map.remove(key)
    };
    if let Some(s) = slot {
        *s.lock().await = None;
    }
}

/// Drops all cached `serve-rpc` sessions whose key contains `needle`.
pub(crate) async fn invalidate_rpc_sessions_matching(needle: &str) {
    let slots: Vec<SessionSlot> = {
        let mut map = session_pool().lock().await;
        let keys: Vec<String> = map.keys().filter(|k| k.contains(needle)).cloned().collect();
        keys.into_iter().filter_map(|k| map.remove(&k)).collect()
    };
    for s in slots {
        *s.lock().await = None;
    }
}

/// Executes a `strata-debug` subcommand via the persistent `serve-rpc` session for `route`.
pub(crate) async fn call_serve_rpc(
    route: &TransportRoute,
    sub_argv: &[String],
    timeout: Duration,
) -> Result<Value, RpcCallError> {
    let key = route.session_key();
    let slot = get_slot(&key).await;
    let mut guard = slot.lock().await;

    let reused = guard.is_some();
    if guard.is_none() {
        *guard = Some(RpcSession::spawn_for_route(route)?);
    }

    let Some(sess) = guard.as_mut() else {
        return Err(RpcCallError::PreExec("serve-rpc slot empty".to_owned()));
    };

    match sess.call_cli(sub_argv, timeout).await {
        Ok(mut val) => {
            if let Value::Object(ref mut obj) = val {
                obj.entry("route").or_insert_with(|| json!(route.kind));
            }
            Ok(val)
        }
        Err(err) => {
            *guard = None;
            // If a reused pipe died silently (EOF/broken pipe before response), reconnect once on PreExec.
            if reused && matches!(err, RpcCallError::PreExec(_)) {
                let mut fresh = RpcSession::spawn_for_route(route)?;
                let res = fresh.call_cli(sub_argv, timeout).await;
                if let Ok(mut val) = res {
                    if let Value::Object(ref mut obj) = val {
                        obj.entry("route").or_insert_with(|| json!(route.kind));
                    }
                    *guard = Some(fresh);
                    return Ok(val);
                }
                return res;
            }
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn persistent_rpc_session_reuses_single_process_across_calls() {
        let script = r#"
count=0
while IFS= read -r line; do
  count=$((count + 1))
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  printf '{"jsonrpc":"2.0","id":%s,"result":{"ok":true,"pid":%s,"call":%s}}\n' "$id" "$$" "$count"
done
"#;
        let args = vec!["-c".to_owned(), script.to_owned()];
        let mut sess = RpcSession::spawn_command("sh", &args).expect("spawn mock rpc");

        let r1 = sess
            .call_cli(&["health".to_owned()], Duration::from_secs(5))
            .await
            .expect("call 1");
        let r2 = sess
            .call_cli(&["health".to_owned()], Duration::from_secs(5))
            .await
            .expect("call 2");

        assert_eq!(r1["ok"], json!(true));
        assert_eq!(r1["call"], json!(1));
        assert_eq!(r2["call"], json!(2));
        assert_eq!(r1["pid"], r2["pid"], "must reuse the same process");
    }

    #[tokio::test]
    async fn rpc_session_reports_eof_and_timeout_as_in_flight_errors() {
        let args = vec!["-c".to_owned(), "exit 0".to_owned()];
        let mut sess = RpcSession::spawn_command("sh", &args).expect("spawn");
        let err = sess
            .call_cli(&["health".to_owned()], Duration::from_secs(2))
            .await
            .expect_err("eof");
        assert!(matches!(
            err,
            RpcCallError::PreExec(_) | RpcCallError::InFlight(_)
        ));

        let slow_args = vec!["-c".to_owned(), "read -r _; sleep 5".to_owned()];
        let mut slow = RpcSession::spawn_command("sh", &slow_args).expect("spawn slow");
        let err2 = slow
            .call_cli(&["health".to_owned()], Duration::from_millis(120))
            .await
            .expect_err("timeout");
        assert!(matches!(err2, RpcCallError::InFlight(_)));
    }
}
