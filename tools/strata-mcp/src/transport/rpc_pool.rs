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
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

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
    /// Failed during spawn or pre-request handshake (remote command never ran; safe to failover).
    PreConnectFailed(String),
    /// Failed before the request could be dispatched on an existing session (safe to reconnect/failover).
    PreExec(String),
    /// Timed out or disconnected after request dispatch (must not duplicate side effects).
    InFlight(String),
}

impl RpcCallError {
    fn is_pre_exec(&self) -> bool {
        matches!(self, Self::PreConnectFailed(_) | Self::PreExec(_))
    }
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
            .map_err(|e| RpcCallError::PreConnectFailed(format!("spawn serve-rpc failed: {e}")))?;
        let Some(stdin) = child.stdin.take() else {
            return Err(RpcCallError::PreConnectFailed(
                "serve-rpc stdin unavailable".to_owned(),
            ));
        };
        let Some(stdout) = child.stdout.take() else {
            return Err(RpcCallError::PreConnectFailed(
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

    async fn spawn_for_route(route: &TransportRoute) -> Result<Self, RpcCallError> {
        let mut sess = if route.kind == "adb-host" {
            let Some(serial) = route.adb_serial.as_deref() else {
                return Err(RpcCallError::PreConnectFailed(
                    "adb-host route missing serial".to_owned(),
                ));
            };
            let args = vec![
                "-s".to_owned(),
                serial.to_owned(),
                "shell".to_owned(),
                format!("su -c '{HOST_DEBUG_BIN} serve-rpc'"),
            ];
            Self::spawn_command("adb", &args)?
        } else {
            let mut args = vec!["-T".to_owned()];
            args.extend(route.base_ssh_args());
            args.push("strata-debug serve-rpc".to_owned());
            Self::spawn_command("ssh", &args)?
        };
        sess.handshake_with_timeout(HANDSHAKE_TIMEOUT).await?;
        Ok(sess)
    }

    async fn write_rpc_line(&mut self, req: &Value) -> Result<(), String> {
        let mut line =
            serde_json::to_string(req).map_err(|e| format!("serialize rpc request: {e}"))?;
        line.push('\n');
        match tokio::time::timeout(Duration::from_secs(5), async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await
        })
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(format!("serve-rpc write failed: {e}")),
            Err(_) => Err("serve-rpc write timed out".to_owned()),
        }
    }

    async fn read_rpc_response(&mut self, id: u64) -> Result<Value, String> {
        let mut skipped = 0usize;
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self
                .stdout
                .read_line(&mut buf)
                .await
                .map_err(|e| format!("serve-rpc read failed: {e}"))?;
            if n == 0 {
                return Err("serve-rpc pipe closed (EOF)".to_owned());
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
                return Err(format!("serve-rpc error: {msg}"));
            }
            skipped += 1;
            if skipped > MAX_BANNER_LINES {
                return Err("serve-rpc produced unexpected non-JSON output".to_owned());
            }
        }
    }

    /// Verifies the SSH/ADB tunnel and remote `strata-debug serve-rpc` readiness before dispatching work.
    async fn handshake_with_timeout(&mut self, timeout: Duration) -> Result<(), RpcCallError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let req = json!({"jsonrpc": "2.0", "id": id, "method": "ping", "params": {}});
        match tokio::time::timeout(timeout, async {
            self.write_rpc_line(&req)
                .await
                .map_err(RpcCallError::PreConnectFailed)?;
            self.read_rpc_response(id)
                .await
                .map_err(RpcCallError::PreConnectFailed)
        })
        .await
        {
            Ok(res) => res.map(|_| ()),
            Err(_) => Err(RpcCallError::PreConnectFailed(format!(
                "serve-rpc handshake timed out after {}ms",
                timeout.as_millis()
            ))),
        }
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
        self.write_rpc_line(&req)
            .await
            .map_err(RpcCallError::PreExec)?;

        match tokio::time::timeout(timeout, self.read_rpc_response(id)).await {
            Ok(Ok(res)) => Ok(res),
            Ok(Err(err)) => Err(RpcCallError::InFlight(err)),
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
        *guard = Some(RpcSession::spawn_for_route(route).await?);
    }

    let Some(sess) = guard.as_mut() else {
        return Err(RpcCallError::PreConnectFailed(
            "serve-rpc slot empty".to_owned(),
        ));
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
            // If a reused pipe died silently (EOF/broken pipe before dispatch), reconnect once on PreExec.
            if reused && err.is_pre_exec() {
                let mut fresh = RpcSession::spawn_for_route(route).await?;
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
        sess.handshake_with_timeout(Duration::from_secs(2))
            .await
            .expect("handshake");

        let r1 = sess
            .call_cli(&["health".to_owned()], Duration::from_secs(5))
            .await
            .expect("call 1");
        let r2 = sess
            .call_cli(&["health".to_owned()], Duration::from_secs(5))
            .await
            .expect("call 2");

        assert_eq!(r1["ok"], json!(true));
        assert_eq!(r1["call"], json!(2));
        assert_eq!(r2["call"], json!(3));
        assert_eq!(r1["pid"], r2["pid"], "must reuse the same process");
    }

    #[tokio::test]
    async fn handshake_eof_and_timeout_report_pre_connect_failed() {
        let eof_args = vec!["-c".to_owned(), "exit 255".to_owned()];
        let mut eof_sess = RpcSession::spawn_command("sh", &eof_args).expect("spawn eof");
        let err = eof_sess
            .handshake_with_timeout(Duration::from_secs(2))
            .await
            .expect_err("handshake eof");
        assert!(matches!(err, RpcCallError::PreConnectFailed(_)));

        let slow_args = vec!["-c".to_owned(), "sleep 5".to_owned()];
        let mut slow_sess = RpcSession::spawn_command("sh", &slow_args).expect("spawn slow");
        let err2 = slow_sess
            .handshake_with_timeout(Duration::from_millis(120))
            .await
            .expect_err("handshake timeout");
        assert!(matches!(err2, RpcCallError::PreConnectFailed(_)));
    }

    #[tokio::test]
    async fn post_handshake_eof_and_timeout_report_in_flight_errors() {
        let drop_after_ping = r#"
IFS= read -r ping
id=$(printf '%s' "$ping" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
printf '{"jsonrpc":"2.0","id":%s,"result":{"pong":true}}\n' "$id"
IFS= read -r _req
exit 0
"#;
        let args = vec!["-c".to_owned(), drop_after_ping.to_owned()];
        let mut sess = RpcSession::spawn_command("sh", &args).expect("spawn");
        sess.handshake_with_timeout(Duration::from_secs(2))
            .await
            .expect("handshake ok");
        let err = sess
            .call_cli(&["health".to_owned()], Duration::from_secs(2))
            .await
            .expect_err("post-handshake eof");
        assert!(matches!(err, RpcCallError::InFlight(_)));

        let slow_after_ping = r#"
IFS= read -r ping
id=$(printf '%s' "$ping" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
printf '{"jsonrpc":"2.0","id":%s,"result":{"pong":true}}\n' "$id"
IFS= read -r _req
sleep 5
"#;
        let slow_args = vec!["-c".to_owned(), slow_after_ping.to_owned()];
        let mut slow = RpcSession::spawn_command("sh", &slow_args).expect("spawn slow");
        slow.handshake_with_timeout(Duration::from_secs(2))
            .await
            .expect("handshake ok");
        let err2 = slow
            .call_cli(&["health".to_owned()], Duration::from_millis(120))
            .await
            .expect_err("timeout");
        assert!(matches!(err2, RpcCallError::InFlight(_)));
    }
}
