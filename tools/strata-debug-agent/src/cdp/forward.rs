//! Async single-process TCP <-> abstract UDS transparent proxy for CDP DevTools forwarding.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::uds::connect_abstract;

/// Rewrites `webSocketDebuggerUrl` fields in `/json`, `/json/list`, and `/json/version` responses.
pub(crate) fn rewrite_ws_urls(body: &[u8], listen_port: u16) -> Vec<u8> {
    let Ok(mut val) = serde_json::from_slice::<Value>(body) else {
        return body.to_vec();
    };
    let patch_obj = |obj: &mut serde_json::Map<String, Value>| {
        if let Some(ws) = obj.get("webSocketDebuggerUrl").and_then(Value::as_str)
            && let Some(idx) = ws.find("/devtools/")
        {
            let rewritten = format!("ws://127.0.0.1:{listen_port}{}", &ws[idx..]);
            obj.insert("webSocketDebuggerUrl".to_owned(), json!(rewritten));
        }
    };
    match &mut val {
        Value::Array(items) => {
            for item in items {
                if let Some(obj) = item.as_object_mut() {
                    patch_obj(obj);
                }
            }
        }
        Value::Object(obj) => patch_obj(obj),
        _ => return body.to_vec(),
    }
    serde_json::to_vec(&val).unwrap_or_else(|_| body.to_vec())
}

async fn handle_client(mut client: TcpStream, socket_name: String, listen_port: u16) {
    let mut head_buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 2048];
    let sep_pos = loop {
        match tokio::time::timeout(Duration::from_secs(5), client.read(&mut tmp)).await {
            Ok(Ok(0)) | Err(_) | Ok(Err(_)) => return,
            Ok(Ok(n)) => {
                head_buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = head_buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos;
                }
                if head_buf.len() > 32_768 {
                    return;
                }
            }
        }
    };

    let head_str = String::from_utf8_lossy(&head_buf[..sep_pos]).into_owned();
    let is_ws = head_str.lines().any(|l| {
        l.to_ascii_lowercase().starts_with("upgrade:")
            && l.to_ascii_lowercase().contains("websocket")
    });

    if is_ws {
        let Ok(mut upstream) = connect_abstract(&socket_name, Duration::from_secs(5)).await else {
            return;
        };
        if upstream.write_all(&head_buf).await.is_err() {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        return;
    }

    let path = head_str
        .lines()
        .next()
        .and_then(|req| req.split_whitespace().nth(1))
        .unwrap_or("/json/list");
    let clean_path = path.split('?').next().unwrap_or(path);
    match super::uds::http_get_raw(&socket_name, path, Duration::from_secs(5)).await {
        Ok((status, mut body)) => {
            if status == 200 && matches!(clean_path, "/json" | "/json/list" | "/json/version") {
                body = rewrite_ws_urls(&body, listen_port);
            }
            let resp_head = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = client.write_all(resp_head.as_bytes()).await;
            let _ = client.write_all(&body).await;
        }
        Err(_) => {
            let _ = client
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        }
    }
}

/// Runs an async TCP listener on `127.0.0.1:<port>` forwarding HTTP + WebSocket to `@<socket_name>`.
pub(crate) async fn serve_cdp_proxy(port: u16, socket_name: String) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("bind 127.0.0.1:{port} failed: {e}"))?;
    let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((stream, _)) => {
                        let _ = stream.set_nodelay(true);
                        let sock = socket_name.clone();
                        tokio::spawn(handle_client(stream, sock, actual_port));
                    }
                    Err(_) => break,
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_websocket_debugger_urls() {
        let body = br#"[{"id":"1","webSocketDebuggerUrl":"ws://127.0.0.1/devtools/page/1"}]"#;
        let out = rewrite_ws_urls(body, 9333);
        let parsed: Value = serde_json::from_slice(&out).expect("json");
        assert_eq!(
            parsed[0]["webSocketDebuggerUrl"],
            json!("ws://127.0.0.1:9333/devtools/page/1")
        );
    }
}
