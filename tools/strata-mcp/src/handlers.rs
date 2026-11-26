//! Core device, shell, file transfer, and app lifecycle MCP handlers.

pub(crate) mod extra;
mod gesture_arg;
pub(crate) mod inspect;
pub(crate) mod perfetto_reload;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::devices::DeviceRegistry;
use crate::params::{AppManageParams, DeviceExecParams, DeviceManageParams, FileTransferParams};
use crate::transport::{
    TargetOverride, ssh_exec_argv, ssh_exec_argv_once, transfer_pull, transfer_push,
};

pub(crate) use inspect::{
    handle_kernel_probe, handle_logcat, handle_sqlite, handle_system, handle_ui_interact,
};

fn push_opt(cmd: &mut Vec<String>, flag: &str, val: Option<&str>) {
    if let Some(v) = val.map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push(flag.to_owned());
        cmd.push(v.to_owned());
    }
}

/// Handles `device_manage` (`list` | `select` | `health`).
pub(crate) async fn handle_device_manage(reg: &DeviceRegistry, p: DeviceManageParams) -> Value {
    match p.action.as_str() {
        "list" => {
            let devs = reg.discover(true).await;
            let sel = reg.selected_id().await;
            let items: Vec<Value> = devs
                .iter()
                .map(|d| d.to_public_json(sel.as_deref()))
                .collect();
            json!({"ok": true, "selected": sel, "count": items.len(), "devices": items})
        }
        "select" => {
            if p.clear.unwrap_or(false)
                || p.device.as_deref().map(str::trim).unwrap_or("").is_empty()
            {
                let _ = reg.select(None).await;
                return json!({"ok": true, "selected": Value::Null, "cleared": true});
            }
            match reg.select(p.device.as_deref()).await {
                Ok(Some(rec)) => {
                    json!({"ok": true, "selected": rec.id, "device": rec.to_public_json(Some(&rec.id))})
                }
                Ok(None) => json!({"ok": true, "selected": Value::Null}),
                Err(e) => json!({"ok": false, "error": e}),
            }
        }
        "health" => {
            let target = TargetOverride {
                device: p.device,
                host: p.host,
                port: p.port,
            };
            let pkg = p.package.unwrap_or_else(|| "auto".to_owned());
            let cmd = vec![
                "strata-debug".to_owned(),
                "health".to_owned(),
                "--package".to_owned(),
                pkg,
            ];
            ssh_exec_argv(reg, &target, &cmd, 25.0).await
        }
        other => {
            json!({"ok": false, "error": format!("unsupported device_manage action '{other}'; use list|select|health")})
        }
    }
}

/// Handles `device_exec` (`host` | `container` shell execution).
pub(crate) async fn handle_device_exec(reg: &DeviceRegistry, p: DeviceExecParams) -> Value {
    let target = p.target.to_override();
    let scope = p.scope.unwrap_or_else(|| "host".to_owned());
    let timeout = p.timeout_sec.unwrap_or(30.0).clamp(1.0, 90.0);
    let cmd = vec![
        "strata-debug".to_owned(),
        "exec".to_owned(),
        "--scope".to_owned(),
        scope,
        "--timeout".to_owned(),
        timeout.to_string(),
        p.command,
    ];
    ssh_exec_argv_once(reg, &target, &cmd, timeout + 10.0).await
}

/// Handles `file_transfer` (`push` | `pull`).
pub(crate) async fn handle_file_transfer(reg: &DeviceRegistry, p: FileTransferParams) -> Value {
    let target = p.target.to_override();
    let host_scope = p.host_scope.unwrap_or(false);
    match p.direction.as_str() {
        "push" => transfer_push(reg, &target, &p.local_path, &p.remote_path, host_scope).await,
        "pull" => transfer_pull(reg, &target, &p.remote_path, &p.local_path, host_scope).await,
        other => {
            json!({"ok": false, "error": format!("unsupported direction '{other}'; use push|pull")})
        }
    }
}

/// Handles `app_manage` (`inspect` | `start` | `stop` | `clear` | `install`).
pub(crate) async fn handle_app_manage(reg: &DeviceRegistry, p: AppManageParams) -> Value {
    let target = p.target.to_override();
    match p.action.as_str() {
        "inspect" => {
            let mut cmd = vec![
                "strata-debug".to_owned(),
                "app".to_owned(),
                "inspect".to_owned(),
                p.package.unwrap_or_else(|| "auto".to_owned()),
            ];
            if p.top_only.unwrap_or(false) {
                cmd.push("--top-only".to_owned());
            }
            ssh_exec_argv(reg, &target, &cmd, 25.0).await
        }
        "start" | "stop" | "clear" => {
            let Some(pkg) = p.package.filter(|s| !s.trim().is_empty()) else {
                return json!({"ok": false, "error": "package is required for start/stop/clear"});
            };
            let mut cmd = vec![
                "strata-debug".to_owned(),
                "app".to_owned(),
                "control".to_owned(),
                p.action,
                pkg,
            ];
            push_opt(&mut cmd, "--activity", p.activity.as_deref());
            push_opt(&mut cmd, "--intent-action", p.intent_action.as_deref());
            push_opt(&mut cmd, "--data-uri", p.data_uri.as_deref());
            ssh_exec_argv_once(reg, &target, &cmd, 25.0).await
        }
        "install" => handle_apk_install(reg, &target, p).await,
        other => json!({"ok": false, "error": format!("unsupported app_manage action '{other}'")}),
    }
}

async fn handle_apk_install(
    reg: &DeviceRegistry,
    target: &TargetOverride,
    p: AppManageParams,
) -> Value {
    let mut cleanup_path: Option<String> = None;
    let remote = if let Some(lp) = p.local_path.as_deref().filter(|s| !s.trim().is_empty()) {
        if !Path::new(lp).is_file() {
            return json!({"ok": false, "error": format!("APK not found: {lp}")});
        }
        let base: String = Path::new(lp)
            .file_name()
            .map(|n| {
                n.to_string_lossy()
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect()
            })
            .unwrap_or_else(|| "app.apk".to_owned());
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let rp = format!(
            "/data/local/tmp/strata_apk_{}_{ts}_{base}",
            std::process::id()
        );
        let push = transfer_push(reg, target, lp, &rp, true).await;
        if push["ok"] != json!(true) {
            return json!({"ok": false, "stage": "push", "error": push["error"]});
        }
        cleanup_path = Some(rp.clone());
        rp
    } else if let Some(rp) = p.remote_path.filter(|s| !s.trim().is_empty()) {
        rp
    } else {
        return json!({"ok": false, "error": "localPath or remotePath is required for action='install'"});
    };

    let mut cmd = vec![
        "strata-debug".to_owned(),
        "app".to_owned(),
        "install".to_owned(),
        remote,
    ];
    if p.grant_permissions == Some(false) {
        cmd.push("--no-grant".to_owned());
    }
    if p.allow_downgrade.unwrap_or(false) {
        cmd.push("--downgrade".to_owned());
    }
    if p.replace == Some(false) {
        cmd.push("--no-replace".to_owned());
    }
    let res = ssh_exec_argv_once(reg, target, &cmd, 90.0).await;
    if let Some(rp) = cleanup_path {
        let rm = [
            "host-exec".to_owned(),
            "/system/bin/rm".to_owned(),
            "-f".to_owned(),
            rp,
        ];
        let _ = ssh_exec_argv(reg, target, &rm, 15.0).await;
    }
    res
}
