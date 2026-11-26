//! Available version listing (`tools.versions`) and version switching/pinning (`tools.use`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use strata_protocol::rpc::{
    TaskRunResult, ToolManagerKind, ToolsUseParams, ToolsVersionsParams, ToolsVersionsResult,
};
use strata_protocol::tools::validate_package_spec;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::ToolsManager;
use crate::tools::actions::launch_locked_task;
use crate::tools::discovery::resolve_live_tool_bins;
use crate::tools::exec::{load_container_tool_env_from_mount, run_container_command_bounded};
use crate::tools::versions_parse::{
    build_use_command, parse_mise_installed_versions, parse_mise_ls_remote_json,
    parse_uv_python_versions_json, validate_safe_container_path, validate_version_token,
};

pub(super) async fn query_versions(
    tools: &ToolsManager,
    params: ToolsVersionsParams,
) -> DaemonResult<ToolsVersionsResult> {
    let validated_name = validate_package_spec(params.manager, &params.name, true)
        .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
    let (container, ns_pid, cfg) = tools
        .manager
        .running_target(params.container.as_deref())
        .await?;
    let mount_dir = PathBuf::from(&cfg.mount_dir);
    let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mount_dir))
        .await
        .map_err(|e| DaemonError::join("tools.versions env join", e))?;

    let bins = resolve_live_tool_bins(tools, &container, ns_pid, &cfg, &env).await;
    let versions = match params.manager {
        ToolManagerKind::Mise => {
            let mise_bin = bins.mise.ok_or_else(|| {
                DaemonError::InvalidState(format!(
                    "mise is not installed in container '{container}'"
                ))
            })?;
            let remote = run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec![
                    mise_bin.clone(),
                    "ls-remote".to_owned(),
                    validated_name.clone(),
                    "--json".to_owned(),
                ],
                env.clone(),
                Duration::from_secs(12),
            )
            .await?;
            if remote.status != 0 {
                return Err(DaemonError::Internal(format!(
                    "mise ls-remote {} failed: {}",
                    validated_name,
                    remote.stderr.lines().next().unwrap_or("unknown error")
                )));
            }
            let installed_map = match run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec![
                    mise_bin,
                    "ls".to_owned(),
                    validated_name.clone(),
                    "--json".to_owned(),
                ],
                env,
                Duration::from_secs(6),
            )
            .await
            {
                Ok(out) if out.status == 0 => parse_mise_installed_versions(&out.stdout),
                _ => BTreeMap::new(),
            };
            parse_mise_ls_remote_json(&remote.stdout, &installed_map, params.include_prerelease)
        }
        ToolManagerKind::Uv => {
            let uv_bin = bins.uv.ok_or_else(|| {
                DaemonError::InvalidState(format!("uv is not installed in container '{container}'"))
            })?;
            let out = run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec![
                    uv_bin,
                    "python".to_owned(),
                    "list".to_owned(),
                    "--output-format".to_owned(),
                    "json".to_owned(),
                    "--all-versions".to_owned(),
                ],
                env,
                Duration::from_secs(10),
            )
            .await?;
            if out.status != 0 {
                return Err(DaemonError::Internal(format!(
                    "uv python list failed: {}",
                    out.stderr.lines().next().unwrap_or("unknown error")
                )));
            }
            parse_uv_python_versions_json(&out.stdout, params.include_prerelease)
        }
        _ => {
            return Err(DaemonError::InvalidParams(
                "tools.versions supports mise and uv managers".to_owned(),
            ));
        }
    };

    Ok(ToolsVersionsResult {
        container,
        manager: params.manager,
        name: validated_name,
        versions,
    })
}

pub(super) async fn use_version(
    tools: &ToolsManager,
    params: ToolsUseParams,
) -> DaemonResult<TaskRunResult> {
    let validated_name = validate_package_spec(params.manager, &params.name, true)
        .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
    let version = validate_version_token(&params.version)?;
    let config_path = match params.config_path.as_deref() {
        Some(raw) if !raw.trim().is_empty() => Some(validate_safe_container_path(raw)?),
        _ => None,
    };

    let (container, ns_pid, cfg) = tools
        .manager
        .running_target(params.container.as_deref())
        .await?;
    let lock_guard = tools.try_acquire_write_lock(&container).await?;
    let mount_dir = PathBuf::from(&cfg.mount_dir);
    let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mount_dir))
        .await
        .map_err(|e| DaemonError::join("tools.use env join", e))?;
    let bins = resolve_live_tool_bins(tools, &container, ns_pid, &cfg, &env).await;

    let command = build_use_command(
        params.manager,
        &validated_name,
        &version,
        params.scope,
        config_path.as_deref(),
        bins.mise.as_deref(),
        bins.uv.as_deref(),
    )?;

    launch_locked_task(tools, container, command, env, "use", lock_guard).await
}
