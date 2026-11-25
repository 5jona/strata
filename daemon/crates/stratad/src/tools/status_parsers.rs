//! Inventory JSON/text parsers and recursive directory sizing for `tools.status` & `tools.list`.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use strata_protocol::rpc::{InstalledToolItem, ToolGroupKind, ToolManagerKind};

use crate::tools::catalog::classify_mise_tool_group;
use crate::tools::catalog_data::KNOWN_APT_PACKAGES;

pub(super) fn dir_size_bytes(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| dir_size_bytes(&entry.path()))
        .sum()
}

pub(super) fn parse_mise_ls_json(raw: &str) -> Vec<InstalledToolItem> {
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
    match val {
        Value::Object(obj) => {
            for (name, entry) in obj {
                if let Some(latest) = entry.get("bump").and_then(Value::as_str) {
                    map.insert(name, latest.to_owned());
                }
            }
        }
        Value::Array(arr) => {
            for entry in arr {
                if let (Some(name), Some(latest)) = (
                    entry.get("name").and_then(Value::as_str),
                    entry.get("bump").and_then(Value::as_str),
                ) {
                    map.insert(name.to_owned(), latest.to_owned());
                }
            }
        }
        _ => {}
    }
    map
}

pub(super) fn parse_uv_tool_list(raw: &str) -> Vec<InstalledToolItem> {
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
            install_path: None,
            source_path: None,
            active: true,
            installed: true,
            broken: false,
        });
    }
    out
}

pub(super) fn parse_dpkg_query(raw: &str) -> Vec<InstalledToolItem> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 3 || !parts[1].split_whitespace().any(|w| w == "installed") {
            continue;
        }
        let name = parts[0].trim();
        let version = parts[2].trim();
        let group = KNOWN_APT_PACKAGES
            .iter()
            .find(|(pkg, _)| *pkg == name)
            .map_or(ToolGroupKind::CliDev, |(_, g)| *g);
        out.push(InstalledToolItem {
            id: format!("apt:{name}"),
            name: name.to_owned(),
            manager: ToolManagerKind::Apt,
            group,
            version: version.to_owned(),
            requested_version: None,
            latest_version: None,
            outdated: false,
            backend: Some("apt".to_owned()),
            install_path: None,
            source_path: None,
            active: true,
            installed: true,
            broken: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsers_handle_mise_uv_and_dpkg_formats() {
        let mise_json = r#"{
            "python": [{"version": "3.12.9", "requested_version": "3.12", "installed": true, "active": true, "install_path": "/opt/strata/mise/installs/python/3.12.9", "source": {"path": "/etc/strata/mise/config.toml"}}],
            "ripgrep": [{"version": "14.1.1", "installed": true, "active": true, "backend": "aqua:BurntSushi/ripgrep"}]
        }"#;
        let items = parse_mise_ls_json(mise_json);
        assert_eq!(items.len(), 2);
        assert!(
            items
                .iter()
                .any(|i| i.name == "python" && i.group == ToolGroupKind::MiseRuntimes)
        );
        assert!(
            items
                .iter()
                .any(|i| i.name == "ripgrep" && i.group == ToolGroupKind::CliDev)
        );

        let outdated = parse_mise_outdated_json(r#"{"python": {"bump": "3.12.10"}}"#);
        assert_eq!(outdated.get("python").map(String::as_str), Some("3.12.10"));

        let uv = parse_uv_tool_list("ruff v0.11.2\n- ruff\nhttpie v3.2.4\n");
        assert_eq!(uv.len(), 2);
        assert_eq!(uv[0].version, "0.11.2");

        let dpkg = parse_dpkg_query(
            "git\tinstall ok installed\t1:2.47.2-0.1\nmissing\tdeinstall\t1.0\npython3-venv\tunknown ok not-installed\t\n",
        );
        assert_eq!(dpkg.len(), 1);
        assert_eq!(dpkg[0].name, "git");
    }
}
