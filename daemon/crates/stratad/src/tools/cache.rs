//! Bounded container tool cache cleanup (`tools.cacheClean`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use strata_protocol::constants::{CONTAINER_MISE_CACHE_DIR, CONTAINER_UV_CACHE_DIR};
use strata_protocol::rpc::{ToolsCacheCleanParams, ToolsCacheCleanResult};
use strata_sys::mount;
use strata_sys::rootfs_io::RootDir;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::ToolsManager;
use crate::tools::discovery::resolve_live_tool_bins;
use crate::tools::exec::{load_container_tool_env_from_mount, run_container_command_bounded};

pub(super) async fn clean_cache(
    tools: &ToolsManager,
    params: ToolsCacheCleanParams,
) -> DaemonResult<ToolsCacheCleanResult> {
    let (container, entry, _) = tools
        .manager
        .resolve_container(params.container.as_deref())
        .await?;
    let _guard = tools.try_acquire_write_lock(&container).await?;
    let mount_dir = PathBuf::from(&entry.mount_dir);
    let clean_mise = params.clean_mise;
    let clean_uv = params.clean_uv;
    let clean_apt = params.clean_apt;

    let before_bytes = tokio::task::spawn_blocking({
        let mdir = mount_dir.clone();
        move || measure_cleanable_bytes(&mdir)
    })
    .await
    .map_err(|e| DaemonError::join("tools.cacheClean before join", e))??;

    tokio::task::spawn_blocking({
        let mdir = mount_dir.clone();
        move || {
            let Ok(root) = RootDir::open(&mdir) else {
                return;
            };
            if clean_mise {
                let _ = root.clear_dir_contents(Path::new("root/.cache/mise"));
                let _ = root.clear_dir_contents(Path::new(
                    CONTAINER_MISE_CACHE_DIR.trim_start_matches('/'),
                ));
            }
            if clean_uv {
                let _ = root.clear_dir_contents(Path::new("root/.cache/uv"));
                let _ = root
                    .clear_dir_contents(Path::new(CONTAINER_UV_CACHE_DIR.trim_start_matches('/')));
            }
        }
    })
    .await
    .map_err(|e| DaemonError::join("tools.cacheClean clear join", e))?;

    if let Ok((_, ns_pid, cfg)) = tools.manager.running_target(Some(&container)).await {
        let mdir = PathBuf::from(&cfg.mount_dir);
        let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mdir))
            .await
            .unwrap_or_default();
        let bins = resolve_live_tool_bins(tools, &container, ns_pid, &cfg, &env).await;
        if clean_apt {
            let _ = run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec!["apt-get".to_owned(), "clean".to_owned()],
                BTreeMap::new(),
                Duration::from_secs(15),
            )
            .await;
        }
        if params.prune_unused
            && let Some(mise_bin) = bins.mise
        {
            let _ = run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec![
                    mise_bin,
                    "prune".to_owned(),
                    "--tools".to_owned(),
                    "-y".to_owned(),
                ],
                env,
                Duration::from_secs(15),
            )
            .await;
        }
    }

    let after_bytes = tokio::task::spawn_blocking(move || measure_cleanable_bytes(&mount_dir))
        .await
        .map_err(|e| DaemonError::join("tools.cacheClean after join", e))??;

    tools.emit_state_changed(container.clone(), "cache_cleaned", None);
    Ok(ToolsCacheCleanResult {
        container,
        before_bytes,
        after_bytes,
        freed_bytes: before_bytes.saturating_sub(after_bytes),
    })
}

fn measure_cleanable_bytes(mount_dir: &Path) -> DaemonResult<u64> {
    if !mount::is_ext4_mounted(mount_dir).unwrap_or(false) {
        return Err(DaemonError::ContainerNotRunning(
            "container must be running to clean tool caches".to_owned(),
        ));
    }
    let root = RootDir::open(mount_dir).map_err(strata_runtime::RuntimeError::from_sys)?;
    Ok(root.dir_size_bytes(Path::new("root/.cache/mise"))
        + root.dir_size_bytes(Path::new("root/.cache/uv"))
        + root.dir_size_bytes(Path::new(CONTAINER_MISE_CACHE_DIR.trim_start_matches('/')))
        + root.dir_size_bytes(Path::new(CONTAINER_UV_CACHE_DIR.trim_start_matches('/')))
        + root.dir_size_bytes(Path::new("var/cache/apt/archives")))
}
