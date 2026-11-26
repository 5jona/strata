//! UI interaction, logcat, SQLite, system inspection, and kernel probe MCP handlers.

use serde_json::{Value, json};

use crate::devices::DeviceRegistry;
use crate::params::{
    KernelProbeParams, LogcatReadParams, SqliteInspectParams, SystemInspectParams, UiInteractParams,
};
use crate::transport::{
    RetryPolicy, RouteRequirement, ssh_exec_argv, ssh_exec_argv_once, ssh_exec_argv_with_req,
};

fn push_opt(cmd: &mut Vec<String>, flag: &str, val: Option<&str>) {
    if let Some(v) = val.map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push(flag.to_owned());
        cmd.push(v.to_owned());
    }
}

/// Handles `ui_interact` (`dump` | `tap` | `gesture`).
pub(crate) async fn handle_ui_interact(reg: &DeviceRegistry, p: UiInteractParams) -> Value {
    let target = p.target.to_override();
    match p.action.as_str() {
        "dump" => {
            let mut cmd = vec![
                "strata-debug".to_owned(),
                "ui".to_owned(),
                "dump".to_owned(),
            ];
            if p.compressed.unwrap_or(false) {
                cmd.push("--compressed".to_owned());
            }
            if p.clickable_only.unwrap_or(false) {
                cmd.push("--clickable-only".to_owned());
            }
            if p.compact.unwrap_or(true) {
                cmd.push("--compact".to_owned());
            }
            push_opt(&mut cmd, "--query", p.query.as_deref());
            push_opt(&mut cmd, "--resource-id", p.resource_id.as_deref());
            push_opt(&mut cmd, "--desc", p.desc.as_deref());
            if let Some(l) = p.limit {
                cmd.extend(["--limit".to_owned(), l.to_string()]);
            }
            ssh_exec_argv(reg, &target, &cmd, 25.0).await
        }
        "tap" => {
            let mut cmd = vec!["strata-debug".to_owned(), "ui".to_owned(), "tap".to_owned()];
            push_opt(&mut cmd, "--text", p.text.as_deref().or(p.query.as_deref()));
            push_opt(&mut cmd, "--resource-id", p.resource_id.as_deref());
            push_opt(&mut cmd, "--desc", p.desc.as_deref());
            if let Some(i) = p.index {
                cmd.extend(["--index".to_owned(), i.to_string()]);
            }
            push_opt(&mut cmd, "--method", p.method.as_deref());
            ssh_exec_argv_once(reg, &target, &cmd, 25.0).await
        }
        "gesture" => {
            let kind = p.kind.as_deref().unwrap_or("edge-swipe");
            let mut cmd = vec![
                "strata-debug".to_owned(),
                "gesture".to_owned(),
                kind.to_owned(),
            ];
            match kind {
                "tap" => {
                    let (Some(x), Some(y)) = (p.x, p.y) else {
                        return json!({"ok": false, "error": "x and y are required for gesture tap"});
                    };
                    cmd.extend([x.to_string(), y.to_string()]);
                }
                "long-press" => {
                    let (Some(x), Some(y)) = (p.x, p.y) else {
                        return json!({"ok": false, "error": "x and y are required for gesture long-press"});
                    };
                    cmd.extend([
                        x.to_string(),
                        y.to_string(),
                        "--duration-ms".to_owned(),
                        p.duration_ms.unwrap_or(600).to_string(),
                    ]);
                }
                "swipe" => {
                    let (Some(x0), Some(y0), Some(x1), Some(y1)) = (p.x0, p.y0, p.x1, p.y1) else {
                        return json!({"ok": false, "error": "x0, y0, x1, y1 are required for gesture swipe"});
                    };
                    cmd.extend([
                        x0.to_string(),
                        y0.to_string(),
                        x1.to_string(),
                        y1.to_string(),
                        "--duration-ms".to_owned(),
                        p.duration_ms.unwrap_or(240).to_string(),
                    ]);
                }
                "edge-swipe" => {
                    cmd.extend([
                        "--side".to_owned(),
                        p.side.unwrap_or_else(|| "right".to_owned()),
                        "--y-percent".to_owned(),
                        p.y_percent.unwrap_or(50.0).to_string(),
                        "--distance-dp".to_owned(),
                        p.distance_dp.unwrap_or(96.0).to_string(),
                        "--duration-ms".to_owned(),
                        p.duration_ms.unwrap_or(240).to_string(),
                    ]);
                    if p.cancel_back.unwrap_or(false) {
                        cmd.push("--cancel-back".to_owned());
                    }
                }
                other => {
                    return json!({"ok": false, "error": format!("unsupported gesture kind '{other}'; use tap|long-press|swipe|edge-swipe")});
                }
            }
            ssh_exec_argv_once(reg, &target, &cmd, 25.0).await
        }
        other => json!({"ok": false, "error": format!("unsupported ui_interact action '{other}'")}),
    }
}

pub(crate) async fn handle_logcat(reg: &DeviceRegistry, p: LogcatReadParams) -> Value {
    let target = p.target.to_override();
    let mut cmd = vec!["strata-debug".to_owned(), "logcat".to_owned()];
    if p.mark.unwrap_or(false) {
        cmd.push("--mark".to_owned());
    }
    push_opt(&mut cmd, "--cursor", p.cursor.as_deref());
    if !p.mark.unwrap_or(false) {
        cmd.extend([
            "--priority".to_owned(),
            p.priority.unwrap_or_else(|| "W".to_owned()),
        ]);
        cmd.extend(["--limit".to_owned(), p.limit.unwrap_or(100).to_string()]);
        push_opt(&mut cmd, "--tag", p.tag.as_deref());
        if let Some(pid) = p.pid {
            cmd.extend(["--pid".to_owned(), pid.to_string()]);
        }
        if p.no_advance.unwrap_or(false) {
            cmd.push("--no-advance".to_owned());
        }
    }
    ssh_exec_argv(reg, &target, &cmd, 25.0).await
}

pub(crate) async fn handle_sqlite(reg: &DeviceRegistry, p: SqliteInspectParams) -> Value {
    let target = p.target.to_override();
    let mode = p.mode.unwrap_or_else(|| "query".to_owned());
    let mut cmd = vec![
        "strata-debug".to_owned(),
        "system".to_owned(),
        "sqlite".to_owned(),
        mode,
        p.db_path,
    ];
    push_opt(&mut cmd, "--sql", p.sql.as_deref());
    if let Some(l) = p.limit {
        cmd.extend(["--limit".to_owned(), l.to_string()]);
    }
    ssh_exec_argv(reg, &target, &cmd, 25.0).await
}

pub(crate) async fn handle_system(reg: &DeviceRegistry, p: SystemInspectParams) -> Value {
    let target = p.target.to_override();
    let mut cmd = vec!["strata-debug".to_owned(), "system".to_owned()];
    match p.mode.as_str() {
        "ksu_modules" => cmd.extend(["ksu".to_owned(), "modules".to_owned()]),
        "inputs" => cmd.extend(["ksu".to_owned(), "inputs".to_owned()]),
        "sysfs" => {
            let Some(path) = p.path.filter(|s| !s.trim().is_empty()) else {
                return json!({"ok": false, "error": "path is required for system_inspect(mode='sysfs')"});
            };
            cmd.extend(["ksu".to_owned(), "sysfs".to_owned(), path]);
        }
        "lsposed_modules" => {
            cmd.extend(["lsposed".to_owned(), "modules".to_owned()]);
            push_opt(&mut cmd, "--module", p.module.as_deref());
        }
        "lsposed_logs" => {
            cmd.extend(["lsposed".to_owned(), "logs".to_owned()]);
            push_opt(&mut cmd, "--module", p.module.as_deref());
            push_opt(&mut cmd, "--grep", p.grep.as_deref());
            if let Some(l) = p.limit {
                cmd.extend(["--limit".to_owned(), l.to_string()]);
            }
        }
        "pstore" => {
            cmd.push("pstore".to_owned());
            push_opt(&mut cmd, "--dir", p.dir.as_deref());
        }
        "dmesg" => {
            cmd.push("dmesg".to_owned());
            if let Some(l) = p.limit {
                cmd.extend(["--limit".to_owned(), l.to_string()]);
            }
            push_opt(&mut cmd, "--filter", p.filter.as_deref());
        }
        "tracing" => {
            cmd.push("tracing".to_owned());
            push_opt(&mut cmd, "--dir", p.dir.as_deref());
            if let Some(l) = p.limit {
                cmd.extend(["--limit".to_owned(), l.to_string()]);
            }
        }
        other => {
            return json!({"ok": false, "error": format!("unsupported system_inspect mode '{other}'")});
        }
    }
    ssh_exec_argv(reg, &target, &cmd, 25.0).await
}

pub(crate) async fn handle_kernel_probe(reg: &DeviceRegistry, p: KernelProbeParams) -> Value {
    let target = p.target.to_override();
    let dur = p.duration_sec.unwrap_or(5).clamp(1, 30);
    let cmd = vec![
        "strata-debug".to_owned(),
        "system".to_owned(),
        "kernel-probe".to_owned(),
        p.preset,
        "--duration".to_owned(),
        dur.to_string(),
    ];
    ssh_exec_argv_with_req(
        reg,
        &target,
        &cmd,
        (dur as f64) + 15.0,
        RetryPolicy::Idempotent,
        RouteRequirement::ContainerOnly,
        "kernel_probe",
    )
    .await
}
