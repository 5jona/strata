//! Streaming SSH file transfer (`push` / `pull`), dynamic port refresh, `adb-host` fallback, and port forward.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use super::adb_forward::{ensure_adb_forward, invalidate_adb_forward, shell_quote};
use super::adb_transfer::{TRANSFER_TIMEOUT, adb_host_pull, adb_host_push, temp_local_path};
use super::container_probe::{build_container_required_error, probe_containers_via_adb};
use super::routing::{TargetOverride, TransportRoute, build_routes};
use crate::devices::DeviceRegistry;

async fn try_ssh_push(
    route: &TransportRoute,
    local_path: &str,
    remote_cmd: &str,
) -> Result<u64, String> {
    if route.kind == "adb-ssh"
        && let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port)
    {
        ensure_adb_forward(ser, lp, route.remote_ssh_port).await?;
    }
    let mut file = File::open(local_path)
        .await
        .map_err(|e| format!("open '{local_path}' failed: {e}"))?;

    let mut args = route.base_ssh_args();
    args.push(remote_cmd.to_owned());
    let mut child = Command::new("ssh")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn ssh failed: {e}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "ssh stdin unavailable".to_owned())?;
    let mut stderr_pipe = child.stderr.take();

    let res = tokio::time::timeout(TRANSFER_TIMEOUT, async {
        let copy_fut = async {
            let copied = tokio::io::copy(&mut file, &mut stdin).await;
            let _ = stdin.shutdown().await;
            drop(stdin);
            copied
        };
        let err_fut = async {
            let mut buf = Vec::new();
            if let Some(ref mut se) = stderr_pipe {
                let _ = se.read_to_end(&mut buf).await;
            }
            buf
        };
        let (copied, err_bytes) = tokio::join!(copy_fut, err_fut);
        let status = child
            .wait()
            .await
            .map_err(|e| format!("wait ssh failed: {e}"))?;
        let bytes = copied.map_err(|e| format!("stream push failed: {e}"))?;
        if status.success() {
            Ok(bytes)
        } else {
            Err(String::from_utf8_lossy(&err_bytes).trim().to_owned())
        }
    })
    .await
    .map_err(|_| "ssh push timed out".to_owned())?;

    if res.is_err()
        && route.kind == "adb-ssh"
        && let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port)
    {
        invalidate_adb_forward(ser, lp);
    }
    res
}

async fn try_ssh_pull(
    route: &TransportRoute,
    remote_cmd: &str,
    local_path: &str,
) -> Result<u64, String> {
    if route.kind == "adb-ssh"
        && let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port)
    {
        ensure_adb_forward(ser, lp, route.remote_ssh_port).await?;
    }
    let dst = Path::new(local_path);
    if let Some(parent) = dst.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let tmp = temp_local_path(dst);
    let mut out_file = File::create(&tmp)
        .await
        .map_err(|e| format!("create '{local_path}' temp file failed: {e}"))?;

    let mut args = route.base_ssh_args();
    args.push(remote_cmd.to_owned());
    let mut child = match Command::new("ssh")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(format!("spawn ssh failed: {e}"));
        }
    };

    let Some(mut stdout) = child.stdout.take() else {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err("ssh stdout unavailable".to_owned());
    };
    let mut stderr_pipe = child.stderr.take();

    let res = tokio::time::timeout(TRANSFER_TIMEOUT, async {
        let copy_fut = tokio::io::copy(&mut stdout, &mut out_file);
        let err_fut = async {
            let mut buf = Vec::new();
            if let Some(ref mut se) = stderr_pipe {
                let _ = se.read_to_end(&mut buf).await;
            }
            buf
        };
        let (copied, err_bytes) = tokio::join!(copy_fut, err_fut);
        let status = child
            .wait()
            .await
            .map_err(|e| format!("wait ssh failed: {e}"))?;
        let bytes = copied.map_err(|e| format!("stream pull failed: {e}"))?;
        out_file
            .flush()
            .await
            .map_err(|e| format!("flush pull file failed: {e}"))?;
        drop(out_file);
        if status.success() {
            tokio::fs::rename(&tmp, dst)
                .await
                .map_err(|e| format!("rename to '{local_path}' failed: {e}"))?;
            Ok(bytes)
        } else {
            Err(String::from_utf8_lossy(&err_bytes).trim().to_owned())
        }
    })
    .await
    .map_err(|_| "ssh pull timed out".to_owned())
    .and_then(|r| r);

    if res.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
        if route.kind == "adb-ssh"
            && let (Some(ser), Some(lp)) = (route.adb_serial.as_deref(), route.local_port)
        {
            invalidate_adb_forward(ser, lp);
        }
    }
    res
}

/// Pushes a local file to container or Android host (`host_scope=true`) using streaming I/O.
pub(crate) async fn transfer_push(
    registry: &DeviceRegistry,
    target: &TargetOverride,
    local_path: &str,
    remote_path: &str,
    host_scope: bool,
) -> Value {
    if !Path::new(local_path).is_file() {
        return json!({"ok": false, "error": format!("local file not found: '{local_path}'")});
    }
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

    for route in routes.iter().filter(|r| r.kind != "adb-host") {
        if let Ok(bytes) = try_ssh_push(route, local_path, &remote_cmd).await {
            return json!({
                "ok": true, "direction": "push", "localPath": local_path,
                "remotePath": remote_path, "bytes": bytes, "route": route.kind,
            });
        }
    }

    if let Some(serial) = routes
        .iter()
        .find_map(|r| (r.kind == "adb-host").then_some(r.adb_serial.as_deref()))
        .flatten()
    {
        let probe = probe_containers_via_adb(serial, false).await;
        if let Some(port) = probe.active_ssh_port
            && let Some(adb_ssh) = routes.iter().find(|r| r.kind == "adb-ssh")
            && adb_ssh.remote_ssh_port != port
        {
            if let Some(lp) = adb_ssh.local_port {
                invalidate_adb_forward(serial, lp);
            }
            let refreshed = adb_ssh.with_remote_port(port);
            if let Ok(bytes) = try_ssh_push(&refreshed, local_path, &remote_cmd).await {
                return json!({
                    "ok": true, "direction": "push", "localPath": local_path,
                    "remotePath": remote_path, "bytes": bytes, "route": refreshed.kind,
                });
            }
        }
        if !host_scope {
            return build_container_required_error("file_transfer(hostScope=false)", &probe);
        }
        let reason = probe.degraded_reason.unwrap_or_else(|| {
            "SSH/container transport unavailable; degraded to adb-host".to_owned()
        });
        return match adb_host_push(serial, local_path, remote_path).await {
            Ok(bytes) => json!({
                "ok": true, "direction": "push", "localPath": local_path,
                "remotePath": remote_path, "bytes": bytes, "route": "adb-host",
                "degradedReason": reason,
            }),
            Err(e) => json!({
                "ok": false, "direction": "push", "localPath": local_path,
                "remotePath": remote_path, "route": "adb-host", "degradedReason": reason, "error": e,
            }),
        };
    }

    json!({"ok": false, "direction": "push", "localPath": local_path, "remotePath": remote_path, "error": "SSH push failed on all routes"})
}

/// Pulls a remote file from container or Android host (`host_scope=true`) using streaming I/O.
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

    for route in routes.iter().filter(|r| r.kind != "adb-host") {
        if let Ok(bytes) = try_ssh_pull(route, &remote_cmd, local_path).await {
            return json!({
                "ok": true, "direction": "pull", "remotePath": remote_path,
                "localPath": local_path, "bytes": bytes, "route": route.kind,
            });
        }
    }

    if let Some(serial) = routes
        .iter()
        .find_map(|r| (r.kind == "adb-host").then_some(r.adb_serial.as_deref()))
        .flatten()
    {
        let probe = probe_containers_via_adb(serial, false).await;
        if let Some(port) = probe.active_ssh_port
            && let Some(adb_ssh) = routes.iter().find(|r| r.kind == "adb-ssh")
            && adb_ssh.remote_ssh_port != port
        {
            if let Some(lp) = adb_ssh.local_port {
                invalidate_adb_forward(serial, lp);
            }
            let refreshed = adb_ssh.with_remote_port(port);
            if let Ok(bytes) = try_ssh_pull(&refreshed, &remote_cmd, local_path).await {
                return json!({
                    "ok": true, "direction": "pull", "remotePath": remote_path,
                    "localPath": local_path, "bytes": bytes, "route": refreshed.kind,
                });
            }
        }
        if !host_scope {
            return build_container_required_error("file_transfer(hostScope=false)", &probe);
        }
        let reason = probe.degraded_reason.unwrap_or_else(|| {
            "SSH/container transport unavailable; degraded to adb-host".to_owned()
        });
        return match adb_host_pull(serial, remote_path, local_path).await {
            Ok(bytes) => json!({
                "ok": true, "direction": "pull", "remotePath": remote_path,
                "localPath": local_path, "bytes": bytes, "route": "adb-host",
                "degradedReason": reason,
            }),
            Err(e) => json!({
                "ok": false, "direction": "pull", "remotePath": remote_path,
                "localPath": local_path, "route": "adb-host", "degradedReason": reason, "error": e,
            }),
        };
    }

    json!({"ok": false, "direction": "pull", "remotePath": remote_path, "localPath": local_path, "error": "SSH pull failed on all routes"})
}

/// Opens a background SSH local port forward (or `adb forward` on `adb-host`).
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
    for route in routes.iter().filter(|r| r.kind != "adb-host") {
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
    if let Some(serial) = routes
        .iter()
        .find_map(|r| (r.kind == "adb-host").then_some(r.adb_serial.as_deref()))
        .flatten()
        && ensure_adb_forward(serial, local_port, remote_port)
            .await
            .is_ok()
    {
        return json!({"ok": true, "localPort": local_port, "remotePort": remote_port, "route": "adb-host"});
    }
    json!({"ok": false, "error": "failed to establish local port forward"})
}
