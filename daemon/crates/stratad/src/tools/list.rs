//! Full container tool inventory (`tools.list`) aggregating mise, uv, system, and manual tools.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use strata_protocol::rpc::{
    InstalledToolItem, ToolGroupKind, ToolInstallSource, ToolManagerKind, ToolsListParams,
    ToolsListResult,
};

use crate::error::{DaemonError, DaemonResult};
use crate::tools::ToolsManager;
use crate::tools::discovery::{classify_source, discover_runtime_managers};
use crate::tools::exec::{load_container_tool_env_from_mount, run_container_command_bounded};
use crate::tools::list_sys::collect_system_packages;
use crate::tools::status_parsers::{
    parse_mise_ls_json, parse_mise_outdated_json, parse_uv_python_list_json, parse_uv_tool_list,
};
use crate::tools::status_parsers_sys::parse_manual_bins;

const MANUAL_BIN_SCAN_SCRIPT: &str = r#"
for d in /root/.local/bin /root/.cargo/bin /usr/local/bin; do
  [ -d "$d" ] || continue
  for f in "$d"/*; do
    [ -x "$f" ] && [ ! -d "$f" ] || continue
    printf '%s|%s\n' "$(basename "$f")" "$f"
  done
done
"#;

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

    let disc = discover_runtime_managers(tools, &container, ns_pid, &cfg, &env).await;
    let mut items: Vec<InstalledToolItem> = Vec::new();
    let mut outdated_warning: Option<String> = None;

    let mise_bin = disc.bins.get("mise").map(|(p, _)| p.clone());
    let mise_available = mise_bin.is_some();
    if let Some(ref bin) = mise_bin {
        if let Ok(out) = run_container_command_bounded(
            &tools.self_exe,
            &container,
            ns_pid,
            &cfg,
            vec![
                bin.clone(),
                "ls".to_owned(),
                "--all-sources".to_owned(),
                "--json".to_owned(),
            ],
            env.clone(),
            Duration::from_secs(8),
        )
        .await
            && out.status == 0
        {
            items.extend(parse_mise_ls_json(&out.stdout, classify_source(bin)));
        }

        if disc.mise_data_dir.as_deref() != Some("/opt/strata/mise") {
            let mut legacy_env = env.clone();
            legacy_env.insert("MISE_DATA_DIR".to_owned(), "/opt/strata/mise".to_owned());
            if let Ok(leg) = run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec![bin.clone(), "ls".to_owned(), "--json".to_owned()],
                legacy_env,
                Duration::from_secs(6),
            )
            .await
                && leg.status == 0
            {
                let existing_ids: BTreeSet<String> = items.iter().map(|i| i.id.clone()).collect();
                for mut item in parse_mise_ls_json(&leg.stdout, ToolInstallSource::StrataManaged) {
                    if !existing_ids.contains(&item.id) {
                        item.source = Some(ToolInstallSource::StrataManaged);
                        items.push(item);
                    }
                }
            }
        }

        if params.check_outdated {
            match run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec![
                    bin.clone(),
                    "outdated".to_owned(),
                    "--bump".to_owned(),
                    "--json".to_owned(),
                ],
                env.clone(),
                Duration::from_secs(12),
            )
            .await
            {
                Ok(out) if out.status == 0 => {
                    let outdated_map = parse_mise_outdated_json(&out.stdout);
                    for item in &mut items {
                        if item.manager == ToolManagerKind::Mise
                            && let Some(latest) = outdated_map.get(&item.name)
                        {
                            item.latest_version = Some(latest.clone());
                            item.outdated = latest != &item.version;
                        }
                    }
                }
                Ok(out) => {
                    outdated_warning = Some(format!(
                        "mise outdated check failed (status {}): {}",
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
    }

    let uv_bin = disc.bins.get("uv").map(|(p, _)| p.clone());
    let uv_available = uv_bin.is_some();
    if let Some(ref bin) = uv_bin {
        if let Ok(out) = run_container_command_bounded(
            &tools.self_exe,
            &container,
            ns_pid,
            &cfg,
            vec![
                bin.clone(),
                "tool".to_owned(),
                "list".to_owned(),
                "--show-paths".to_owned(),
            ],
            env.clone(),
            Duration::from_secs(6),
        )
        .await
            && out.status == 0
        {
            items.extend(parse_uv_tool_list(&out.stdout, classify_source(bin)));
        }
        if let Ok(py_out) = run_container_command_bounded(
            &tools.self_exe,
            &container,
            ns_pid,
            &cfg,
            vec![
                bin.clone(),
                "python".to_owned(),
                "list".to_owned(),
                "--output-format".to_owned(),
                "json".to_owned(),
                "--only-installed".to_owned(),
            ],
            env.clone(),
            Duration::from_secs(6),
        )
        .await
            && py_out.status == 0
        {
            items.extend(parse_uv_python_list_json(&py_out.stdout));
        }
    }

    collect_system_packages(
        tools,
        &container,
        ns_pid,
        &cfg,
        &env,
        disc.system_pkg_manager,
        &mut items,
    )
    .await;

    for key in ["host-exec", "strata-debug"] {
        if let Some((path, ver)) = disc.bins.get(key) {
            items.push(InstalledToolItem {
                id: format!("system:{key}"),
                name: key.to_owned(),
                manager: ToolManagerKind::System,
                group: ToolGroupKind::CoreBridge,
                version: ver.clone().unwrap_or_else(|| "installed".to_owned()),
                requested_version: None,
                latest_version: None,
                outdated: false,
                backend: Some("system".to_owned()),
                install_path: Some(path.clone()),
                source_path: None,
                source: Some(classify_source(path)),
                active: true,
                installed: true,
                broken: false,
            });
        }
    }

    let already_seen: BTreeSet<String> = items.iter().map(|i| i.name.clone()).collect();
    if let Ok(bin_out) = run_container_command_bounded(
        &tools.self_exe,
        &container,
        ns_pid,
        &cfg,
        vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            MANUAL_BIN_SCAN_SCRIPT.to_owned(),
        ],
        env,
        Duration::from_secs(5),
    )
    .await
        && bin_out.status == 0
    {
        items.extend(parse_manual_bins(&bin_out.stdout, &already_seen));
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
