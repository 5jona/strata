//! Perfetto trace orchestration (device capture + PC `trace_processor_shell` SQL) and hot-reload verify.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::process::Command;

use crate::devices::DeviceRegistry;
use crate::params::{PerfettoTraceParams, ReloadVerifyParams};
use crate::perfetto_pc;
use crate::transport::{
    TargetOverride, build_routes, probe_containers_via_adb, ssh_exec_argv, ssh_exec_argv_once,
    transfer_pull,
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

/// Manages device Perfetto capture/ring sessions and runs PC-side `trace_processor_shell` PerfettoSQL.
pub(crate) async fn handle_perfetto(reg: &DeviceRegistry, p: PerfettoTraceParams) -> Value {
    let target = p.target.to_override();
    let action = p.action.as_deref().unwrap_or("capture");

    if action == "analyze" {
        let local_trace = if let Some(lp) = p.local_path.clone() {
            lp
        } else if let Some(rp) = p.remote_path.as_deref() {
            let dst = p.local_out.unwrap_or_else(|| {
                std::env::temp_dir()
                    .join("strata-mcp-traces")
                    .join(format!("trace-{}.pftrace", now_millis()))
                    .to_string_lossy()
                    .into_owned()
            });
            let pull = transfer_pull(reg, &target, rp, &dst, rp.starts_with("/data/")).await;
            if pull["ok"] != json!(true) {
                return json!({"ok": false, "stage": "pull", "error": pull["error"]});
            }
            dst
        } else {
            return json!({"ok": false, "error": "localPath or remotePath is required for perfetto_trace(action='analyze')"});
        };
        return perfetto_pc::analyze_local_trace(
            &local_trace,
            p.analysis_preset.as_deref(),
            p.sql.as_deref(),
        )
        .await;
    }

    let mut cmd = vec![
        "strata-debug".to_owned(),
        "perfetto".to_owned(),
        "--action".to_owned(),
        action.to_owned(),
    ];
    push_opt(&mut cmd, "--preset", p.preset.as_deref());
    if let Some(d) = p.duration_sec {
        cmd.extend(["--duration".to_owned(), d.to_string()]);
    }
    if let Some(b) = p.buffer_kb {
        cmd.extend(["--buffer-kb".to_owned(), b.to_string()]);
    }
    if let Some(m) = p.max_duration_sec {
        cmd.extend(["--max-duration".to_owned(), m.to_string()]);
    }
    push_opt(&mut cmd, "--session", p.session_key.as_deref());
    push_opt(&mut cmd, "--out", p.remote_path.as_deref());

    let timeout = f64::from(p.duration_sec.unwrap_or(5)) + 30.0;
    let mut res = if action == "status" {
        ssh_exec_argv(reg, &target, &cmd, timeout).await
    } else {
        ssh_exec_argv_once(reg, &target, &cmd, timeout).await
    };

    let want_pull = matches!(action, "capture" | "snapshot" | "stop")
        && (p.pull_to_local.unwrap_or(false)
            || p.local_out.is_some()
            || p.analysis_preset.is_some()
            || p.sql.is_some());
    if want_pull
        && res["ok"] == json!(true)
        && let Some(rpath) = res["remotePath"].as_str().map(str::to_owned)
    {
        let dst = p.local_out.unwrap_or_else(|| {
            std::env::temp_dir()
                .join("strata-mcp-traces")
                .join(format!("trace-{action}-{}.pftrace", now_millis()))
                .to_string_lossy()
                .into_owned()
        });
        let pull = transfer_pull(reg, &target, &rpath, &dst, rpath.starts_with("/data/")).await;
        if pull["ok"] == json!(true) {
            res["pulled"] = json!(true);
            res["localPath"] = json!(dst.clone());
            if p.analysis_preset.is_some() || p.sql.is_some() {
                res["pcAnalysis"] = perfetto_pc::analyze_local_trace(
                    &dst,
                    p.analysis_preset.as_deref(),
                    p.sql.as_deref(),
                )
                .await;
            }
        } else {
            res["pulled"] = json!(false);
            res["pullError"] = pull["error"].clone();
        }
    }
    res
}

/// Runs `scripts/reload.sh` (`--ssh` or `--adb` fallback) and non-destructive `scripts/verify-live.py`.
pub(crate) async fn handle_reload_verify(reg: &DeviceRegistry, p: ReloadVerifyParams) -> Value {
    let target: TargetOverride = p.target.to_override();
    let routes = match build_routes(reg, &target).await {
        Ok(r) => r,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let adb_serial = routes
        .iter()
        .find_map(|r| r.adb_serial.as_deref())
        .map(str::to_owned);
    let ssh_route = routes.iter().find(|r| r.kind == "ssh");

    let mut degraded_reason: Option<String> = None;
    let use_adb = if let Some(ser) = adb_serial.as_deref() {
        if ssh_route.is_none() {
            let probe = probe_containers_via_adb(ser, false).await;
            if probe.active_ssh_port.is_none() {
                degraded_reason = Some(probe.degraded_reason.unwrap_or_else(|| {
                    "SSH/container transport unavailable; degraded to scripts/reload.sh --adb"
                        .to_owned()
                }));
                true
            } else {
                true
            }
        } else {
            false
        }
    } else {
        false
    };

    let repo = std::env::var("STRATA_REPO_ROOT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|d| d.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| ".".to_owned());
    let reload_sh = Path::new(&repo).join("scripts/reload.sh");
    let verify_py = Path::new(&repo).join("scripts/verify-live.py");

    let (r_args, v_args, route_kind) = if use_adb {
        let Some(ser) = adb_serial.as_deref() else {
            return json!({"ok": false, "error": "no SSH or ADB route available"});
        };
        let mut r = vec![
            reload_sh.to_string_lossy().into_owned(),
            "--adb".to_owned(),
            ser.to_owned(),
        ];
        if p.skip_build.unwrap_or(true) {
            r.push("--skip-build".to_owned());
        }
        let mut v = vec![
            verify_py.to_string_lossy().into_owned(),
            "--mode".to_owned(),
            "adb".to_owned(),
            "--adb".to_owned(),
            ser.to_owned(),
            "--non-destructive".to_owned(),
        ];
        if p.ui_only.unwrap_or(false) {
            v.push("--ui-only".to_owned());
        }
        (r, v, "adb-host")
    } else {
        let Some(route) = ssh_route else {
            return json!({"ok": false, "error": "no SSH or ADB route available"});
        };
        let mut r = vec![
            reload_sh.to_string_lossy().into_owned(),
            "--ssh".to_owned(),
            route.target_spec(),
        ];
        if let Some(port) = route.port {
            r.extend(["--ssh-port".to_owned(), port.to_string()]);
        }
        if p.skip_build.unwrap_or(true) {
            r.push("--skip-build".to_owned());
        }
        let mut v = vec![
            verify_py.to_string_lossy().into_owned(),
            "--mode".to_owned(),
            "ssh".to_owned(),
            "--ssh".to_owned(),
            route.target_spec(),
            "--non-destructive".to_owned(),
        ];
        if let Some(port) = route.port {
            v.extend(["--ssh-port".to_owned(), port.to_string()]);
        }
        if p.ui_only.unwrap_or(false) {
            v.push("--ui-only".to_owned());
        }
        (r, v, route.kind)
    };

    let mut eff_route = route_kind;
    let r1 = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new("bash").args(&r_args).output(),
    )
    .await;
    let Ok(Ok(mut out1)) = r1 else {
        return json!({"ok": false, "stage": "reload", "route": eff_route, "error": "scripts/reload.sh timed out or failed to spawn"});
    };

    // If SSH reload failed and an online ADB serial is present, automatically fall back to `--adb`.
    let mut eff_v_args = v_args;
    if !out1.status.success()
        && !use_adb
        && let Some(ser) = adb_serial.as_deref()
    {
        let mut adb_r = vec![
            reload_sh.to_string_lossy().into_owned(),
            "--adb".to_owned(),
            ser.to_owned(),
        ];
        if p.skip_build.unwrap_or(true) {
            adb_r.push("--skip-build".to_owned());
        }
        if let Ok(Ok(adb_out)) = tokio::time::timeout(
            Duration::from_secs(120),
            Command::new("bash").args(&adb_r).output(),
        )
        .await
        {
            out1 = adb_out;
            eff_route = "adb-host";
            degraded_reason =
                Some("SSH reload failed; fell back to scripts/reload.sh --adb".to_owned());
            eff_v_args = vec![
                verify_py.to_string_lossy().into_owned(),
                "--mode".to_owned(),
                "adb".to_owned(),
                "--adb".to_owned(),
                ser.to_owned(),
                "--non-destructive".to_owned(),
            ];
            if p.ui_only.unwrap_or(false) {
                eff_v_args.push("--ui-only".to_owned());
            }
        }
    }

    if !out1.status.success() {
        let mut resp = json!({
            "ok": false, "stage": "reload", "route": eff_route, "rc": out1.status.code(),
            "stdout": String::from_utf8_lossy(&out1.stdout),
            "stderr": String::from_utf8_lossy(&out1.stderr),
        });
        if let Some(reason) = degraded_reason {
            resp["degradedReason"] = json!(reason);
        }
        return resp;
    }

    let r2 = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new("python3").args(&eff_v_args).output(),
    )
    .await;
    let mut resp = match r2 {
        Ok(Ok(out2)) => json!({
            "ok": out2.status.success(), "stage": "verify", "route": eff_route, "rc": out2.status.code(),
            "stdout": String::from_utf8_lossy(&out2.stdout),
            "stderr": String::from_utf8_lossy(&out2.stderr),
        }),
        _ => {
            json!({"ok": false, "stage": "verify", "route": eff_route, "error": "verify-live.py timed out or failed to spawn"})
        }
    };
    if let Some(reason) = degraded_reason {
        resp["degradedReason"] = json!(reason);
    }
    resp
}
