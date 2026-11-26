//! L4 atomic single-shot observe step: mark -> action -> settle -> concurrent signal collection.

mod gesture_spec;

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::cdp;
use crate::gesture::run_gesture;
use crate::health::{collect_health_with_snapshot, fetch_windows_and_foreground};
use crate::host::run_host;
use crate::logcat::{DEFAULT_CURSOR_PATH, mark_cursor, read_incremental};
use crate::visual::{VisualOpts, run_visual};
use gesture_spec::parse_gesture_spec;

const STRATA_BIN: &str = "/data/adb/ksu/bin/strata";

/// Parameters for an atomic L4 observe step.
#[derive(Debug, Clone)]
pub(crate) struct ObserveOpts {
    pub(crate) action_rpc: Option<(String, String)>,
    pub(crate) action_shell: Option<String>,
    pub(crate) action_cdp_eval: Option<String>,
    pub(crate) action_gesture: Option<Value>,
    pub(crate) cursor: Option<String>,
    pub(crate) settle_ms: u64,
    pub(crate) package: String,
    pub(crate) priority: String,
    pub(crate) tag: Option<String>,
    pub(crate) include_cdp: bool,
    pub(crate) cdp_pid: Option<u32>,
    pub(crate) include_visual: bool,
    pub(crate) roi: Option<String>,
    pub(crate) visual_out: Option<String>,
    pub(crate) inline_image: bool,
    pub(crate) max_width: Option<u32>,
}

async fn execute_action(opts: &ObserveOpts, pkg: Option<&str>) -> (Option<Value>, Option<String>) {
    if let Some((method, params)) = opts.action_rpc.as_ref() {
        let res = run_host(
            &[STRATA_BIN, "--json", "rpc", method, params],
            None,
            Duration::from_secs(15),
        )
        .await;
        let body: Value = serde_json::from_str(&res.stdout_text())
            .unwrap_or_else(|_| json!(res.stdout_text().trim()));
        return (
            Some(json!({"type": "rpc", "method": method, "rc": res.code, "result": body})),
            if res.ok() {
                None
            } else {
                Some(res.stderr_text())
            },
        );
    }
    if let Some(shell) = opts.action_shell.as_deref() {
        let res = run_host(
            &["/system/bin/sh", "-c", shell],
            None,
            Duration::from_secs(15),
        )
        .await;
        return (
            Some(json!({
                "type": "shell",
                "rc": res.code,
                "stdout": crate::host::tail_chars(res.stdout_text().trim(), 1000),
                "stderr": crate::host::tail_chars(res.stderr_text().trim(), 400),
            })),
            if res.ok() {
                None
            } else {
                Some(format!("shell exited rc={}", res.code))
            },
        );
    }
    if let Some(expr) = opts.action_cdp_eval.as_deref() {
        let out = cdp::eval_js(expr, pkg, opts.cdp_pid, None, 5.0).await;
        let err = if out["ok"] == json!(true) {
            None
        } else {
            out["error"].as_str().map(str::to_owned)
        };
        return (Some(json!({"type": "cdp-eval", "result": out})), err);
    }
    if let Some(spec) = opts.action_gesture.as_ref() {
        return match parse_gesture_spec(spec) {
            Ok(kind) => {
                let res = run_gesture(kind).await;
                let err = if res["ok"] == json!(true) {
                    None
                } else {
                    res["error"].as_str().map(str::to_owned)
                };
                (Some(json!({"type": "gesture", "result": res})), err)
            }
            Err(e) => (None, Some(e)),
        };
    }
    (None, None)
}

/// Runs an atomic L4 observation step with parallel post-settle evidence collection.
pub(crate) async fn run_observe(opts: &ObserveOpts) -> Value {
    let t0 = Instant::now();
    let auto_pkg = opts.package.is_empty() || opts.package == "auto";
    let has_action = opts.action_rpc.is_some()
        || opts.action_shell.is_some()
        || opts.action_cdp_eval.is_some()
        || opts.action_gesture.is_some();

    let pre_pkg = if !auto_pkg {
        Some(opts.package.clone())
    } else if opts.action_cdp_eval.is_some() {
        fetch_windows_and_foreground().await.1.package
    } else {
        None
    };

    let eff_cursor = opts.cursor.as_deref().unwrap_or(DEFAULT_CURSOR_PATH);

    let cursor_before = if opts.cursor.is_none() || (has_action && !Path::new(eff_cursor).is_file())
    {
        Some(mark_cursor(eff_cursor).await)
    } else {
        None
    };

    let (action_result, action_error) = execute_action(opts, pre_pkg.as_deref()).await;

    if opts.settle_ms > 0 && (has_action || opts.cursor.is_none()) {
        tokio::time::sleep(Duration::from_millis(opts.settle_ms)).await;
    }

    let (win_text, post_fg) = if auto_pkg {
        let (wt, fg) = fetch_windows_and_foreground().await;
        (Some(wt), Some(fg))
    } else {
        (None, None)
    };
    let resolved_pkg = if auto_pkg {
        post_fg.as_ref().and_then(|f| f.package.clone())
    } else {
        Some(opts.package.clone())
    };
    let pkg_arg = resolved_pkg.as_deref().unwrap_or("unknown");
    let want_visual =
        opts.include_visual || opts.roi.is_some() || opts.visual_out.is_some() || opts.inline_image;

    let log_fut = read_incremental(
        eff_cursor,
        &opts.priority,
        opts.tag.as_deref(),
        None,
        60,
        true,
    );
    let health_fut = collect_health_with_snapshot(pkg_arg, win_text, post_fg, true, false, None);
    let cdp_fut = async {
        if opts.include_cdp {
            Some(cdp::audit_webview(resolved_pkg.as_deref(), opts.cdp_pid, None, None, 5.0).await)
        } else {
            None
        }
    };
    let vis_fut = async {
        if want_visual {
            Some(
                run_visual(&VisualOpts {
                    roi: opts.roi.clone(),
                    out_path: opts.visual_out.clone(),
                    format: "jpeg".to_owned(),
                    quality: 75,
                    max_width: opts.max_width.or(Some(720)),
                    golden_raw: None,
                    burst_sheet: false,
                    burst_interval_ms: 80,
                    inline_base64: opts.inline_image,
                })
                .await,
            )
        } else {
            None
        }
    };

    let (logs, health, cdp_info, vis_info) = tokio::join!(log_fut, health_fut, cdp_fut, vis_fut);

    json!({
        "ok": action_error.is_none(),
        "elapsedMs": t0.elapsed().as_millis() as u64,
        "cursorPath": eff_cursor,
        "cursorBefore": cursor_before.and_then(|v| v["marked"]["ts"].as_str().map(str::to_owned)),
        "actionResult": action_result,
        "actionError": action_error,
        "logcat": logs,
        "health": health,
        "cdp": cdp_info,
        "visual": vis_info,
    })
}
