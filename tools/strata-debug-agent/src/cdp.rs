//! L1 WebView CDP target listing, JS evaluation, console capture, and DOM auditing.

pub(crate) mod discovery;
pub(crate) mod forward;
pub(crate) mod http;
pub(crate) mod uds;

use std::time::Duration;

use serde_json::{Value, json};

pub(crate) use discovery::{DevToolsSocket, match_socket, scan_devtools_sockets};

pub(crate) const DOM_AUDIT_JS: &str = include_str!("../js/dom_audit.js");

/// Lists discovered DevTools sockets and queries `/json/list` targets from each.
pub(crate) async fn list_targets(target: Option<&str>, pid: Option<u32>) -> Value {
    let all = scan_devtools_sockets();
    let filtered: Vec<DevToolsSocket> = all
        .into_iter()
        .filter(|s| {
            if let Some(p) = pid {
                s.pid == Some(p)
            } else if let Some(t) = target {
                s.process.contains(t)
            } else {
                true
            }
        })
        .collect();

    let mut targets = Vec::new();
    let mut sockets_json = Vec::new();
    for s in filtered {
        let mut entry = json!({"socket": s.socket, "pid": s.pid, "process": s.process});
        match uds::http_get_json(&s.socket, "/json/list", Duration::from_secs(3)).await {
            Ok(Value::Array(items)) => {
                for mut item in items {
                    if let Some(obj) = item.as_object_mut() {
                        obj.entry("socket").or_insert_with(|| json!(s.socket));
                        obj.entry("pid").or_insert_with(|| json!(s.pid));
                        obj.entry("process").or_insert_with(|| json!(s.process));
                    }
                    targets.push(item);
                }
            }
            Ok(_) => {}
            Err(err) => {
                entry["probeError"] = json!(err);
            }
        }
        sockets_json.push(entry);
    }
    json!({"ok": true, "sockets": sockets_json, "targets": targets})
}

fn ws_path_from_page(page: &Value) -> String {
    if let Some(ws) = page["webSocketDebuggerUrl"].as_str()
        && let Some(idx) = ws.find("/devtools/")
    {
        return ws[idx..].to_owned();
    }
    format!("/devtools/page/{}", page["id"].as_str().unwrap_or(""))
}

async fn select_page(
    target: Option<&str>,
    pid: Option<u32>,
    url_match: Option<&str>,
    timeout: Duration,
) -> Result<(DevToolsSocket, Value), Value> {
    let Some(sock) = match_socket(target, pid) else {
        return Err(json!({
            "ok": false,
            "error": "no webview_devtools_remote socket found",
            "hint": "Enable WebView.setWebContentsDebuggingEnabled(true) and keep the WebView in foreground.",
        }));
    };
    let raw = uds::http_get_json(&sock.socket, "/json/list", timeout)
        .await
        .map_err(|e| json!({"ok": false, "socket": sock.socket, "error": format!("CDP HTTP query failed: {e}")}))?;
    let pages: Vec<Value> = raw
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|v| v.get("type").and_then(Value::as_str).unwrap_or("page") == "page")
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let chosen = url_match
        .and_then(|u| {
            pages
                .iter()
                .find(|p| p["url"].as_str().is_some_and(|s| s.contains(u)))
                .cloned()
        })
        .or_else(|| pages.first().cloned());
    let Some(page) = chosen else {
        return Err(
            json!({"ok": false, "socket": sock.socket, "error": "no WebView page targets found"}),
        );
    };
    Ok((sock, page))
}

/// Evaluates a JavaScript expression in a matched WebView target over abstract UDS CDP WebSocket.
pub(crate) async fn eval_js(
    expression: &str,
    target: Option<&str>,
    pid: Option<u32>,
    url_match: Option<&str>,
    timeout_sec: f64,
) -> Value {
    let dur = Duration::from_secs_f64(timeout_sec.clamp(1.0, 30.0));
    let (sock, page) = match select_page(target, pid, url_match, dur).await {
        Ok(pair) => pair,
        Err(err) => return err,
    };
    let mut client = match uds::CdpWsClient::connect_abstract(
        &sock.socket,
        &ws_path_from_page(&page),
        dur,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "socket": sock.socket, "error": e}),
    };
    let _ = client.call("Runtime.enable", json!({}), dur).await;
    let res = client
        .call(
            "Runtime.evaluate",
            json!({"expression": expression, "returnByValue": true, "awaitPromise": true}),
            dur,
        )
        .await;
    client.close().await;
    match res {
        Ok(msg) => {
            let exc = msg["result"]
                .get("exceptionDetails")
                .cloned()
                .filter(|v| !v.is_null());
            json!({
                "ok": exc.is_none(), "socket": sock.socket, "pid": sock.pid,
                "pageUrl": page["url"],
                "value": msg["result"]["result"].get("value").cloned().unwrap_or(Value::Null),
                "exception": exc,
            })
        }
        Err(e) => json!({"ok": false, "socket": sock.socket, "pageUrl": page["url"], "error": e}),
    }
}

/// Captures console events and uncaught exceptions for `duration_sec`.
pub(crate) async fn collect_console(
    target: Option<&str>,
    pid: Option<u32>,
    url_match: Option<&str>,
    duration_sec: f64,
    timeout_sec: f64,
) -> Value {
    let dur = Duration::from_secs_f64(timeout_sec.clamp(1.0, 30.0));
    let (sock, page) = match select_page(target, pid, url_match, dur).await {
        Ok(pair) => pair,
        Err(err) => return err,
    };
    let mut client = match uds::CdpWsClient::connect_abstract(
        &sock.socket,
        &ws_path_from_page(&page),
        dur,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "socket": sock.socket, "error": e}),
    };
    let _ = client.call("Runtime.enable", json!({}), dur).await;
    client
        .drain_events(Duration::from_secs_f64(duration_sec.clamp(0.1, 20.0)))
        .await;
    let events: Vec<Value> = client.console_events.iter().take(50).cloned().collect();
    let exceptions: Vec<Value> = client.exceptions.iter().take(20).cloned().collect();
    client.close().await;
    json!({"ok": true, "socket": sock.socket, "pid": sock.pid, "pageUrl": page["url"], "events": events, "exceptions": exceptions})
}

/// Executes the embedded `dom_audit.js` script plus optional extra JS evaluation.
pub(crate) async fn audit_webview(
    target: Option<&str>,
    pid: Option<u32>,
    url_match: Option<&str>,
    extra_js: Option<&str>,
    timeout_sec: f64,
) -> Value {
    let dur = Duration::from_secs_f64(timeout_sec.clamp(1.0, 30.0));
    let (sock, page) = match select_page(target, pid, url_match, dur).await {
        Ok(pair) => pair,
        Err(err) => return err,
    };
    let mut client = match uds::CdpWsClient::connect_abstract(
        &sock.socket,
        &ws_path_from_page(&page),
        dur,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "socket": sock.socket, "error": e}),
    };
    let _ = client.call("Runtime.enable", json!({}), dur).await;
    let audit_res = client
        .call(
            "Runtime.evaluate",
            json!({"expression": DOM_AUDIT_JS, "returnByValue": true, "awaitPromise": true}),
            dur,
        )
        .await;
    let eval_out = if let Some(expr) = extra_js {
        client
            .call(
                "Runtime.evaluate",
                json!({"expression": expr, "returnByValue": true, "awaitPromise": true}),
                dur,
            )
            .await
            .ok()
            .and_then(|v| v["result"]["result"].get("value").cloned())
    } else {
        None
    };
    let console_errs: Vec<Value> = client
        .console_events
        .iter()
        .filter(|e| matches!(e["type"].as_str(), Some("error" | "warning")))
        .take(10)
        .cloned()
        .collect();
    let exceptions: Vec<Value> = client.exceptions.iter().take(10).cloned().collect();
    client.close().await;

    match audit_res {
        Ok(msg) => json!({
            "ok": true, "socket": sock.socket, "pid": sock.pid, "pageUrl": page["url"],
            "audit": msg["result"]["result"].get("value").cloned().unwrap_or_else(|| json!({})),
            "evalResult": eval_out, "consoleErrors": console_errs, "exceptions": exceptions,
        }),
        Err(e) => json!({"ok": false, "socket": sock.socket, "pageUrl": page["url"], "error": e}),
    }
}

#[cfg(test)]
mod tests {
    use super::DOM_AUDIT_JS;

    #[test]
    fn dom_audit_script_contains_layout_quality_metrics() {
        for key in [
            "horizontalOverflow",
            "horizontalOverflowSelectors",
            "toastBottomOverlap",
            r#"[data-pochita="toast-viewport"]"#,
            r#"[data-pochita="app-shell-bottom"]"#,
            "brokenImages",
            "naturalWidth",
            "domNodeCount",
        ] {
            assert!(
                DOM_AUDIT_JS.contains(key),
                "dom_audit.js missing expected metric/selector: {key}"
            );
        }
    }
}
