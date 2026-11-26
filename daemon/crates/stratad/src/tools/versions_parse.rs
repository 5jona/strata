//! Command builders, input validators, and JSON parsers for `tools.versions` and `tools.use`.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use strata_protocol::rpc::{ToolManagerKind, ToolUseScope, ToolVersionOption};

use crate::error::{DaemonError, DaemonResult};

const MAX_VERSION_OPTIONS: usize = 120;

pub(super) fn build_use_command(
    manager: ToolManagerKind,
    name: &str,
    version: &str,
    scope: Option<ToolUseScope>,
    config_path: Option<&str>,
    mise_bin: Option<&str>,
    uv_bin: Option<&str>,
) -> DaemonResult<Vec<String>> {
    match manager {
        ToolManagerKind::Mise => {
            let bin = mise_bin.unwrap_or("mise").to_owned();
            let spec = format!("{name}@{version}");
            let mut cmd = vec![bin, "use".to_owned(), "--yes".to_owned()];
            if let Some(path) = config_path {
                cmd.push("--path".to_owned());
                cmd.push(path.to_owned());
            } else if scope == Some(ToolUseScope::Workspace) {
                cmd.push("--path".to_owned());
                cmd.push("/workspace/mise.toml".to_owned());
            } else {
                cmd.push("-g".to_owned());
            }
            cmd.push(spec);
            Ok(cmd)
        }
        ToolManagerKind::Uv => {
            let bin = uv_bin.unwrap_or("uv").to_owned();
            if name == "python" || name.starts_with("python") {
                let scope_tag = if scope == Some(ToolUseScope::Workspace) {
                    "workspace"
                } else {
                    "global"
                };
                Ok(vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "set -eu; uv_bin=$1; ver=$2; scope=$3; \"$uv_bin\" python install \"$ver\"; if [ \"$scope\" = \"workspace\" ]; then cd /workspace && \"$uv_bin\" python pin \"$ver\"; else \"$uv_bin\" python pin --global \"$ver\"; fi".to_owned(),
                    "strata-uv-use".to_owned(),
                    bin,
                    version.to_owned(),
                    scope_tag.to_owned(),
                ])
            } else {
                Ok(vec![
                    bin,
                    "tool".to_owned(),
                    "install".to_owned(),
                    "--force".to_owned(),
                    format!("{name}=={version}"),
                ])
            }
        }
        _ => Err(DaemonError::InvalidParams(
            "tools.use supports mise and uv managers".to_owned(),
        )),
    }
}

pub(super) fn validate_version_token(raw: &str) -> DaemonResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || trimmed.starts_with('-')
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
    {
        return Err(DaemonError::InvalidParams(format!(
            "invalid version specifier '{raw}'"
        )));
    }
    Ok(trimmed.to_owned())
}

pub(super) fn validate_safe_container_path(raw: &str) -> DaemonResult<String> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('/')
        || trimmed.len() < 6
        || trimmed.len() > 256
        || !trimmed.ends_with(".toml")
        || trimmed.contains('\0')
        || trimmed
            .split('/')
            .skip(1)
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        || trimmed.bytes().any(|b| {
            b.is_ascii_whitespace()
                || b.is_ascii_control()
                || matches!(b, b';' | b'|' | b'&' | b'$' | b'`' | b'\\')
        })
    {
        return Err(DaemonError::InvalidParams(format!(
            "invalid container config path '{raw}' (expected clean absolute .toml path)"
        )));
    }
    Ok(trimmed.to_owned())
}

fn is_prerelease(version: &str) -> bool {
    let lower = version.to_ascii_lowercase();
    lower.contains("alpha")
        || lower.contains("beta")
        || lower.contains("rc")
        || lower.contains("dev")
        || lower.contains("nightly")
        || lower.contains("canary")
        || lower.contains("-pre")
        || lower.contains("a0")
        || lower.contains("b0")
}

pub(super) fn parse_mise_installed_versions(raw: &str) -> BTreeMap<String, bool> {
    let mut map = BTreeMap::new();
    let Ok(val) = serde_json::from_str::<Value>(raw.trim()) else {
        return map;
    };
    let mut collect = |arr: &[Value]| {
        for entry in arr {
            if let Some(ver) = entry.get("version").and_then(Value::as_str) {
                let active = entry
                    .get("active")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                map.insert(ver.to_owned(), active);
            }
        }
    };
    match val {
        Value::Array(arr) => collect(&arr),
        Value::Object(obj) => {
            for v in obj.values() {
                if let Some(arr) = v.as_array() {
                    collect(arr);
                }
            }
        }
        _ => {}
    }
    map
}

pub(super) fn parse_mise_ls_remote_json(
    raw: &str,
    installed_map: &BTreeMap<String, bool>,
    include_prerelease: bool,
) -> Vec<ToolVersionOption> {
    let mut entries: Vec<(String, Option<String>)> = Vec::new();
    if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(raw.trim()) {
        for item in arr {
            if let Some(v) = item.get("version").and_then(Value::as_str) {
                let created = item
                    .get("created_at")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                entries.push((v.to_owned(), created));
            }
        }
    } else {
        for line in raw.lines() {
            let v = line.trim();
            if !v.is_empty() {
                entries.push((v.to_owned(), None));
            }
        }
    }

    entries.reverse();
    let mut out = Vec::new();
    for (version, created_at) in entries {
        let pre = is_prerelease(&version);
        if pre && !include_prerelease {
            continue;
        }
        let installed = installed_map.contains_key(&version);
        let active = installed_map.get(&version).copied().unwrap_or(false);
        out.push(ToolVersionOption {
            version,
            created_at,
            installed,
            active,
            prerelease: pre,
        });
        if out.len() >= MAX_VERSION_OPTIONS {
            break;
        }
    }
    out
}

pub(super) fn parse_uv_python_versions_json(
    raw: &str,
    include_prerelease: bool,
) -> Vec<ToolVersionOption> {
    let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(raw.trim()) else {
        return Vec::new();
    };
    let mut installed_vers = BTreeSet::new();
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();
    for item in &arr {
        let Some(ver) = item.get("version").and_then(Value::as_str) else {
            continue;
        };
        if item.get("path").and_then(Value::as_str).is_some() {
            installed_vers.insert(ver.to_owned());
        }
        if seen.insert(ver.to_owned()) {
            ordered.push(ver.to_owned());
        }
    }
    let mut out = Vec::new();
    for version in ordered {
        let pre = is_prerelease(&version);
        if pre && !include_prerelease {
            continue;
        }
        let installed = installed_vers.contains(&version);
        out.push(ToolVersionOption {
            version,
            created_at: None,
            installed,
            active: false,
            prerelease: pre,
        });
        if out.len() >= MAX_VERSION_OPTIONS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mise_remote_newest_first_and_tags_installed() {
        let raw = r#"[{"version":"3.12.8","created_at":"2025-01-01T00:00:00Z"},{"version":"3.13.0rc1"},{"version":"3.12.9","created_at":"2025-02-01T00:00:00Z"}]"#;
        let mut installed = BTreeMap::new();
        installed.insert("3.12.9".to_owned(), true);
        let stable = parse_mise_ls_remote_json(raw, &installed, false);
        assert_eq!(stable.len(), 2);
        assert_eq!(stable[0].version, "3.12.9");
        assert!(stable[0].installed && stable[0].active);
        assert_eq!(stable[1].version, "3.12.8");

        let all = parse_mise_ls_remote_json(raw, &installed, true);
        assert_eq!(all.len(), 3);
        assert!(all.iter().any(|v| v.version == "3.13.0rc1" && v.prerelease));
    }

    #[test]
    fn build_use_command_supports_global_workspace_and_custom_path() {
        let cmd = build_use_command(
            ToolManagerKind::Mise,
            "python",
            "3.12.9",
            Some(ToolUseScope::Workspace),
            None,
            Some("/root/.local/bin/mise"),
            None,
        )
        .expect("workspace use");
        assert_eq!(
            cmd,
            vec![
                "/root/.local/bin/mise",
                "use",
                "--yes",
                "--path",
                "/workspace/mise.toml",
                "python@3.12.9"
            ]
        );
        assert!(validate_version_token("3.12;rm").is_err());
        assert!(validate_safe_container_path("/root/../etc/passwd").is_err());
        assert!(validate_safe_container_path("/").is_err());
        assert!(validate_safe_container_path("/etc/passwd").is_err());
        assert!(validate_safe_container_path("/root/./mise.toml").is_err());
        assert!(validate_safe_container_path("/root/.config/mise/config.toml").is_ok());

        let obj_installed =
            parse_mise_installed_versions(r#"{"python":[{"version":"3.12.9","active":true}]}"#);
        assert_eq!(obj_installed.get("3.12.9"), Some(&true));
    }
}
