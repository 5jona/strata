//! Filtering, classification, dynamic `mise registry` search, and bundle expansion.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use strata_protocol::rpc::{
    ToolBundleSpec, ToolCatalogEntry, ToolGroupKind, ToolManagerKind, ToolsCatalogParams,
    ToolsCatalogResult,
};
use strata_protocol::tools::validate_package_spec;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::ToolsManager;
use crate::tools::catalog_bundles::BUNDLES;
pub(super) use crate::tools::catalog_bundles::profile_packages;
use crate::tools::catalog_data::CATALOG_ITEMS;
use crate::tools::discovery::resolve_live_tool_bins;
use crate::tools::exec::{load_container_tool_env_from_mount, run_container_command_bounded};

const MAX_REGISTRY_RESULTS: usize = 40;

/// Maps known mise tool identifiers to `ToolGroupKind` cleanly.
pub(super) fn classify_mise_tool_group(name: &str) -> ToolGroupKind {
    if let Some(item) = CATALOG_ITEMS
        .iter()
        .find(|entry| entry.manager == ToolManagerKind::Mise && entry.id == name)
    {
        return item.group;
    }
    match name {
        "python" | "node" | "rust" | "go" | "bun" | "uv" | "ruby" | "java" | "deno" | "zig" => {
            ToolGroupKind::MiseRuntimes
        }
        "zsh" | "fish" | "nu" | "nushell" | "starship" | "zoxide" | "eza" | "carapace" => {
            ToolGroupKind::Shell
        }
        _ => ToolGroupKind::CliDev,
    }
}

/// Builds a filtered `tools.catalog` response, optionally enriching with live `mise registry --json`.
pub(super) async fn query_catalog(
    tools: &ToolsManager,
    params: ToolsCatalogParams,
) -> DaemonResult<ToolsCatalogResult> {
    let (container, _, _) = tools
        .manager
        .resolve_container(params.container.as_deref())
        .await?;
    let mut res = build_catalog(container.clone(), &params);

    let query_lower = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_ascii_lowercase);

    if let Ok((_, ns_pid, cfg)) = tools.manager.running_target(Some(&container)).await {
        let mdir = PathBuf::from(&cfg.mount_dir);
        let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mdir))
            .await
            .map_err(|e| DaemonError::join("tools.catalog env join", e))?;
        let bins = resolve_live_tool_bins(tools, &container, ns_pid, &cfg, &env).await;
        if let Some(mise_bin) = bins.mise {
            res.registry_available = true;
            if let Some(q) = query_lower
                && let Ok(out) = run_container_command_bounded(
                    &tools.self_exe,
                    &container,
                    ns_pid,
                    &cfg,
                    vec![mise_bin, "registry".to_owned(), "--json".to_owned()],
                    env,
                    Duration::from_secs(5),
                )
                .await
                && out.status == 0
            {
                let existing: HashSet<String> = res.entries.iter().map(|e| e.id.clone()).collect();
                let dynamic = parse_mise_registry_matches(&out.stdout, &q, params.group, &existing);
                res.entries.extend(dynamic);
            }
        }
    }

    Ok(res)
}

/// Builds the static curated `tools.catalog` response for `container`.
pub(super) fn build_catalog(container: String, params: &ToolsCatalogParams) -> ToolsCatalogResult {
    let query = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_ascii_lowercase);

    let entries = CATALOG_ITEMS
        .iter()
        .filter(|item| params.group.is_none_or(|g| item.group == g))
        .filter(|item| {
            let Some(q) = query.as_deref() else {
                return true;
            };
            item.id.to_ascii_lowercase().contains(q)
                || item.name.to_ascii_lowercase().contains(q)
                || item.summary.to_ascii_lowercase().contains(q)
                || item.tags.iter().any(|t| t.to_ascii_lowercase().contains(q))
        })
        .map(|item| ToolCatalogEntry {
            id: item.id.to_owned(),
            name: item.name.to_owned(),
            summary: item.summary.to_owned(),
            manager: item.manager,
            group: item.group,
            default_version: item.default_version.to_owned(),
            backend_hint: item.backend_hint.to_owned(),
            arm64_verified: true,
            estimated_mb: item.estimated_mb,
            recommended: item.recommended,
            tags: item.tags.iter().map(|t| (*t).to_owned()).collect(),
        })
        .collect();

    let bundles = BUNDLES
        .iter()
        .map(|b| ToolBundleSpec {
            id: b.id.to_owned(),
            name: b.name.to_owned(),
            summary: b.summary.to_owned(),
            tools: b.tools.iter().map(|t| (*t).to_owned()).collect(),
        })
        .collect();

    ToolsCatalogResult {
        container,
        entries,
        bundles,
        registry_available: false,
    }
}

pub(super) fn parse_mise_registry_matches(
    raw: &str,
    query_lower: &str,
    group_filter: Option<ToolGroupKind>,
    existing_ids: &HashSet<String>,
) -> Vec<ToolCatalogEntry> {
    let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(raw.trim()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in arr {
        let Some(short) = entry.get("short").and_then(Value::as_str) else {
            continue;
        };
        if existing_ids.contains(short)
            || validate_package_spec(ToolManagerKind::Mise, short, true).is_err()
        {
            continue;
        }
        let desc = entry
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let aliases: Vec<&str> = entry
            .get("aliases")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let matched = short.to_ascii_lowercase().contains(query_lower)
            || desc.to_ascii_lowercase().contains(query_lower)
            || aliases
                .iter()
                .any(|a| a.to_ascii_lowercase().contains(query_lower));
        if !matched {
            continue;
        }
        let group = classify_mise_tool_group(short);
        if group_filter.is_some_and(|g| g != group) {
            continue;
        }
        let backend_hint = entry
            .get("backends")
            .and_then(Value::as_array)
            .and_then(|b| b.first())
            .and_then(Value::as_str)
            .unwrap_or("mise")
            .to_owned();
        let summary = if desc.trim().is_empty() {
            format!("mise 注册表工具（{backend_hint}）")
        } else {
            desc.trim().to_owned()
        };
        out.push(ToolCatalogEntry {
            id: short.to_owned(),
            name: short.to_owned(),
            summary,
            manager: ToolManagerKind::Mise,
            group,
            default_version: "latest".to_owned(),
            backend_hint,
            arm64_verified: false,
            estimated_mb: 0,
            recommended: false,
            tags: vec!["mise-registry".to_owned()],
        });
        if out.len() >= MAX_REGISTRY_RESULTS {
            break;
        }
    }
    out
}

/// Expands bundle IDs into validated `(ToolManagerKind, spec)` installation entries.
pub(super) fn expand_bundle_specs(
    bundle_ids: &[String],
) -> DaemonResult<Vec<(ToolManagerKind, String)>> {
    let mut out = Vec::new();
    for raw_id in bundle_ids {
        let trimmed = raw_id.trim();
        let bundle = BUNDLES.iter().find(|b| b.id == trimmed).ok_or_else(|| {
            DaemonError::InvalidParams(format!("unknown tool bundle '{trimmed}'"))
        })?;
        for member in bundle.tools {
            let Some((mgr_str, spec)) = member.split_once(':') else {
                return Err(DaemonError::Internal(format!(
                    "malformed bundle spec '{member}'"
                )));
            };
            let mgr = match mgr_str {
                "mise" => ToolManagerKind::Mise,
                "apt" => ToolManagerKind::Apt,
                "uv" => ToolManagerKind::Uv,
                other => {
                    return Err(DaemonError::Internal(format!(
                        "unsupported manager '{other}' in bundle '{trimmed}'"
                    )));
                }
            };
            let validated = validate_package_spec(mgr, spec, true)
                .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
            out.push((mgr, validated));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_only_arm64_safe_validated_packages() {
        for item in CATALOG_ITEMS {
            assert!(
                validate_package_spec(item.manager, item.id, true).is_ok(),
                "catalog item '{}' failed package validation",
                item.id
            );
        }
        let all_ids: Vec<String> = BUNDLES.iter().map(|b| b.id.to_owned()).collect();
        let expanded = expand_bundle_specs(&all_ids).expect("expand all bundles");
        assert!(!expanded.is_empty());
        assert!(!expanded.iter().any(|(_, s)| s.contains("openssh-server")));
        assert!(BUNDLES.iter().any(|b| b.id == "shell-zsh"));
        assert!(BUNDLES.iter().any(|b| b.id == "shell-fish"));
        assert!(BUNDLES.iter().any(|b| b.id == "shell-nushell"));
    }

    #[test]
    fn catalog_filters_by_group_and_query_and_parses_registry() {
        let res = build_catalog(
            "main".to_owned(),
            &ToolsCatalogParams {
                container: Some("main".to_owned()),
                group: Some(ToolGroupKind::DebugL3L5),
                query: Some("ebpf".to_owned()),
            },
        );
        assert_eq!(res.entries.len(), 1);
        assert_eq!(res.entries[0].id, "bpftrace");

        let reg = r#"[{"short":"lazygit","backends":["aqua:jesseduffield/lazygit"],"description":"Simple terminal UI for git"}]"#;
        let matches = parse_mise_registry_matches(reg, "lazygit", None, &HashSet::new());
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "lazygit");
        assert!(!matches[0].arm64_verified);
    }
}
