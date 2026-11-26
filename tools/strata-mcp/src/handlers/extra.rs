//! Visual capture, L1 WebView CDP debug/forward, and L4 atomic observe MCP handlers.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::gesture_arg::normalize_action_gesture;
use crate::devices::DeviceRegistry;
use crate::params::{CdpDebugParams, ObserveStepParams, VisualCaptureParams};
use crate::transport::{
    RetryPolicy, open_ssh_port_forward, shell_quote, ssh_exec_argv, ssh_exec_argv_once,
    ssh_exec_raw_with_policy, transfer_pull,
};

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn push_opt(cmd: &mut Vec<String>, flag: &str, val: Option<&str>) {
    if let Some(v) = val.map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push(flag.to_owned());
        cmd.push(v.to_owned());
    }
}

/// Runs L3 visual capture, optional local pull, and optional inline base64 thumbnail extraction.
pub(crate) async fn handle_visual(reg: &DeviceRegistry, p: VisualCaptureParams) -> Value {
    let target = p.target.to_override();
    let want_inline = p.return_image.unwrap_or(false);
    let want_pull = p.pull_to_local.unwrap_or(false) || p.local_out.is_some();
    let fmt = p.format.unwrap_or_else(|| {
        if want_inline {
            "jpeg".to_owned()
        } else {
            "webp".to_owned()
        }
    });
    let ext = match fmt.to_ascii_lowercase().as_str() {
        "png" => "png",
        "jpg" | "jpeg" => "jpg",
        _ => "webp",
    };

    let remote_tmp = want_pull.then(|| {
        format!(
            "/tmp/strata-mcp-cap-{}-{}.{ext}",
            std::process::id(),
            now_millis()
        )
    });
    let mut cmd = vec![
        "strata-debug".to_owned(),
        "visual".to_owned(),
        "--format".to_owned(),
        fmt,
        "--quality".to_owned(),
        p.quality.unwrap_or(75).to_string(),
    ];
    if let Some(mw) = p.max_width.or_else(|| want_inline.then_some(720)) {
        cmd.extend(["--max-width".to_owned(), mw.to_string()]);
    }
    push_opt(&mut cmd, "--roi", p.roi.as_deref());
    push_opt(&mut cmd, "--golden", p.golden.as_deref());
    push_opt(&mut cmd, "--out", remote_tmp.as_deref());
    if p.burst_sheet.unwrap_or(false) {
        cmd.push("--burst-sheet".to_owned());
    }
    if want_inline {
        cmd.push("--inline-base64".to_owned());
    }

    let mut res = ssh_exec_argv(reg, &target, &cmd, 30.0).await;
    if let Some(rpath) = remote_tmp
        && res["ok"] == json!(true)
    {
        let local = p.local_out.unwrap_or_else(|| {
            std::env::temp_dir()
                .join("strata-mcp-captures")
                .join(format!("cap-{}.{ext}", now_millis()))
                .to_string_lossy()
                .into_owned()
        });
        let is_adb_host = res["route"] == json!("adb-host");
        let pull = transfer_pull(reg, &target, &rpath, &local, is_adb_host).await;
        let rm_cmd = if is_adb_host {
            vec![
                "host-exec".to_owned(),
                "/system/bin/rm".to_owned(),
                "-f".to_owned(),
                rpath,
            ]
        } else {
            vec!["rm".to_owned(), "-f".to_owned(), rpath]
        };
        let _ = ssh_exec_argv(reg, &target, &rm_cmd, 10.0).await;
        if pull["ok"] == json!(true) {
            res["pulled"] = json!(true);
            res["localPath"] = json!(local);
        } else {
            res["pulled"] = json!(false);
            res["pullError"] = pull["error"].clone();
        }
    }
    res
}

/// Dispatches L1 WebView CDP (`list` / `eval` / `audit` / `console` / `forward`).
pub(crate) async fn handle_cdp(reg: &DeviceRegistry, p: CdpDebugParams) -> Value {
    let target = p.target.to_override();
    match p.action.as_str() {
        "list" | "list-targets" => {
            let mut cmd = vec![
                "strata-debug".to_owned(),
                "cdp".to_owned(),
                "list-targets".to_owned(),
            ];
            push_opt(&mut cmd, "--target", p.target_app.as_deref());
            if let Some(pid) = p.pid {
                cmd.extend(["--pid".to_owned(), pid.to_string()]);
            }
            ssh_exec_argv(reg, &target, &cmd, 20.0).await
        }
        "eval" => {
            let Some(expr) = p.expression.as_deref() else {
                return json!({"ok": false, "error": "expression is required for cdp_debug(action='eval')"});
            };
            let mut cmd = vec![
                "strata-debug".to_owned(),
                "cdp".to_owned(),
                "eval".to_owned(),
                expr.to_owned(),
            ];
            push_opt(&mut cmd, "--target", p.target_app.as_deref());
            if let Some(pid) = p.pid {
                cmd.extend(["--pid".to_owned(), pid.to_string()]);
            }
            push_opt(&mut cmd, "--url-match", p.url_match.as_deref());
            let t = p.timeout_sec.unwrap_or(5.0);
            cmd.extend(["--timeout".to_owned(), t.to_string()]);
            ssh_exec_argv_once(reg, &target, &cmd, t + 10.0).await
        }
        "audit" => {
            let mut cmd = vec![
                "strata-debug".to_owned(),
                "cdp".to_owned(),
                "audit".to_owned(),
            ];
            push_opt(&mut cmd, "--target", p.target_app.as_deref());
            if let Some(pid) = p.pid {
                cmd.extend(["--pid".to_owned(), pid.to_string()]);
            }
            push_opt(&mut cmd, "--url-match", p.url_match.as_deref());
            push_opt(&mut cmd, "--eval-js", p.eval_js.as_deref());
            ssh_exec_argv(reg, &target, &cmd, 25.0).await
        }
        "console" => {
            let mut cmd = vec![
                "strata-debug".to_owned(),
                "cdp".to_owned(),
                "console".to_owned(),
            ];
            push_opt(&mut cmd, "--target", p.target_app.as_deref());
            if let Some(pid) = p.pid {
                cmd.extend(["--pid".to_owned(), pid.to_string()]);
            }
            push_opt(&mut cmd, "--url-match", p.url_match.as_deref());
            let dur = p.duration_sec.unwrap_or(2.0);
            cmd.extend(["--duration".to_owned(), dur.to_string()]);
            ssh_exec_argv(reg, &target, &cmd, dur + 12.0).await
        }
        "forward" => {
            let port = p.local_port.unwrap_or(9222);
            let mut launch = format!("nohup strata-debug cdp forward --port {port}");
            if let Some(t) = p.target_app.as_deref() {
                launch.push_str(&format!(" --target {}", shell_quote(t)));
            }
            if let Some(pid) = p.pid {
                launch.push_str(&format!(" --pid {pid}"));
            }
            launch.push_str(" </dev/null >/dev/null 2>&1 & echo $!");
            let start =
                ssh_exec_raw_with_policy(reg, &target, &launch, 10.0, RetryPolicy::SideEffect)
                    .await;
            let tun = open_ssh_port_forward(reg, &target, port, port).await;
            json!({
                "ok": start["ok"] == json!(true) && tun["ok"] == json!(true),
                "localPort": port, "remote": start, "tunnel": tun,
            })
        }
        other => json!({"ok": false, "error": format!("unsupported cdp action '{other}'")}),
    }
}

fn build_observe_argv(p: ObserveStepParams) -> Result<(Vec<String>, bool), String> {
    let mut cmd = vec![
        "strata-debug".to_owned(),
        "observe".to_owned(),
        "--settle-ms".to_owned(),
        p.settle_ms.unwrap_or(350).to_string(),
        "--package".to_owned(),
        p.package.unwrap_or_else(|| "auto".to_owned()),
        "--priority".to_owned(),
        p.priority.unwrap_or_else(|| "W".to_owned()),
    ];
    push_opt(&mut cmd, "--cursor", p.cursor.as_deref());
    push_opt(&mut cmd, "--tag", p.tag.as_deref());
    push_opt(&mut cmd, "--roi", p.roi.as_deref());
    if p.include_cdp == Some(false) {
        cmd.push("--no-cdp".to_owned());
    }
    if p.include_visual.unwrap_or(false) {
        cmd.push("--visual".to_owned());
    }
    if p.return_image.unwrap_or(false) {
        cmd.push("--inline-image".to_owned());
    }
    let mut has_action = false;
    if let Some(rpc) = p.action_rpc
        && rpc.len() == 2
    {
        has_action = true;
        cmd.extend(["--action-rpc".to_owned(), rpc[0].clone(), rpc[1].clone()]);
    }
    if p.action_shell
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        has_action = true;
    }
    push_opt(&mut cmd, "--action-shell", p.action_shell.as_deref());
    if p.action_cdp_eval
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        has_action = true;
    }
    push_opt(&mut cmd, "--action-cdp-eval", p.action_cdp_eval.as_deref());
    if let Some(g) = p.action_gesture.filter(|v| !v.is_null()) {
        let encoded = normalize_action_gesture(g)?;
        has_action = true;
        cmd.extend(["--action-gesture".to_owned(), encoded]);
    }
    Ok((cmd, has_action))
}

/// Runs L4 atomic observe step with optional inline visual capture.
pub(crate) async fn handle_observe(reg: &DeviceRegistry, p: ObserveStepParams) -> Value {
    let target = p.target.to_override();
    let (cmd, has_action) = match build_observe_argv(p) {
        Ok(v) => v,
        Err(err) => return json!({"ok": false, "error": err}),
    };
    if has_action {
        ssh_exec_argv_once(reg, &target, &cmd, 45.0).await
    } else {
        ssh_exec_argv(reg, &target, &cmd, 45.0).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::TargetArgs;

    #[test]
    fn builds_observe_argv_with_single_encoded_swipe_gesture() {
        let params = ObserveStepParams {
            action_rpc: None,
            action_shell: None,
            action_cdp_eval: None,
            action_gesture: Some(Value::String(
                r#"{"kind":"swipe","x0":10,"y0":336,"x1":900,"y1":336,"durationMs":240}"#
                    .to_owned(),
            )),
            cursor: None,
            settle_ms: None,
            package: None,
            priority: None,
            tag: None,
            include_cdp: None,
            include_visual: None,
            return_image: None,
            roi: None,
            target: TargetArgs::default(),
        };
        let (argv, has_action) = build_observe_argv(params).expect("argv");
        assert!(has_action);
        let idx = argv
            .iter()
            .position(|a| a == "--action-gesture")
            .expect("flag");
        let parsed: Value = serde_json::from_str(&argv[idx + 1]).expect("single-level JSON");
        assert_eq!(parsed["kind"], json!("swipe"));
        assert_eq!(parsed["x0"], json!(10));
        assert_eq!(parsed["x1"], json!(900));
    }
}
