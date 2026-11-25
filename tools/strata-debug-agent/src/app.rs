//! Android application lifecycle, foreground inspection, and package diagnostics.

mod parsers;

use std::time::Duration;

use serde_json::{Value, json};

use crate::health::{detect_foreground, parse_gfxinfo};
use crate::host::run_host;
use parsers::{parse_dumpsys_package, parse_meminfo_summary};

fn read_uid_for_pid(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if line.starts_with("Uid:") {
            return line.split_whitespace().nth(1)?.parse().ok();
        }
    }
    None
}

async fn get_pids(package: &str) -> Vec<u32> {
    let res = run_host(
        &["/system/bin/pidof", package],
        None,
        Duration::from_secs(5),
    )
    .await;
    if !res.ok() {
        return Vec::new();
    }
    res.stdout_text()
        .split_whitespace()
        .filter_map(|t| t.parse::<u32>().ok())
        .collect()
}

/// Inspects top foreground activity or full package metadata, memory, and gfxinfo concurrently.
pub(crate) async fn inspect_app(package: &str, top_only: bool) -> Value {
    let need_fg = top_only || package.is_empty() || package == "auto";
    let fg = if need_fg {
        Some(detect_foreground().await)
    } else {
        None
    };
    let resolved = if package.is_empty() || package == "auto" {
        fg.as_ref()
            .and_then(|f| f.package.clone())
            .unwrap_or_default()
    } else {
        package.to_owned()
    };
    if resolved.is_empty() {
        return json!({"ok": false, "error": "unable to detect foreground package"});
    }

    if top_only {
        let pids = get_pids(&resolved).await;
        let pid = pids.first().copied();
        let uid = pid.and_then(read_uid_for_pid);
        return json!({
            "ok": true, "package": resolved,
            "activity": fg.as_ref().and_then(|f| f.activity.clone()),
            "component": fg.as_ref().and_then(|f| f.component.clone()),
            "pid": pid, "uid": uid,
        });
    }

    let pkg_cmd = ["/system/bin/dumpsys", "package", &resolved];
    let mem_cmd = ["/system/bin/dumpsys", "meminfo", &resolved];
    let gfx_cmd = ["/system/bin/dumpsys", "gfxinfo", &resolved];
    let (pids, pkg_res, mem_res, gfx_res) = tokio::join!(
        get_pids(&resolved),
        run_host(&pkg_cmd, None, Duration::from_secs(10)),
        run_host(&mem_cmd, None, Duration::from_secs(10)),
        run_host(&gfx_cmd, None, Duration::from_secs(10)),
    );
    let pid = pids.first().copied();
    let uid = pid.and_then(read_uid_for_pid);

    json!({
        "ok": true, "package": resolved,
        "activity": fg.as_ref().and_then(|f| f.activity.clone()),
        "pid": pid, "pids": pids, "uid": uid,
        "packageInfo": parse_dumpsys_package(&pkg_res.stdout_text()),
        "meminfo": parse_meminfo_summary(&mem_res.stdout_text()),
        "gfxinfo": parse_gfxinfo(&gfx_res.stdout_text()),
    })
}

/// Starts, force-stops, or clears data of an Android application package.
pub(crate) async fn control_app(
    action: &str,
    package: &str,
    activity: Option<&str>,
    intent_action: Option<&str>,
    data_uri: Option<&str>,
) -> Value {
    match action {
        "stop" => {
            let res = run_host(
                &["/system/bin/am", "force-stop", package],
                None,
                Duration::from_secs(10),
            )
            .await;
            json!({"ok": res.ok(), "action": "stop", "package": package, "rc": res.code, "stderr": res.stderr_text()})
        }
        "clear" => {
            let res = run_host(
                &["/system/bin/pm", "clear", package],
                None,
                Duration::from_secs(15),
            )
            .await;
            json!({"ok": res.ok(), "action": "clear", "package": package, "rc": res.code, "stdout": res.stdout_text().trim(), "stderr": res.stderr_text()})
        }
        "start" => {
            let mut cmd: Vec<String> = vec!["/system/bin/am".to_owned(), "start".to_owned()];
            let mut method = "am-start";
            let mut component: Option<String> = None;

            if let Some(act) = activity {
                let comp = if act.contains('/') {
                    act.to_owned()
                } else {
                    format!("{package}/{act}")
                };
                cmd.extend(["-n".to_owned(), comp.clone()]);
                component = Some(comp);
                if let Some(a) = intent_action {
                    cmd.extend(["-a".to_owned(), a.to_owned()]);
                }
                if let Some(d) = data_uri {
                    cmd.extend(["-d".to_owned(), d.to_owned()]);
                }
            } else if intent_action.is_some() || data_uri.is_some() {
                cmd.extend(["-p".to_owned(), package.to_owned()]);
                if let Some(a) = intent_action {
                    cmd.extend(["-a".to_owned(), a.to_owned()]);
                }
                if let Some(d) = data_uri {
                    cmd.extend(["-d".to_owned(), d.to_owned()]);
                }
            } else {
                let resolved = run_host(
                    &[
                        "/system/bin/cmd",
                        "package",
                        "resolve-activity",
                        "--brief",
                        package,
                    ],
                    None,
                    Duration::from_secs(8),
                )
                .await;
                for line in resolved.stdout_text().lines().rev() {
                    let cand = line.trim();
                    if cand.contains('/') && !cand.starts_with("priority=") {
                        component = Some(cand.to_owned());
                        break;
                    }
                }
                if let Some(comp) = component.as_ref() {
                    cmd.extend(["-n".to_owned(), comp.clone()]);
                } else {
                    method = "monkey";
                    cmd = vec![
                        "/system/bin/monkey".to_owned(),
                        "-p".to_owned(),
                        package.to_owned(),
                        "-c".to_owned(),
                        "android.intent.category.LAUNCHER".to_owned(),
                        "1".to_owned(),
                    ];
                }
            }
            let refs: Vec<&str> = cmd.iter().map(String::as_str).collect();
            let res = run_host(&refs, None, Duration::from_secs(15)).await;
            json!({
                "ok": res.ok(), "action": "start", "package": package, "method": method,
                "component": component, "rc": res.code,
                "stdout": res.stdout_text().trim().chars().take(400).collect::<String>(),
                "stderr": res.stderr_text().trim().chars().take(240).collect::<String>(),
            })
        }
        other => json!({"ok": false, "error": format!("unsupported app action: {other}")}),
    }
}

/// Installs an APK on Android via `pm install`.
pub(crate) async fn install_apk(
    apk_path: &str,
    replace: bool,
    grant: bool,
    downgrade: bool,
) -> Value {
    let mut args = vec!["/system/bin/pm", "install"];
    if replace {
        args.push("-r");
    }
    if grant {
        args.push("-g");
    }
    if downgrade {
        args.push("-d");
    }
    args.push(apk_path);
    let res = run_host(&args, None, Duration::from_secs(60)).await;
    let stdout = res.stdout_text();
    let ok = res.ok() && !stdout.contains("Failure");
    json!({
        "ok": ok, "apkPath": apk_path, "rc": res.code,
        "stdout": stdout.trim().chars().take(400).collect::<String>(),
        "stderr": res.stderr_text().trim().chars().take(240).collect::<String>(),
    })
}
