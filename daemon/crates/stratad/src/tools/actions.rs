//! Streaming mutation task orchestration (`bootstrap`, `install`, `uninstall`, `upgrade`).

use std::path::PathBuf;
use std::sync::Arc;

use strata_protocol::EventEnvelope;
use strata_protocol::rpc::{
    TaskRunParams, TaskRunResult, ToolsBootstrapParams, ToolsBootstrapProfile,
    ToolsConfigSetParams, ToolsInstallParams, ToolsStateChangedPayload, ToolsUninstallParams,
    ToolsUpgradeParams, events,
};
use strata_sys::mount;
use strata_sys::rootfs_io::RootDir;
use tokio::sync::OwnedMutexGuard;

use crate::error::{DaemonError, DaemonResult};
use crate::sideveil_seam::{EventHub, PluginEventSink};
use crate::task_runner::TaskRunOptions;
use crate::tools::ToolsManager;
use crate::tools::action_cmds::{
    build_bootstrap_command, build_install_command, build_uninstall_command, build_upgrade_command,
};
use crate::tools::catalog::profile_packages;
use crate::tools::config::apply_config_update;
use crate::tools::exec::load_container_tool_env_from_mount;

const TOOLS_TASK_TIMEOUT_SECS: u64 = 600;

pub(super) async fn bootstrap(
    tools: &ToolsManager,
    params: ToolsBootstrapParams,
) -> DaemonResult<TaskRunResult> {
    let (container, _, cfg) = tools
        .manager
        .running_target(params.container.as_deref())
        .await?;
    let lock_guard = tools.try_acquire_write_lock(&container).await?;
    let mount_dir = PathBuf::from(&cfg.mount_dir);

    if let Some(preset) = params.mirror_preset {
        let mdir = mount_dir.clone();
        let cid = container.clone();
        tokio::task::spawn_blocking(move || {
            if mount::is_ext4_mounted(&mdir).unwrap_or(false)
                && let Ok(root) = RootDir::open(&mdir)
            {
                apply_config_update(
                    &cid,
                    &root,
                    ToolsConfigSetParams {
                        mirror_preset: Some(preset),
                        ..Default::default()
                    },
                )?;
            }
            Ok::<_, DaemonError>(())
        })
        .await
        .map_err(|e| DaemonError::join("tools.bootstrap mirror preset", e))??;
    }

    let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mount_dir))
        .await
        .map_err(|e| DaemonError::join("tools.bootstrap env", e))?;

    let profile = params.profile.unwrap_or(ToolsBootstrapProfile::Dev);
    let (apt_pkgs, mise_pkgs) = profile_packages(profile);
    let command = build_bootstrap_command(apt_pkgs, mise_pkgs);

    launch_locked_task(tools, container, command, env, "bootstrap", lock_guard).await
}

pub(super) async fn install(
    tools: &ToolsManager,
    params: ToolsInstallParams,
) -> DaemonResult<TaskRunResult> {
    let (container, _, cfg) = tools
        .manager
        .running_target(params.container.as_deref())
        .await?;
    let lock_guard = tools.try_acquire_write_lock(&container).await?;
    let command = build_install_command(params.manager, &params.packages, params.pin)?;
    let mount_dir = PathBuf::from(&cfg.mount_dir);
    let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mount_dir))
        .await
        .map_err(|e| DaemonError::join("tools.install env", e))?;

    launch_locked_task(tools, container, command, env, "install", lock_guard).await
}

pub(super) async fn uninstall(
    tools: &ToolsManager,
    params: ToolsUninstallParams,
) -> DaemonResult<TaskRunResult> {
    let (container, _, cfg) = tools
        .manager
        .running_target(params.container.as_deref())
        .await?;
    let lock_guard = tools.try_acquire_write_lock(&container).await?;
    let command = build_uninstall_command(params.manager, &params.packages)?;
    let mount_dir = PathBuf::from(&cfg.mount_dir);
    let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mount_dir))
        .await
        .map_err(|e| DaemonError::join("tools.uninstall env", e))?;

    launch_locked_task(tools, container, command, env, "uninstall", lock_guard).await
}

pub(super) async fn upgrade(
    tools: &ToolsManager,
    params: ToolsUpgradeParams,
) -> DaemonResult<TaskRunResult> {
    let (container, _, cfg) = tools
        .manager
        .running_target(params.container.as_deref())
        .await?;
    let lock_guard = tools.try_acquire_write_lock(&container).await?;
    let command = build_upgrade_command(params.manager, &params.packages, params.bump)?;
    let mount_dir = PathBuf::from(&cfg.mount_dir);
    let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mount_dir))
        .await
        .map_err(|e| DaemonError::join("tools.upgrade env", e))?;

    launch_locked_task(tools, container, command, env, "upgrade", lock_guard).await
}

async fn launch_locked_task(
    tools: &ToolsManager,
    container: String,
    command: Vec<String>,
    env: std::collections::BTreeMap<String, String>,
    action: &'static str,
    lock_guard: OwnedMutexGuard<()>,
) -> DaemonResult<TaskRunResult> {
    let hub = Arc::clone(&tools.event_hub);
    let cid = container.clone();
    let on_exit = Box::new(move |exit_code: i32, timed_out: bool| {
        drop(lock_guard);
        let reason = if exit_code == 0 && !timed_out {
            format!("{action}_completed")
        } else {
            format!("{action}_failed")
        };
        emit_tools_event(&hub, cid, &reason, None);
    });

    let res = tools
        .task_runner
        .run_task_with_options(
            TaskRunParams {
                container: Some(container.clone()),
                command,
                cwd: None,
                timeout_secs: Some(TOOLS_TASK_TIMEOUT_SECS),
            },
            TaskRunOptions {
                env,
                on_exit: Some(on_exit),
            },
        )
        .await?;

    emit_tools_event(
        &tools.event_hub,
        container,
        &format!("{action}_started"),
        Some(res.task_id.clone()),
    );
    Ok(res)
}

fn emit_tools_event(hub: &EventHub, container: String, reason: &str, task_id: Option<String>) {
    let payload = serde_json::to_value(ToolsStateChangedPayload {
        container,
        reason: reason.to_owned(),
        task_id,
    })
    .ok();
    hub.emit(EventEnvelope::new(events::TOOLS_STATE_CHANGED, payload));
}
