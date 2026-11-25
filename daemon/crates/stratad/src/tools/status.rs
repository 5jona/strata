//! Local-only status snapshot (`tools.status`) and full inventory (`tools.list`) queries.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use strata_protocol::constants::{
    CONTAINER_LAYOUT_PATH, CONTAINER_LAYOUT_SCHEMA_VERSION, CONTAINER_MISE_CONFIG_FILE,
    CONTAINER_OPT_BIN_DIR, CONTAINER_WORKSPACE_DIR,
};
use strata_protocol::rpc::{
    ToolManagerHealth, ToolsListParams, ToolsListResult, ToolsStatusParams, ToolsStatusResult,
};
use strata_sys::mount;
use strata_sys::rootfs_io::RootDir;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::ToolsManager;
use crate::tools::catalog_data::KNOWN_APT_PACKAGES;
use crate::tools::config::read_config_from_root;
use crate::tools::exec::{
    load_container_tool_env_from_mount, read_root_utf8, run_container_command_bounded,
};
use crate::tools::status_parsers::{
    dir_size_bytes, parse_dpkg_query, parse_mise_ls_json, parse_mise_outdated_json,
    parse_uv_tool_list,
};

const STATUS_PROBE_SCRIPT: &str = "\
for cmd in /opt/strata/bin/mise /opt/strata/bin/uv apt-get host-exec strata-debug; do
  if command -v \"$cmd\" >/dev/null 2>&1 || [ -x \"$cmd\" ]; then
    v=$(\"$cmd\" --version 2>/dev/null | head -n1 || true)
    printf 'BIN:%s|%s\\n' \"$cmd\" \"$v\"
  fi
done
if [ -x /opt/strata/bin/mise ]; then
  /opt/strata/bin/mise ls --json 2>/dev/null || true
fi";

struct HostFsSnapshot {
    mounted: bool,
    layout_schema_version: Option<u32>,
    layout_compliant: bool,
    mirror_preset: strata_protocol::rpc::ToolMirrorPreset,
    has_github_token: bool,
    cache_bytes: u64,
    installs_bytes: u64,
    mise_present: bool,
    uv_present: bool,
    apt_present: bool,
    host_exec_present: bool,
    strata_debug_present: bool,
}

impl HostFsSnapshot {
    fn empty(mounted: bool) -> Self {
        Self {
            mounted,
            layout_schema_version: None,
            layout_compliant: false,
            mirror_preset: strata_protocol::rpc::ToolMirrorPreset::Official,
            has_github_token: false,
            cache_bytes: 0,
            installs_bytes: 0,
            mise_present: false,
            uv_present: false,
            apt_present: false,
            host_exec_present: false,
            strata_debug_present: false,
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

    let mut mise = ToolManagerHealth {
        installed: fs_snap.mise_present,
        version: None,
        bin_path: fs_snap
            .mise_present
            .then(|| format!("{CONTAINER_OPT_BIN_DIR}/mise")),
    };
    let mut uv = ToolManagerHealth {
        installed: fs_snap.uv_present,
        version: None,
        bin_path: fs_snap
            .uv_present
            .then(|| format!("{CONTAINER_OPT_BIN_DIR}/uv")),
    };
    let mut apt = ToolManagerHealth {
        installed: fs_snap.apt_present,
        version: None,
        bin_path: fs_snap.apt_present.then(|| "/usr/bin/apt-get".to_owned()),
    };
    let mut host_exec = ToolManagerHealth {
        installed: fs_snap.host_exec_present,
        version: None,
        bin_path: fs_snap
            .host_exec_present
            .then(|| "/usr/bin/host-exec".to_owned()),
    };
    let mut strata_debug = ToolManagerHealth {
        installed: fs_snap.strata_debug_present,
        version: None,
        bin_path: fs_snap
            .strata_debug_present
            .then(|| "/usr/bin/strata-debug".to_owned()),
    };
    let mut installed_tool_count = 0_usize;

    if let Ok((_, ns_pid, cfg)) = tools.manager.running_target(Some(&container)).await
        && let Ok(out) = run_container_command_bounded(
            &tools.self_exe,
            &container,
            ns_pid,
            &cfg,
            vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                STATUS_PROBE_SCRIPT.to_owned(),
            ],
            BTreeMap::new(),
            Duration::from_secs(4),
        )
        .await
    {
        let mut json_buf = String::new();
        for line in out.stdout.lines() {
            if let Some(rest) = line.strip_prefix("BIN:") {
                let Some((cmd, ver)) = rest.split_once('|') else {
                    continue;
                };
                let v = (!ver.trim().is_empty()).then(|| ver.trim().to_owned());
                let target = match cmd {
                    "/opt/strata/bin/mise" => Some(&mut mise),
                    "/opt/strata/bin/uv" => Some(&mut uv),
                    "apt-get" => Some(&mut apt),
                    "host-exec" => Some(&mut host_exec),
                    "strata-debug" => Some(&mut strata_debug),
                    _ => None,
                };
                if let Some(h) = target {
                    h.installed = true;
                    h.version = v;
                }
            } else {
                json_buf.push_str(line);
                json_buf.push('\n');
            }
        }
        installed_tool_count = parse_mise_ls_json(&json_buf).len();
    }

    Ok(ToolsStatusResult {
        container,
        layout_schema_version: fs_snap.layout_schema_version,
        layout_compliant: fs_snap.mounted && fs_snap.layout_compliant,
        workspace_dir: CONTAINER_WORKSPACE_DIR.to_owned(),
        mise_config_path: CONTAINER_MISE_CONFIG_FILE.to_owned(),
        mise,
        uv,
        apt,
        host_exec,
        strata_debug,
        installed_tool_count,
        outdated_tool_count: None,
        mirror_preset: fs_snap.mirror_preset,
        has_github_token: fs_snap.has_github_token,
        cache_bytes: fs_snap.cache_bytes,
        installs_bytes: fs_snap.installs_bytes,
    })
}

pub(super) async fn query_list(
    tools: &ToolsManager,
    params: ToolsListParams,
) -> DaemonResult<ToolsListResult> {
    let (container, ns_pid, cfg) = tools
        .manager
        .running_target(params.container.as_deref())
        .await?;
    let mount_dir = PathBuf::from(&cfg.mount_dir);
    let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mount_dir))
        .await
        .map_err(|e| DaemonError::join("tools.list env join", e))?;

    let mut items = Vec::new();
    let mise_out = run_container_command_bounded(
        &tools.self_exe,
        &container,
        ns_pid,
        &cfg,
        vec!["mise".to_owned(), "ls".to_owned(), "--json".to_owned()],
        env.clone(),
        Duration::from_secs(6),
    )
    .await;
    let mise_available = mise_out
        .as_ref()
        .is_ok_and(|o| o.status == 0 && !o.timed_out);
    if let Ok(out) = &mise_out {
        items.extend(parse_mise_ls_json(&out.stdout));
    }

    let mut outdated_warning = None;
    if params.check_outdated && mise_available {
        match run_container_command_bounded(
            &tools.self_exe,
            &container,
            ns_pid,
            &cfg,
            vec![
                "mise".to_owned(),
                "outdated".to_owned(),
                "--json".to_owned(),
            ],
            env.clone(),
            Duration::from_secs(20),
        )
        .await
        {
            Ok(out) if out.status == 0 && !out.timed_out => {
                let outdated_map = parse_mise_outdated_json(&out.stdout);
                for item in &mut items {
                    if let Some(latest) = outdated_map.get(&item.name) {
                        item.latest_version = Some(latest.clone());
                        item.outdated = true;
                    }
                }
            }
            Ok(out) => {
                outdated_warning = Some(format!(
                    "mise outdated check did not complete cleanly (exit {}): {}",
                    out.status,
                    out.stderr
                        .lines()
                        .next()
                        .unwrap_or("offline or rate-limited")
                ));
            }
            Err(e) => outdated_warning = Some(format!("mise outdated check failed: {e}")),
        }
    }

    let uv_out = run_container_command_bounded(
        &tools.self_exe,
        &container,
        ns_pid,
        &cfg,
        vec!["uv".to_owned(), "tool".to_owned(), "list".to_owned()],
        env.clone(),
        Duration::from_secs(6),
    )
    .await;
    let uv_available = uv_out.as_ref().is_ok_and(|o| o.status == 0 && !o.timed_out);
    if let Ok(out) = &uv_out
        && out.status == 0
    {
        items.extend(parse_uv_tool_list(&out.stdout));
    }

    let mut apt_cmd = vec![
        "dpkg-query".to_owned(),
        "-W".to_owned(),
        "-f=${Package}\\t${Status}\\t${Version}\\n".to_owned(),
    ];
    for (pkg, _) in KNOWN_APT_PACKAGES {
        apt_cmd.push((*pkg).to_owned());
    }
    if let Ok(apt_out) = run_container_command_bounded(
        &tools.self_exe,
        &container,
        ns_pid,
        &cfg,
        apt_cmd,
        env,
        Duration::from_secs(6),
    )
    .await
    {
        items.extend(parse_dpkg_query(&apt_out.stdout));
    }

    if let Some(filter) = params.group {
        items.retain(|item| item.group == filter);
    }

    Ok(ToolsListResult {
        container,
        mise_available,
        uv_available,
        items,
        outdated_warning,
    })
}

fn inspect_host_fs(container: &str, mount_dir: &Path) -> HostFsSnapshot {
    if !mount::is_ext4_mounted(mount_dir).unwrap_or(false) {
        return HostFsSnapshot::empty(false);
    }
    let Ok(root) = RootDir::open(mount_dir) else {
        return HostFsSnapshot::empty(true);
    };
    let layout_schema_version =
        read_root_utf8(&root, CONTAINER_LAYOUT_PATH.trim_start_matches('/'))
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.get("schemaVersion")?.as_u64())
            .and_then(|n| u32::try_from(n).ok());

    let dirs_ok = [
        "workspace",
        "opt/strata/bin",
        "opt/strata/mise",
        "opt/strata/uv",
        "etc/strata/mise",
        "var/cache/strata/mise",
        "var/cache/strata/uv",
    ]
    .iter()
    .all(|rel| root.is_dir(Path::new(rel)));
    let files_ok = root
        .open_file_ro_nofollow(Path::new("etc/profile.d/strata-env.sh"))
        .is_ok()
        && root
            .open_file_ro_nofollow(Path::new(
                CONTAINER_MISE_CONFIG_FILE.trim_start_matches('/'),
            ))
            .is_ok();
    let cfg_res = read_config_from_root(container, &root).ok();

    HostFsSnapshot {
        mounted: true,
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
        cache_bytes: dir_size_bytes(&mount_dir.join("var/cache/strata")),
        installs_bytes: dir_size_bytes(&mount_dir.join("opt/strata")),
        mise_present: root
            .open_file_ro_nofollow(Path::new("opt/strata/bin/mise"))
            .is_ok(),
        uv_present: root
            .open_file_ro_nofollow(Path::new("opt/strata/bin/uv"))
            .is_ok(),
        apt_present: root
            .open_file_ro_nofollow(Path::new("usr/bin/apt-get"))
            .is_ok(),
        host_exec_present: root
            .open_file_ro_nofollow(Path::new("usr/bin/host-exec"))
            .is_ok()
            || root
                .open_file_ro_nofollow(Path::new("opt/strata/bin/host-exec"))
                .is_ok(),
        strata_debug_present: root
            .open_file_ro_nofollow(Path::new("usr/bin/strata-debug"))
            .is_ok()
            || root
                .open_file_ro_nofollow(Path::new("opt/strata/bin/strata-debug"))
                .is_ok(),
    }
}
