//! Local-only status snapshot (`tools.status`) and host filesystem inspection.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use strata_protocol::constants::{
    CONTAINER_LAYOUT_PATH, CONTAINER_LAYOUT_SCHEMA_VERSION, CONTAINER_PROFILE_ENV_PATH,
    CONTAINER_WORKSPACE_DIR,
};
use strata_protocol::rpc::{
    ToolInstallSource, ToolManagerKind, ToolsStatusParams, ToolsStatusResult,
};
use strata_sys::mount;
use strata_sys::rootfs_io::RootDir;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::ToolsManager;
use crate::tools::config::read_config_from_root;
use crate::tools::discovery::{
    HostManagerPaths, apply_probe_health, discover_host_managers, discover_runtime_managers,
    health_from_path,
};
use crate::tools::exec::{
    load_container_tool_env_from_mount, read_root_utf8, run_container_command_bounded,
};
use crate::tools::status_parsers::parse_mise_ls_json;

struct HostFsSnapshot {
    layout_schema_version: Option<u32>,
    layout_compliant: bool,
    mirror_preset: strata_protocol::rpc::ToolMirrorPreset,
    has_github_token: bool,
    cache_bytes: u64,
    installs_bytes: u64,
    paths: HostManagerPaths,
}

impl HostFsSnapshot {
    fn empty() -> Self {
        Self {
            layout_schema_version: None,
            layout_compliant: false,
            mirror_preset: strata_protocol::rpc::ToolMirrorPreset::Official,
            has_github_token: false,
            cache_bytes: 0,
            installs_bytes: 0,
            paths: HostManagerPaths::default(),
        }
    }
}

pub(super) async fn query_status(
    tools: &ToolsManager,
    params: ToolsStatusParams,
) -> DaemonResult<ToolsStatusResult> {
    let (container, entry, _) = tools
        .manager
        .resolve_container(params.container.as_deref())
        .await?;
    let mount_dir = PathBuf::from(&entry.mount_dir);
    let cid = container.clone();

    let fs_snap = tokio::task::spawn_blocking(move || inspect_host_fs(&cid, &mount_dir))
        .await
        .map_err(|e| DaemonError::join("tools.status fs join", e))?;

    let mut mise = health_from_path(fs_snap.paths.mise);
    let mut uv = health_from_path(fs_snap.paths.uv);
    let mut apt = health_from_path(fs_snap.paths.apt);
    let mut host_exec = health_from_path(fs_snap.paths.host_exec);
    let mut strata_debug = health_from_path(fs_snap.paths.strata_debug);
    let mut system_pkg_manager = fs_snap.paths.system_pkg_manager;
    let mut mise_data_dir: Option<String> = None;
    let mut mise_config_paths = fs_snap.paths.config_paths;
    let mut installed_tool_count = 0_usize;

    if let Ok((_, ns_pid, cfg)) = tools.manager.running_target(Some(&container)).await {
        let mdir = PathBuf::from(&cfg.mount_dir);
        let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mdir))
            .await
            .map_err(|e| DaemonError::join("tools.status env join", e))?;

        let disc = discover_runtime_managers(tools, &container, ns_pid, &cfg, &env).await;
        apply_probe_health(&mut mise, &disc.bins, "mise");
        apply_probe_health(&mut uv, &disc.bins, "uv");
        apply_probe_health(&mut apt, &disc.bins, "apt-get");
        apply_probe_health(&mut host_exec, &disc.bins, "host-exec");
        apply_probe_health(&mut strata_debug, &disc.bins, "strata-debug");
        if disc.system_pkg_manager.is_some() {
            system_pkg_manager = disc.system_pkg_manager;
        }
        if disc.mise_data_dir.is_some() {
            mise_data_dir = disc.mise_data_dir;
        }
        if !disc.mise_config_paths.is_empty() {
            mise_config_paths = disc.mise_config_paths;
        }

        if let Some(ref mise_bin) = mise.bin_path
            && let Ok(ls_out) = run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec![
                    mise_bin.clone(),
                    "ls".to_owned(),
                    "--all-sources".to_owned(),
                    "--json".to_owned(),
                ],
                env,
                Duration::from_secs(6),
            )
            .await
            && ls_out.status == 0
        {
            installed_tool_count =
                parse_mise_ls_json(&ls_out.stdout, ToolInstallSource::UserInstalled).len();
        }
    }

    let legacy_strata_mise_detected =
        fs_snap.paths.legacy_mise_installs && mise_data_dir.as_deref() != Some("/opt/strata/mise");
    let mise_config_path = mise_config_paths
        .first()
        .cloned()
        .unwrap_or_else(|| "/root/.config/mise/config.toml".to_owned());
    let system_pkg_manager_name = system_pkg_manager.map(|m| match m {
        ToolManagerKind::Apt => "apt".to_owned(),
        ToolManagerKind::Apk => "apk".to_owned(),
        ToolManagerKind::Pacman => "pacman".to_owned(),
        ToolManagerKind::Dnf => "dnf".to_owned(),
        _ => "system".to_owned(),
    });

    Ok(ToolsStatusResult {
        container,
        workspace_dir: CONTAINER_WORKSPACE_DIR.to_owned(),
        mise_config_path,
        layout_schema_version: fs_snap.layout_schema_version,
        layout_compliant: fs_snap.layout_compliant,
        mirror_preset: fs_snap.mirror_preset,
        has_github_token: fs_snap.has_github_token,
        mise,
        uv,
        apt,
        host_exec,
        strata_debug,
        installed_tool_count,
        outdated_tool_count: None,
        cache_bytes: fs_snap.cache_bytes,
        installs_bytes: fs_snap.installs_bytes,
        mise_data_dir,
        mise_config_paths,
        legacy_strata_mise_detected,
        system_pkg_manager: system_pkg_manager_name,
    })
}

fn inspect_host_fs(container: &str, mount_dir: &Path) -> HostFsSnapshot {
    if !mount::is_ext4_mounted(mount_dir).unwrap_or(false) {
        return HostFsSnapshot::empty();
    }
    let Ok(root) = RootDir::open(mount_dir) else {
        return HostFsSnapshot::empty();
    };
    let layout_schema_version =
        read_root_utf8(&root, CONTAINER_LAYOUT_PATH.trim_start_matches('/'))
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.get("schemaVersion")?.as_u64())
            .and_then(|n| u32::try_from(n).ok());

    let dirs_ok = ["workspace", "etc/strata", "etc/profile.d"]
        .iter()
        .all(|rel| root.is_dir(Path::new(rel)));
    let files_ok = root
        .open_file_ro_nofollow(Path::new(
            CONTAINER_PROFILE_ENV_PATH.trim_start_matches('/'),
        ))
        .is_ok();
    let cfg_res = read_config_from_root(container, &root, None, Vec::new()).ok();
    let paths = discover_host_managers(&root);

    let cache_bytes = root.dir_size_bytes(Path::new("root/.cache/mise"))
        + root.dir_size_bytes(Path::new("root/.cache/uv"))
        + root.dir_size_bytes(Path::new("var/cache/strata"));
    let installs_bytes = root.dir_size_bytes(Path::new("root/.local/share/mise"))
        + root.dir_size_bytes(Path::new("root/.local/share/uv"))
        + root.dir_size_bytes(Path::new("opt/strata"));

    HostFsSnapshot {
        layout_schema_version,
        layout_compliant: dirs_ok
            && files_ok
            && layout_schema_version == Some(CONTAINER_LAYOUT_SCHEMA_VERSION),
        mirror_preset: cfg_res
            .as_ref()
            .map_or(strata_protocol::rpc::ToolMirrorPreset::Official, |c| {
                c.mirror_preset
            }),
        has_github_token: cfg_res.is_some_and(|c| c.has_github_token),
        cache_bytes,
        installs_bytes,
        paths,
    }
}
