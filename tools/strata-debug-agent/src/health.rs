//! L2a system health snapshot: foreground activity, windows, SurfaceFlinger, gfxinfo, and Strata daemon.

mod parsers;

use std::time::Duration;

use serde_json::{Value, json};

use crate::host::run_host;
pub(crate) use parsers::{
    ForegroundApp, parse_foreground_activity, parse_gfxinfo, parse_window_windows,
};

const STRATA_BIN: &str = "/data/adb/ksu/bin/strata";
const SIDEVEIL_CTL: &str = "/data/adb/modules/sideveil/bin/sideveil-ctl";

/// Fetches `dumpsys window windows` once and extracts foreground app if present.
pub(crate) async fn fetch_windows_and_foreground() -> (String, ForegroundApp) {
    let win_res = run_host(
        &["/system/bin/dumpsys", "window", "windows"],
        None,
        Duration::from_secs(8),
    )
    .await;
    let win_text = win_res.stdout_text();
    let parsed = parse_foreground_activity(&win_text);
    if parsed.package.is_some() {
        return (win_text, parsed);
    }
    let fg = detect_foreground_fallback().await;
    (win_text, fg)
}

async fn detect_foreground_fallback() -> ForegroundApp {
    let cmds: [&[&str]; 2] = [
        &["/system/bin/dumpsys", "activity", "activities"],
        &["/system/bin/dumpsys", "window", "displays"],
    ];
    for cmd in cmds {
        let res = run_host(cmd, None, Duration::from_secs(6)).await;
        if res.ok() {
            let parsed = parse_foreground_activity(&res.stdout_text());
            if parsed.package.is_some() {
                return parsed;
            }
        }
    }
    ForegroundApp {
        package: None,
        activity: None,
        component: None,
    }
}

/// Detects the current foreground Android package via dumpsys fallback chain.
pub(crate) async fn detect_foreground() -> ForegroundApp {
    let (_, fg) = fetch_windows_and_foreground().await;
    fg
}

async fn query_strata_json(subargs: &[&str]) -> Option<Value> {
    let mut args = vec![STRATA_BIN, "--json"];
    args.extend_from_slice(subargs);
    let res = run_host(&args, None, Duration::from_secs(5)).await;
    if !res.ok() {
        return None;
    }
    serde_json::from_str::<Value>(&res.stdout_text()).ok()
}

/// Collects concurrent Strata daemon/container health + Android window/gfxinfo/SF snapshot.
pub(crate) async fn collect_health(
    package: &str,
    include_sf: bool,
    include_strata: bool,
    include_sideveil: Option<bool>,
) -> Value {
    collect_health_with_snapshot(
        package,
        None,
        None,
        include_sf,
        include_strata,
        include_sideveil,
    )
    .await
}

/// Collects health reusing optional pre-captured `dumpsys window windows` and foreground info.
pub(crate) async fn collect_health_with_snapshot(
    package: &str,
    pre_windows: Option<String>,
    pre_fg: Option<ForegroundApp>,
    include_sf: bool,
    include_strata: bool,
    include_sideveil: Option<bool>,
) -> Value {
    let auto_pkg = package.is_empty() || package == "auto";
    let (win_text, fg) = match (pre_windows, pre_fg) {
        (Some(wt), fg_opt) => {
            let fg_res = if fg_opt.is_some() {
                fg_opt
            } else if auto_pkg {
                let parsed = parse_foreground_activity(&wt);
                if parsed.package.is_some() {
                    Some(parsed)
                } else {
                    Some(detect_foreground_fallback().await)
                }
            } else {
                None
            };
            (wt, fg_res)
        }
        (None, fg_opt) if auto_pkg && fg_opt.is_none() => {
            let (wt, detected) = fetch_windows_and_foreground().await;
            (wt, Some(detected))
        }
        (None, fg_opt) => {
            let win_res = run_host(
                &["/system/bin/dumpsys", "window", "windows"],
                None,
                Duration::from_secs(8),
            )
            .await;
            (win_res.stdout_text(), fg_opt)
        }
    };

    let resolved = fg
        .as_ref()
        .and_then(|f| f.package.clone())
        .unwrap_or_else(|| {
            if auto_pkg {
                "unknown".to_owned()
            } else {
                package.to_owned()
            }
        });

    let gfx_fut = async {
        if resolved != "unknown" {
            let cmd = ["/system/bin/dumpsys", "gfxinfo", &resolved];
            Some(run_host(&cmd, None, Duration::from_secs(8)).await)
        } else {
            None
        }
    };
    let sf_fut = async {
        if include_sf && resolved != "unknown" {
            Some(
                run_host(
                    &["/system/bin/dumpsys", "SurfaceFlinger", "--list"],
                    None,
                    Duration::from_secs(6),
                )
                .await,
            )
        } else {
            None
        }
    };
    let daemon_fut = async {
        if include_strata {
            query_strata_json(&["daemon", "status"]).await
        } else {
            None
        }
    };
    let list_fut = async {
        if include_strata {
            query_strata_json(&["list"]).await
        } else {
            None
        }
    };

    let (gfx_res, sf_res, daemon, containers) = tokio::join!(gfx_fut, sf_fut, daemon_fut, list_fut);

    let windows = parse_window_windows(&win_text, true);
    let pkg_windows: Vec<Value> = if resolved != "unknown" {
        windows
            .iter()
            .filter(|w| {
                w["name"].as_str().is_some_and(|s| s.contains(&resolved))
                    || w["package"].as_str().is_some_and(|s| s.contains(&resolved))
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    let gfx = gfx_res.map_or_else(|| json!({}), |r| parse_gfxinfo(&r.stdout_text()));
    let sf_layers: Vec<Value> = sf_res
        .filter(crate::host::CmdOutput::ok)
        .map(|r| {
            r.stdout_text()
                .lines()
                .filter(|ln| ln.contains(&resolved))
                .take(20)
                .map(|ln| json!({"name": ln.trim()}))
                .collect()
        })
        .unwrap_or_default();

    let want_sv = include_sideveil.unwrap_or(resolved == "pochita.sideveil.host");
    let sv_inspect = if want_sv {
        let r = run_host(
            &[SIDEVEIL_CTL, "debug", "inspect-ui"],
            None,
            Duration::from_secs(5),
        )
        .await;
        if r.ok() {
            serde_json::from_str::<Value>(&r.stdout_text()).ok()
        } else {
            None
        }
    } else {
        None
    };

    json!({
        "ok": true,
        "package": resolved,
        "foreground": fg.as_ref().and_then(|f| f.component.clone()),
        "visibleWindowCount": windows.len(),
        "packageWindows": pkg_windows,
        "gfxinfo": gfx,
        "surfaceFlingerMatches": sf_layers,
        "daemon": daemon,
        "containers": containers,
        "sideveilInspect": sv_inspect,
    })
}
