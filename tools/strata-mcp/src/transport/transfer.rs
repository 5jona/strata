//! Binary pipe file transfer (`push` / `pull`) and SSH local port forwarding over ControlMaster.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::routing::{TargetOverride, build_routes, ensure_adb_forward, shell_quote};
use crate::devices::DeviceRegistry;

/// Pushes a local file to container or Android host (`host_scope=true`) over SSH binary pipe.
pub(crate) async fn transfer_push(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    local_path: &str,
    remote_path: &str,
    host_scope: bool,
) -> Value {
    let bytes = match tokio::fs::read(local_path).await {
        Ok(b) => b,
        Err(e) => return json!({"ok": false, "error": format!("read '{local_path}' failed: {e}")}),
    };
    let routes = match build_routes(registry, target).await {
        Ok(r) => r,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let q = shell_quote(remote_path);
    let remote_cmd = if host_scope {
        format!("host-exec /system/bin/sh -c 'cat > {q}'")
    } else {
        format!("cat > {q}")
    };

    for route in &routes {
        if route.kind == "adb-ssh"
            && let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port)
            && ensure_adb_forward(ser, lp, route.remote_ssh_port)
                .await
                .is_err()
        {
            continue;
        }
        let mut args = route.base_ssh_args();
        args.push(remote_cmd.clone());
        let mut cmd = Command::new("ssh");
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let Ok(mut child) = cmd.spawn() else { continue };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&bytes).await;
            let _ = stdin.shutdown().await;
        }
        if let Ok(Ok(out)) =
            tokio::time::timeout(Duration::from_secs(90), child.wait_with_output()).await
            && out.status.success()
        {
            return json!({
                "ok": true, "direction": "push", "localPath": local_path,
                "remotePath": remote_path, "bytes": bytes.len(), "route": route.kind,
            });
        }
    }
    json!({"ok": false, "direction": "push", "localPath": local_path, "remotePath": remote_path, "error": "SSH push failed on all routes"})
}

/// Pulls a remote file from container or Android host (`host_scope=true`) into `local_path`.
pub(crate) async fn transfer_pull(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    remote_path: &str,
    local_path: &str,
    host_scope: bool,
) -> Value {
    let routes = match build_routes(registry, target).await {
        Ok(r) => r,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let q = shell_quote(remote_path);
    let remote_cmd = if host_scope {
        format!("host-exec /system/bin/cat {q}")
    } else {
        format!("cat {q}")
    };

    for route in &routes {
        if route.kind == "adb-ssh"
            && let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port)
            && ensure_adb_forward(ser, lp, route.remote_ssh_port)
                .await
                .is_err()
        {
            continue;
        }
        let mut args = route.base_ssh_args();
        args.push(remote_cmd.clone());
        let out = tokio::time::timeout(
            Duration::from_secs(90),
            Command::new("ssh")
                .args(&args)
                .stdin(Stdio::null())
                .output(),
        )
        .await;
        if let Ok(Ok(res)) = out
            && res.status.success()
        {
            let dst = Path::new(local_path);
            if let Some(parent) = dst.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if let Err(e) = tokio::fs::write(dst, &res.stdout).await {
                return json!({"ok": false, "error": format!("write '{local_path}' failed: {e}")});
            }
            return json!({
                "ok": true, "direction": "pull", "remotePath": remote_path,
                "localPath": local_path, "bytes": res.stdout.len(), "route": route.kind,
            });
        }
    }
    json!({"ok": false, "direction": "pull", "remotePath": remote_path, "localPath": local_path, "error": "SSH pull failed on all routes"})
}

/// Opens a background SSH local port forward (`-L 127.0.0.1:<local>:127.0.0.1:<remote>`).
pub(crate) async fn open_ssh_port_forward(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    local_port: u16,
    remote_port: u16,
) -> Value {
    let routes = match build_routes(registry, target).await {
        Ok(r) => r,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    for route in &routes {
        if route.kind == "adb-ssh"
            && let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port)
            && ensure_adb_forward(ser, lp, route.remote_ssh_port)
                .await
                .is_err()
        {
            continue;
        }
        let mut args = vec![
            "-f".to_owned(),
            "-N".to_owned(),
            "-o".to_owned(),
            "ExitOnForwardFailure=yes".to_owned(),
            "-L".to_owned(),
            format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}"),
        ];
        args.extend(route.base_ssh_args());
        if let Ok(Ok(out)) = tokio::time::timeout(
            Duration::from_secs(15),
            Command::new("ssh").args(&args).output(),
        )
        .await
            && out.status.success()
        {
            return json!({"ok": true, "localPort": local_port, "remotePort": remote_port, "route": route.kind});
        }
    }
    json!({"ok": false, "error": "failed to establish SSH local port forward"})
}
