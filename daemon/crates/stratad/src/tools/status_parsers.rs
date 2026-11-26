//! Inventory JSON/text parsers for `tools.status` & `tools.list`.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use strata_protocol::rpc::{InstalledToolItem, ToolGroupKind, ToolInstallSource, ToolManagerKind};

use crate::tools::catalog::classify_mise_tool_group;
use crate::tools::discovery::classify_source;

pub(super) fn parse_mise_ls_json(
    raw: &str,
    fallback_source: ToolInstallSource,
) -> Vec<InstalledToolItem> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(trimmed) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (tool_name, val) in map {
        let Some(arr) = val.as_array() else {
            continue;
        };
        for entry in arr {
            let version = entry
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            let installed = entry
                .get("installed")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let active = entry
                .get("active")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let requested_version = entry
                .get("requested_version")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let install_path = entry
                .get("install_path")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let source_path = entry
                .get("source")
                .and_then(|s| s.get("path"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let backend = entry
                .get("backend")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let source = install_path
                .as_deref()
                .map_or(fallback_source, classify_source);
            out.push(InstalledToolItem {
                id: format!("mise:{tool_name}@{version}"),
                name: tool_name.clone(),
                manager: ToolManagerKind::Mise,
                group: classify_mise_tool_group(&tool_name),
                version,
                requested_version,
                latest_version: None,
                outdated: false,
                backend,
                install_path,
                source_path,
                source: Some(source),
                active,
                installed,
                broken: !installed,
            });
        }
    }
    out
}

pub(super) fn parse_mise_outdated_json(raw: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Ok(val) = serde_json::from_str::<Value>(raw.trim()) else {
        return map;
    };
    let pick_latest = |entry: &Value| {
        entry
            .get("bump")
            .and_then(Value::as_str)
            .or_else(|| entry.get("latest").and_then(Value::as_str))
            .map(str::to_owned)
    };
    match val {
        Value::Object(obj) => {
            for (name, entry) in obj {
                if let Some(latest) = pick_latest(&entry) {
                    map.insert(name, latest);
                }
            }
        }
        Value::Array(arr) => {
            for entry in arr {
                if let (Some(name), Some(latest)) = (
                    entry.get("name").and_then(Value::as_str),
                    pick_latest(&entry),
                ) {
                    map.insert(name.to_owned(), latest);
                }
            }
        }
        _ => {}
    }
    map
}

pub(super) fn parse_uv_python_list_json(raw: &str) -> Vec<InstalledToolItem> {
    let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(raw.trim()) else {
        return Vec::new();
    };
    let mut seen_paths = BTreeSet::new();
    let mut out = Vec::new();
    for entry in arr {
        let Some(path) = entry.get("path").and_then(Value::as_str) else {
            continue;
        };
        if !seen_paths.insert(path.to_owned()) {
            continue;
        }
        let version = entry
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let impl_name = entry
            .get("implementation")
            .and_then(Value::as_str)
            .unwrap_or("cpython");
        let tool_name = if impl_name.eq_ignore_ascii_case("cpython") {
            "python".to_owned()
        } else {
            impl_name.to_ascii_lowercase()
        };
        let source = classify_source(path);
        out.push(InstalledToolItem {
            id: format!("uv-python:{impl_name}@{version}:{path}"),
            name: tool_name,
            manager: ToolManagerKind::Uv,
            group: ToolGroupKind::PythonUv,
            version,
            requested_version: None,
            latest_version: None,
            outdated: false,
            backend: Some("uv-python".to_owned()),
            install_path: Some(path.to_owned()),
            source_path: None,
            source: Some(source),
            active: true,
            installed: true,
            broken: false,
        });
    }
    out
}

pub(super) fn parse_uv_tool_list(raw: &str, source: ToolInstallSource) -> Vec<InstalledToolItem> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('-') || trimmed.starts_with("No tools") {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let version = parts
            .next()
            .unwrap_or("unknown")
            .trim_start_matches('v')
            .to_owned();
        let install_path = parts
            .next()
            .map(|p| p.trim_matches(|c| c == '(' || c == ')').to_owned())
            .filter(|p| p.starts_with('/'));
        let item_source = install_path.as_deref().map_or(source, classify_source);
        out.push(InstalledToolItem {
            id: format!("uv:{name}"),
            name: name.to_owned(),
            manager: ToolManagerKind::Uv,
            group: ToolGroupKind::PythonUv,
            version,
            requested_version: None,
            latest_version: None,
            outdated: false,
            backend: Some("uv".to_owned()),
            install_path,
            source_path: None,
            source: Some(item_source),
            active: true,
            installed: true,
            broken: false,
        });
    }
    out
}
