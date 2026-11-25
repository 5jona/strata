//! Linux abstract Unix socket HTTP/1.1 client and `tokio-tungstenite` CDP WebSocket client.

use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixStream as StdUnixStream};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{WebSocketStream, client_async};

use super::http::read_http_response;

/// Connects asynchronously to a Linux abstract Unix domain socket (`\0<name>`).
pub(crate) async fn connect_abstract(
    socket_name: &str,
    timeout: Duration,
) -> Result<UnixStream, String> {
    let name = socket_name.to_owned();
    let connect_fut = tokio::task::spawn_blocking(move || -> Result<StdUnixStream, String> {
        let addr = SocketAddr::from_abstract_name(name.as_bytes())
            .map_err(|e| format!("invalid abstract socket '{name}': {e}"))?;
        let std_stream = StdUnixStream::connect_addr(&addr)
            .map_err(|e| format!("connect @{name} failed: {e}"))?;
        std_stream
            .set_nonblocking(true)
            .map_err(|e| format!("set_nonblocking failed: {e}"))?;
        Ok(std_stream)
    });

    let std_stream = tokio::time::timeout(timeout, connect_fut)
        .await
        .map_err(|_| format!("connect @{socket_name} timed out"))?
        .map_err(|e| format!("connect task failed: {e}"))??;

    UnixStream::from_std(std_stream).map_err(|e| format!("tokio UnixStream wrap failed: {e}"))
}

/// Sends an HTTP/1.1 GET request over an abstract UDS and returns raw `(status_code, body)`.
/// Stops reading immediately when `Content-Length` or chunked EOF arrives because Chromium
/// DevTools ignores `Connection: close` and keeps the socket open.
pub(crate) async fn http_get_raw(
    socket_name: &str,
    path: &str,
    timeout: Duration,
) -> Result<(u16, Vec<u8>), String> {
    let mut stream = connect_abstract(socket_name, timeout).await?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    tokio::time::timeout(timeout, async {
        stream
            .write_all(req.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        read_http_response(&mut stream).await
    })
    .await
    .map_err(|_| format!("HTTP GET {path} timed out"))?
}

/// Sends an HTTP/1.1 GET over an abstract UDS and parses the JSON body.
pub(crate) async fn http_get_json(
    socket_name: &str,
    path: &str,
    timeout: Duration,
) -> Result<Value, String> {
    let (status, body) = http_get_raw(socket_name, path, timeout).await?;
    if status != 200 {
        return Err(format!("HTTP {status} for {path}"));
    }
    serde_json::from_slice(&body).map_err(|e| format!("invalid JSON from {path}: {e}"))
}

fn format_console_arg(arg: &Value) -> String {
    if let Some(val) = arg.get("value") {
        return match val {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
    }
    arg.get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Async RFC 6455 CDP WebSocket client running over an abstract Unix domain socket.
pub(crate) struct CdpWsClient {
    ws: WebSocketStream<UnixStream>,
    next_id: u64,
    pub(crate) console_events: Vec<Value>,
    pub(crate) exceptions: Vec<Value>,
}

impl CdpWsClient {
    /// Connects and completes the WebSocket upgrade against `@<socket_name>` at `ws_path`.
    pub(crate) async fn connect_abstract(
        socket_name: &str,
        ws_path: &str,
        timeout: Duration,
    ) -> Result<Self, String> {
        let stream = connect_abstract(socket_name, timeout).await?;
        let uri = format!("ws://localhost{ws_path}");
        let (ws, _) = tokio::time::timeout(timeout, client_async(&uri, stream))
            .await
            .map_err(|_| "CDP WebSocket handshake timed out".to_owned())?
            .map_err(|e| format!("CDP WebSocket handshake failed: {e}"))?;
        Ok(Self {
            ws,
            next_id: 1,
            console_events: Vec::new(),
            exceptions: Vec::new(),
        })
    }

    fn record_event(&mut self, msg: &Value) {
        match msg.get("method").and_then(Value::as_str) {
            Some("Runtime.consoleAPICalled") => {
                let p = &msg["params"];
                let args: Vec<String> = p["args"]
                    .as_array()
                    .map(|arr| arr.iter().map(format_console_arg).collect())
                    .unwrap_or_default();
                self.console_events.push(json!({
                    "type": p["type"],
                    "text": args.join(" "),
                }));
            }
            Some("Runtime.exceptionThrown") => {
                let det = &msg["params"]["exceptionDetails"];
                self.exceptions.push(json!({
                    "text": det["text"],
                    "line": det["lineNumber"],
                    "column": det["columnNumber"],
                    "description": det["exception"]["description"],
                }));
            }
            _ => {}
        }
    }

    /// Sends one CDP JSON-RPC command and awaits its matching `id` response.
    pub(crate) async fn call(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({"id": id, "method": method, "params": params}).to_string();
        self.ws
            .send(Message::Text(payload.into()))
            .await
            .map_err(|e| format!("ws send failed: {e}"))?;

        let deadline = Instant::now() + timeout;
        while let Some(rem) = deadline.checked_duration_since(Instant::now()) {
            let next = tokio::time::timeout(rem, self.ws.next()).await;
            match next {
                Ok(Some(Ok(Message::Text(txt)))) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                        if val.get("id").and_then(Value::as_u64) == Some(id) {
                            return Ok(val);
                        }
                        self.record_event(&val);
                    }
                }
                Ok(Some(Ok(Message::Ping(p)))) => {
                    let _ = self.ws.send(Message::Pong(p)).await;
                }
                Ok(Some(Ok(Message::Close(_))) | None) => {
                    return Err("CDP WebSocket closed before response".to_owned());
                }
                Ok(Some(Err(e))) => return Err(format!("ws recv error: {e}")),
                Ok(Some(Ok(_))) => {}
                Err(_) => return Err(format!("CDP call '{method}' timed out")),
            }
        }
        Err(format!("CDP call '{method}' timed out"))
    }

    /// Collects incoming CDP events for `duration`.
    pub(crate) async fn drain_events(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while let Some(rem) = deadline.checked_duration_since(Instant::now()) {
            match tokio::time::timeout(rem, self.ws.next()).await {
                Ok(Some(Ok(Message::Text(txt)))) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                        self.record_event(&val);
                    }
                }
                Ok(Some(Ok(Message::Ping(p)))) => {
                    let _ = self.ws.send(Message::Pong(p)).await;
                }
                Ok(Some(Ok(_))) => {}
                _ => break,
            }
        }
    }

    pub(crate) async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener as StdUnixListener;

    use tokio::io::AsyncReadExt;

    use super::*;

    #[tokio::test]
    async fn http_get_json_over_abstract_uds_handles_keepalive_content_length() {
        let sock_name = format!("strata_cdp_test_{}_{}", std::process::id(), 1);
        let addr = SocketAddr::from_abstract_name(sock_name.as_bytes()).expect("abstract addr");
        let std_listener = StdUnixListener::bind_addr(&addr).expect("bind abstract");
        std_listener.set_nonblocking(true).expect("nonblocking");
        let listener = tokio::net::UnixListener::from_std(std_listener).expect("tokio listener");

        let body = br#"[{"type":"page"}]"#;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let srv = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.expect("accept");
            let mut req_buf = [0u8; 256];
            let _ = conn.read(&mut req_buf).await;
            conn.write_all(head.as_bytes()).await.expect("write head");
            conn.write_all(body).await.expect("write body");
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let val = http_get_json(&sock_name, "/json/list", Duration::from_millis(800))
            .await
            .expect("http_get_json without EOF");
        srv.abort();
        assert_eq!(val, json!([{"type": "page"}]));
    }
}
