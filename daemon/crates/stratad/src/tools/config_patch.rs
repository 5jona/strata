//! Lightweight line-oriented `/etc/strata/mise/config.toml` and APT mirror rewriters.

use std::collections::BTreeMap;
use std::path::Path;

use strata_protocol::rpc::ToolsConfigSetParams;
use strata_protocol::tools::{
    validate_http_url, validate_proxy_list, validate_release_age, validate_token_secret,
};
use strata_sys::rootfs_io::RootDir;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::exec::read_root_utf8;

pub(super) fn validate_config_params(params: &ToolsConfigSetParams) -> DaemonResult<()> {
    for opt in [
        params.apt_mirror_url.as_deref(),
        params.node_mirror_url.as_deref(),
        params.pypi_index_url.as_deref(),
        params.npm_registry_url.as_deref(),
        params.rustup_dist_server.as_deref(),
        params.http_proxy.as_deref(),
        params.https_proxy.as_deref(),
    ] {
        if let Some(raw) = opt
            && !raw.trim().is_empty()
        {
            validate_http_url(raw).map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
        }
    }
    if let Some(goproxy) = params.goproxy.as_deref()
        && !goproxy.trim().is_empty()
    {
        validate_proxy_list(goproxy).map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
    }
    if let Some(no_proxy) = params.no_proxy.as_deref()
        && !no_proxy.trim().is_empty()
    {
        validate_proxy_list(no_proxy).map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
    }
    if let Some(jobs) = params.jobs
        && !(1..=32).contains(&jobs)
    {
        return Err(DaemonError::InvalidParams(
            "jobs must be between 1 and 32".to_owned(),
        ));
    }
    if let Some(age) = params.minimum_release_age.as_deref()
        && !age.trim().is_empty()
    {
        validate_release_age(age).map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
    }
    if let Some(tok) = params.github_token.as_deref()
        && !tok.trim().is_empty()
    {
        validate_token_secret(tok).map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
    }
    if let Some(repls) = params.url_replacements.as_ref() {
        for (k, v) in repls {
            if k.trim().is_empty() || v.trim().is_empty() {
                return Err(DaemonError::InvalidParams(
                    "urlReplacements keys and values must be non-empty".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(super) struct ParsedMiseToml {
    pub(super) jobs: u32,
    pub(super) minimum_release_age: Option<String>,
    pub(super) url_replacements: BTreeMap<String, String>,
}

/// Parses `jobs`, `minimum_release_age`, and `[settings.url_replacements]` from `config.toml`.
pub(super) fn parse_mise_toml_summary(raw: &str) -> ParsedMiseToml {
    let mut jobs = 4_u32;
    let mut minimum_release_age = None;
    let mut url_replacements = BTreeMap::new();
    let mut section = String::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed[1..trimmed.len() - 1].trim().to_owned();
            continue;
        }
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let key = unquote_toml_key(k.trim());
        let val = v.trim();
        match section.as_str() {
            "settings" => match key.as_str() {
                "jobs" => {
                    if let Ok(n) = val.parse::<u32>() {
                        jobs = n;
                    }
                }
                "minimum_release_age" => {
                    minimum_release_age = Some(unquote_toml_str(val));
                }
                _ => {}
            },
            "settings.url_replacements" if !key.is_empty() => {
                url_replacements.insert(key, unquote_toml_str(val));
            }
            _ => {}
        }
    }

    ParsedMiseToml {
        jobs,
        minimum_release_age,
        url_replacements,
    }
}

/// Updates `jobs`, `minimum_release_age`, and `[settings.url_replacements]` while keeping `[tools]` intact.
pub(super) fn patch_mise_toml(
    raw: &str,
    jobs: Option<u32>,
    minimum_release_age: Option<&Option<String>>,
    url_replacements: Option<&BTreeMap<String, String>>,
) -> String {
    let mut out_lines: Vec<String> = Vec::new();
    let mut section = String::new();
    let mut saw_settings = false;
    let mut wrote_jobs = false;
    let mut wrote_age = false;
    let mut in_url_repl = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if section == "settings" {
                if !wrote_jobs && let Some(j) = jobs {
                    out_lines.push(format!("jobs = {j}"));
                }
                if !wrote_age && let Some(Some(age)) = minimum_release_age {
                    out_lines.push(format!("minimum_release_age = {}", quote_toml_str(age)));
                }
            }
            let next_sec = trimmed[1..trimmed.len() - 1].trim().to_owned();
            in_url_repl = next_sec == "settings.url_replacements" && url_replacements.is_some();
            if in_url_repl {
                section = next_sec;
                continue;
            }
            if next_sec == "settings" {
                saw_settings = true;
            }
            section = next_sec;
            out_lines.push(line.to_owned());
            continue;
        }
        if in_url_repl {
            continue;
        }
        if section == "settings"
            && let Some((k, _)) = trimmed.split_once('=')
        {
            let key = unquote_toml_key(k.trim());
            if key == "jobs"
                && let Some(j) = jobs
            {
                out_lines.push(format!("jobs = {j}"));
                wrote_jobs = true;
                continue;
            }
            if key == "minimum_release_age"
                && let Some(opt_age) = minimum_release_age
            {
                wrote_age = true;
                if let Some(age) = opt_age {
                    out_lines.push(format!("minimum_release_age = {}", quote_toml_str(age)));
                }
                continue;
            }
        }
        out_lines.push(line.to_owned());
    }

    if section == "settings" {
        if !wrote_jobs && let Some(j) = jobs {
            out_lines.push(format!("jobs = {j}"));
        }
        if !wrote_age && let Some(Some(age)) = minimum_release_age {
            out_lines.push(format!("minimum_release_age = {}", quote_toml_str(age)));
        }
    }
    if !saw_settings && (jobs.is_some() || matches!(minimum_release_age, Some(Some(_)))) {
        out_lines.push(String::new());
        out_lines.push("[settings]".to_owned());
        if let Some(j) = jobs {
            out_lines.push(format!("jobs = {j}"));
        }
        if let Some(Some(age)) = minimum_release_age {
            out_lines.push(format!("minimum_release_age = {}", quote_toml_str(age)));
        }
    }
    if let Some(repls) = url_replacements
        && !repls.is_empty()
    {
        if !out_lines.last().is_some_and(String::is_empty) {
            out_lines.push(String::new());
        }
        out_lines.push("[settings.url_replacements]".to_owned());
        for (k, v) in repls {
            out_lines.push(format!("{} = {}", quote_toml_str(k), quote_toml_str(v)));
        }
    }

    let mut result = out_lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Updates APT main mirror URIs cleanly in `/etc/apt/sources.list*` without touching `99strata-sandbox`.
pub(super) fn sync_apt_mirror_sources(root: &RootDir, mirror_url: Option<&str>) {
    let os_rel = read_root_utf8(root, "usr/lib/os-release")
        .or_else(|| read_root_utf8(root, "etc/os-release"))
        .unwrap_or_default();
    let is_ubuntu = os_rel.to_ascii_lowercase().contains("id=ubuntu");
    let target = mirror_url
        .map(|s| s.trim_end_matches('/').to_owned())
        .unwrap_or_else(|| {
            if is_ubuntu {
                "https://ports.ubuntu.com/ubuntu-ports".to_owned()
            } else {
                "https://deb.debian.org/debian".to_owned()
            }
        });
    let from_prefixes = if is_ubuntu {
        &[
            "https://ports.ubuntu.com/ubuntu-ports",
            "http://ports.ubuntu.com/ubuntu-ports",
            "https://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports",
            "http://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports",
        ][..]
    } else {
        &[
            "https://deb.debian.org/debian",
            "http://deb.debian.org/debian",
            "https://mirrors.tuna.tsinghua.edu.cn/debian",
            "http://mirrors.tuna.tsinghua.edu.cn/debian",
        ][..]
    };

    for rel in [
        "etc/apt/sources.list",
        "etc/apt/sources.list.d/debian.sources",
        "etc/apt/sources.list.d/ubuntu.sources",
    ] {
        let Some(content) = read_root_utf8(root, rel) else {
            continue;
        };
        let mut updated = content.clone();
        for old in from_prefixes {
            updated = updated.replace(old, &target);
        }
        if updated != content {
            let _ = root.write_atomic(Path::new(rel), updated.as_bytes(), 0o644);
        }
    }
}

fn unquote_toml_key(raw: &str) -> String {
    if (raw.starts_with('"') && raw.ends_with('"'))
        || (raw.starts_with('\'') && raw.ends_with('\''))
    {
        return unquote_toml_str(raw);
    }
    raw.to_owned()
}

fn unquote_toml_str(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        return trimmed[1..trimmed.len() - 1].replace("\\\"", "\"");
    }
    trimmed.to_owned()
}

fn quote_toml_str(val: &str) -> String {
    let escaped = val.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_mise_toml_preserves_tools_and_updates_settings() {
        let orig = "[settings]\njobs = 4\nyes = true\n\n[tools]\npython = \"3.12\"\n";
        let mut repls = BTreeMap::new();
        repls.insert("github.com".to_owned(), "mirror.example.com".to_owned());
        let patched = patch_mise_toml(orig, Some(8), Some(&Some("7d".to_owned())), Some(&repls));
        assert!(patched.contains("python = \"3.12\""));
        let parsed = parse_mise_toml_summary(&patched);
        assert_eq!(parsed.jobs, 8);
        assert_eq!(parsed.minimum_release_age.as_deref(), Some("7d"));
        assert_eq!(
            parsed
                .url_replacements
                .get("github.com")
                .map(String::as_str),
            Some("mirror.example.com")
        );
    }
}
