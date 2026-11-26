//! System package (`dpkg`/`apk`/`pacman`/`rpm`) and manual CLI output parsers + unit tests.

use std::collections::BTreeSet;

use strata_protocol::rpc::{InstalledToolItem, ToolGroupKind, ToolInstallSource, ToolManagerKind};

use crate::tools::catalog_data::{CURATED_DISCOVERED_BINS, KNOWN_APT_PACKAGES};
use crate::tools::discovery::classify_source;

pub(super) fn parse_dpkg_query(raw: &str, manual_only: bool) -> Vec<InstalledToolItem> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 3 || !parts[1].split_whitespace().any(|w| w == "installed") {
            continue;
        }
        let name = parts[0].trim();
        let version = parts[2].trim();
        let known = KNOWN_APT_PACKAGES.iter().find(|(pkg, _)| *pkg == name);
        let group = match known {
            Some((_, g)) => *g,
            None if manual_only && CURATED_DISCOVERED_BINS.contains(&name) => {
                ToolGroupKind::Discovered
            }
            None => continue,
        };
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
            source: Some(ToolInstallSource::System),
            active: true,
            installed: true,
            broken: false,
        });
    }
    out
}

pub(super) fn parse_generic_sys_pkg_lines(
    raw: &str,
    manager: ToolManagerKind,
    backend_tag: &str,
) -> Vec<InstalledToolItem> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (name, version) = if let Some((n, v)) = trimmed.split_once('\t') {
            (n.trim(), v.trim())
        } else if let Some((n, v)) = trimmed.split_once(' ') {
            (n.trim(), v.trim())
        } else {
            continue;
        };
        let group = KNOWN_APT_PACKAGES
            .iter()
            .find(|(pkg, _)| *pkg == name)
            .map(|(_, g)| *g)
            .or_else(|| {
                CURATED_DISCOVERED_BINS
                    .contains(&name)
                    .then_some(ToolGroupKind::Discovered)
            });
        let Some(group) = group else { continue };
        out.push(InstalledToolItem {
            id: format!("{backend_tag}:{name}"),
            name: name.to_owned(),
            manager,
            group,
            version: version.to_owned(),
            requested_version: None,
            latest_version: None,
            outdated: false,
            backend: Some(backend_tag.to_owned()),
            install_path: None,
            source_path: None,
            source: Some(ToolInstallSource::System),
            active: true,
            installed: true,
            broken: false,
        });
    }
    out
}

pub(super) fn parse_manual_bins(
    raw: &str,
    already_seen: &BTreeSet<String>,
) -> Vec<InstalledToolItem> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for line in raw.lines() {
        let Some((name, path)) = line.trim().split_once('|') else {
            continue;
        };
        if !CURATED_DISCOVERED_BINS.contains(&name)
            || already_seen.contains(name)
            || !seen.insert(name.to_owned())
        {
            continue;
        }
        out.push(InstalledToolItem {
            id: format!("manual:{name}"),
            name: name.to_owned(),
            manager: ToolManagerKind::Manual,
            group: ToolGroupKind::Discovered,
            version: "installed".to_owned(),
            requested_version: None,
            latest_version: None,
            outdated: false,
            backend: Some("manual".to_owned()),
            install_path: Some(path.to_owned()),
            source_path: None,
            source: Some(classify_source(path)),
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
    use crate::tools::status_parsers::{
        parse_mise_ls_json, parse_mise_outdated_json, parse_uv_python_list_json, parse_uv_tool_list,
    };

    #[test]
    fn parsers_handle_mise_uv_dpkg_and_manual_formats() {
        let mise_json = r#"{
            "python": [{"version": "3.12.9", "requested_version": "3.12", "installed": true, "active": true, "install_path": "/root/.local/share/mise/installs/python/3.12.9", "source": {"path": "/root/.config/mise/config.toml"}}],
            "ripgrep": [{"version": "14.1.1", "installed": true, "active": true, "install_path": "/opt/strata/mise/installs/ripgrep/14.1.1", "backend": "aqua:BurntSushi/ripgrep"}]
        }"#;
        let items = parse_mise_ls_json(mise_json, ToolInstallSource::UserInstalled);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| {
            i.name == "python"
                && i.group == ToolGroupKind::MiseRuntimes
                && i.source == Some(ToolInstallSource::UserInstalled)
        }));
        assert!(items.iter().any(|i| {
            i.name == "ripgrep"
                && i.group == ToolGroupKind::CliDev
                && i.source == Some(ToolInstallSource::StrataManaged)
        }));

        let outdated = parse_mise_outdated_json(
            r#"{"python": {"bump": "3.12.10"}, "node": {"latest": "22.14.0"}}"#,
        );
        assert_eq!(outdated.get("python").map(String::as_str), Some("3.12.10"));
        assert_eq!(outdated.get("node").map(String::as_str), Some("22.14.0"));

        let uv = parse_uv_tool_list(
            "ruff v0.11.2 (/root/.local/share/uv/tools/ruff)\n- ruff\nhttpie v3.2.4\n",
            ToolInstallSource::UserInstalled,
        );
        assert_eq!(uv.len(), 2);
        assert_eq!(uv[0].version, "0.11.2");
        assert_eq!(
            uv[0].install_path.as_deref(),
            Some("/root/.local/share/uv/tools/ruff")
        );

        let uv_py = parse_uv_python_list_json(
            r#"[{"version":"3.12.9","path":"/root/.local/share/uv/python/cpython-3.12.9/bin/python3","implementation":"cpython"},{"version":"3.13.0","path":null,"implementation":"cpython"}]"#,
        );
        assert_eq!(uv_py.len(), 1);
        assert_eq!(uv_py[0].version, "3.12.9");

        let dpkg = parse_dpkg_query(
            "git\tinstall ok installed\t1:2.47.2-0.1\nmissing\tdeinstall\t1.0\n",
            false,
        );
        assert_eq!(dpkg.len(), 1);
        assert_eq!(dpkg[0].name, "git");

        let manual = parse_manual_bins(
            "nvim|/usr/local/bin/nvim\ngit|/usr/local/bin/git\n",
            &BTreeSet::from(["git".to_owned()]),
        );
        assert_eq!(manual.len(), 1);
        assert_eq!(manual[0].name, "nvim");
        assert_eq!(manual[0].manager, ToolManagerKind::Manual);
    }
}
