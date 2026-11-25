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

/// Detects the current foreground Android package via dumpsys fallback chain.
pub(crate) async fn detect_foreground() -> ForegroundApp {
    let cmds: [&[&str]; 3] = [
        &["/system/bin/dumpsys", "activity", "activities"],
        &["/system/bin/dumpsys", "window", "displays"],
        &["/system/bin/dumpsys", "window", "windows"],
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
    let fg = if package.is_empty() || package == "auto" {
        Some(detect_foreground().await)
    } else {
        None
    };
    let resolved = fg
        .as_ref()
        .and_then(|f| f.package.clone())
        .unwrap_or_else(|| {
            if package.is_empty() || package == "auto" {
                "unknown".to_owned()
            } else {
                package.to_owned()
            }
        });

    let win_fut = run_host(
        &["/system/bin/dumpsys", "window", "windows"],
        None,
        Duration::from_secs(8),
    );
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

    let (win_res, gfx_res, sf_res, daemon, containers) =
        tokio::join!(win_fut, gfx_fut, sf_fut, daemon_fut, list_fut);

    let windows = parse_window_windows(&win_res.stdout_text(), true);
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
