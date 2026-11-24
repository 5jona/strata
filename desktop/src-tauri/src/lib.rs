//! Tauri v2 desktop entry point wiring managed state, commands, and system tray.

mod commands;
mod tray;

use std::path::PathBuf;
use std::sync::Mutex;

use strata_desktop_core::adb::{AdbManager, SystemCommandRunner};
use strata_desktop_core::devices::DeviceStore;
use tauri::Manager;

/// Shared Tauri application state guarding ADB tunnel manager and device persistence.
#[derive(Debug)]
pub(crate) struct AppState {
    pub(crate) adb: Mutex<AdbManager<SystemCommandRunner>>,
    pub(crate) store: Mutex<DeviceStore>,
}

/// Initializes and runs the Strata Desktop Tauri v2 application.
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let store_path = resolve_device_store_path(app);
            app.manage(AppState {
                adb: Mutex::new(AdbManager::system()),
                store: Mutex::new(DeviceStore::new(store_path)),
            });
            tray::setup_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_adb_devices,
            commands::connect_adb_device,
            commands::disconnect_device,
            commands::load_saved_devices,
            commands::save_device_profile,
            commands::delete_device_profile,
            commands::probe_device_endpoint,
            commands::open_native_terminal,
            commands::open_vscode_remote,
        ]);

    if let Err(err) = builder.run(tauri::generate_context!()) {
        eprintln!("strata-desktop exited with error: {err}");
    }
}

fn resolve_device_store_path(app: &tauri::App) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("strata-desktop"))
        .join("devices.json")
}
