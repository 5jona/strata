//! Persistent NDJSON JSON-RPC 2.0 stdin/stdout loop (`strata-debug serve-rpc`).

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::cli::Cli;
use clap::Parser;

/// Reads newline-delimited JSON-RPC requests (`{"id":..., "method":"cli", "params":{"argv":[...]}}`)
/// and executes them in-process without spawning new `strata-debug` binaries.
pub(crate) async fn run_serve_rpc() -> i32 {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Value>(trimmed) {
            Ok(req) => handle_request(req).await,
            Err(err) => json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": {"code": -32700, "message": format!("parse error: {err}")},
            }),
        };
        let mut out = serde_json::to_string(&resp).unwrap_or_else(|_| "{}".to_owned());
        out.push('\n');
        if stdout.write_all(out.as_bytes()).await.is_err() || stdout.flush().await.is_err() {
            break;
        }
    }
    0
}

async fn handle_request(req: Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "ping" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"pong": true, "version": env!("CARGO_PKG_VERSION")},
        }),
        "cli" => {
            let mut argv = vec!["strata-debug".to_owned()];
            if let Some(arr) = params.get("argv").and_then(Value::as_array) {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        argv.push(s.to_owned());
                    }
                }
            }
            match Cli::try_parse_from(argv) {
                Ok(parsed) => {
                    let result = crate::dispatch::dispatch(parsed.cmd).await;
                    json!({"jsonrpc": "2.0", "id": id, "result": result})
                }
                Err(err) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32602, "message": err.to_string()},
                }),
            }
        }
        other => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("unknown method '{other}'")},
        }),
    }
}
