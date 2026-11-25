//! Thin Tauri IPC command handlers delegating to `strata-desktop-core`.

use std::process::Command;
use std::time::Duration;

use strata_desktop_core::adb::{AdbDevice, AdbTunnel, PortForwardEntry};
use strata_desktop_core::devices::{DeviceProfile, DeviceStoreFile};
use strata_desktop_core::launcher::{
    NativeTerminalKind, SshTarget, build_terminal_launch_command, build_vscode_remote_uri,
    default_terminal_candidates,
};
use strata_desktop_core::probe::{EndpointProbeResult, probe_stratad_endpoint};
use tauri::State;

use crate::AppState;

/// Lists attached USB and wireless Android devices via `adb devices -l`.
#[tauri::command]
pub(crate) fn list_adb_devices(state: State<'_, AppState>) -> Result<Vec<AdbDevice>, String> {
    let guard = state
        .adb
        .lock()
        .map_err(|_| "ADB state mutex poisoned".to_owned())?;
    guard.list_devices().map_err(|e| e.to_string())
}

/// Establishes an `adb forward` tunnel for `serial` and attempts root Bearer token fetch.
#[tauri::command]
pub(crate) fn connect_adb_device(
    state: State<'_, AppState>,
    serial: String,
    remote_port: Option<u16>,
) -> Result<AdbTunnel, String> {
    let mut guard = state
        .adb
        .lock()
        .map_err(|_| "ADB state mutex poisoned".to_owned())?;
    guard
        .connect_device(&serial, remote_port, None)
        .map_err(|e| e.to_string())
}

/// Removes an active `adb forward` tunnel for `serial`.
#[tauri::command]
pub(crate) fn disconnect_device(
    state: State<'_, AppState>,
    serial: String,
) -> Result<bool, String> {
    let mut guard = state
        .adb
        .lock()
        .map_err(|_| "ADB state mutex poisoned".to_owned())?;
    guard.disconnect_device(&serial).map_err(|e| e.to_string())
}

/// Forwards an arbitrary container or device TCP port to `127.0.0.1` over ADB.
#[tauri::command]
pub(crate) fn forward_device_port(
    state: State<'_, AppState>,
    serial: String,
    remote_port: u16,
    local_port: Option<u16>,
) -> Result<PortForwardEntry, String> {
    let mut guard = state
        .adb
        .lock()
        .map_err(|_| "ADB state mutex poisoned".to_owned())?;
    guard
        .forward_device_port(&serial, remote_port, local_port)
        .map_err(|e| e.to_string())
}

/// Lists active TCP `adb forward` entries, optionally filtered by device serial.
#[tauri::command]
pub(crate) fn list_port_forwards(
    state: State<'_, AppState>,
    serial: Option<String>,
) -> Result<Vec<PortForwardEntry>, String> {
    let guard = state
        .adb
        .lock()
        .map_err(|_| "ADB state mutex poisoned".to_owned())?;
    guard
        .list_port_forwards(serial.as_deref())
        .map_err(|e| e.to_string())
}

/// Removes an active `adb forward` rule on `serial` for `tcp:<local_port>`.
#[tauri::command]
pub(crate) fn remove_port_forward(
    state: State<'_, AppState>,
    serial: String,
    local_port: u16,
) -> Result<bool, String> {
    let mut guard = state
        .adb
        .lock()
        .map_err(|_| "ADB state mutex poisoned".to_owned())?;
    guard
        .remove_port_forward(&serial, local_port)
        .map_err(|e| e.to_string())
}

/// Loads saved multi-device desktop profiles from local config storage.
#[tauri::command]
pub(crate) fn load_saved_devices(state: State<'_, AppState>) -> Result<DeviceStoreFile, String> {
    let guard = state
        .store
        .lock()
        .map_err(|_| "Device store mutex poisoned".to_owned())?;
    guard.load().map_err(|e| e.to_string())
}

/// Persists (inserts or updates) a device profile atomically.
#[tauri::command]
pub(crate) fn save_device_profile(
    state: State<'_, AppState>,
    profile: DeviceProfile,
) -> Result<DeviceStoreFile, String> {
    let guard = state
        .store
        .lock()
        .map_err(|_| "Device store mutex poisoned".to_owned())?;
    guard.upsert(profile).map_err(|e| e.to_string())
}

/// Deletes a saved device profile by ID.
#[tauri::command]
pub(crate) fn delete_device_profile(
    state: State<'_, AppState>,
    id: String,
) -> Result<DeviceStoreFile, String> {
    let guard = state
        .store
        .lock()
        .map_err(|_| "Device store mutex poisoned".to_owned())?;
    let (_, store) = guard.remove(&id).map_err(|e| e.to_string())?;
    Ok(store)
}

/// Probes `GET /api/info` and authenticated `POST /api/rpc` (`daemon.ping`) on a device endpoint.
#[tauri::command]
pub(crate) fn probe_device_endpoint(
    host: String,
    port: u16,
    token: Option<String>,
) -> EndpointProbeResult {
    probe_stratad_endpoint(&host, port, token.as_deref(), Duration::from_secs(3))
}

/// Launches an external native terminal emulator connected via SSH to a container.
#[tauri::command]
pub(crate) fn open_native_terminal(
    target: SshTarget,
    preferred: Option<NativeTerminalKind>,
) -> Result<String, String> {
    let candidates = match preferred {
        Some(kind) => vec![kind],
        None => default_terminal_candidates(),
    };
    let mut last_err = String::from("no supported native terminal found in PATH");
    for kind in candidates {
        let cmd = build_terminal_launch_command(kind, &target).map_err(|e| e.to_string())?;
        match Command::new(&cmd.program).args(&cmd.args).spawn() {
            Ok(_) => return Ok(cmd.program),
            Err(err) => last_err = format!("{}: {err}", cmd.program),
        }
    }
    Err(last_err)
}

/// Opens a container workspace in VS Code Remote-SSH using a validated `vscode://` URI.
#[tauri::command]
pub(crate) fn open_vscode_remote(target: SshTarget) -> Result<String, String> {
    let uri = build_vscode_remote_uri(&target).map_err(|e| e.to_string())?;
    let status = if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", "", &uri]).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(&uri).spawn()
    } else {
        Command::new("xdg-open").arg(&uri).spawn()
    };
    status.map_err(|e| format!("failed to launch VS Code URI: {e}"))?;
    Ok(uri)
}
