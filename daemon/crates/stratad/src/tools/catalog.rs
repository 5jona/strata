//! Filtering, classification, and bundle expansion over the curated ARM64/GKI tool catalog.

use strata_protocol::rpc::{
    ToolBundleSpec, ToolCatalogEntry, ToolGroupKind, ToolManagerKind, ToolsBootstrapProfile,
    ToolsCatalogParams, ToolsCatalogResult,
};
use strata_protocol::tools::validate_package_spec;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::catalog_data::CATALOG_ITEMS;

struct StaticBundle {
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    tools: &'static [&'static str],
}

const BUNDLES: &[StaticBundle] = &[
    StaticBundle {
        id: "core-dev",
        name: "Core Dev",
        summary: "Python 3.12, Node LTS, uv, git, curl, tmux, ripgrep, fd, jq",
        tools: &[
            "apt:ca-certificates",
            "apt:curl",
            "apt:git",
            "apt:tmux",
            "mise:python@3.12",
            "mise:node@lts",
            "mise:uv@latest",
            "mise:ripgrep@latest",
            "mise:fd@latest",
            "mise:jq@latest",
        ],
    },
    StaticBundle {
        id: "python-uv",
        name: "Python + uv",
        summary: "Python 3.12 managed by mise alongside uv, ruff, and httpie",
        tools: &["mise:python@3.12", "mise:uv@latest", "uv:ruff", "uv:httpie"],
    },
    StaticBundle {
        id: "node-ts",
        name: "Node + Bun",
        summary: "Node.js LTS and Bun TypeScript runtime",
        tools: &["mise:node@lts", "mise:bun@latest"],
    },
    StaticBundle {
        id: "debug-l3-l5",
        name: "Debug L3-L5",
        summary: "Android/GKI container diagnostics: strace, tcpdump, bpftrace, sqlite3",
        tools: &["apt:strace", "apt:tcpdump", "apt:bpftrace", "apt:sqlite3"],
    },
];

/// Returns APT & mise packages added after bootstrap base init for a given profile.
pub(super) fn profile_packages(
    profile: ToolsBootstrapProfile,
) -> (&'static [&'static str], &'static [&'static str]) {
    match profile {
        ToolsBootstrapProfile::Minimal => {
            (&["ca-certificates", "curl", "git", "xz-utils", "zstd"], &[])
        }
        ToolsBootstrapProfile::Dev => (
            &["ca-certificates", "curl", "git", "xz-utils", "zstd", "tmux"],
            &[
                "python@3.12",
                "node@lts",
                "ripgrep@latest",
                "fd@latest",
                "jq@latest",
            ],
        ),
        ToolsBootstrapProfile::Debug => (
            &[
                "ca-certificates",
                "curl",
                "git",
                "xz-utils",
                "zstd",
                "tmux",
                "strace",
                "tcpdump",
                "bpftrace",
                "sqlite3",
            ],
            &[
                "python@3.12",
                "node@lts",
                "ripgrep@latest",
                "fd@latest",
                "jq@latest",
            ],
        ),
    }
}

/// Maps known mise tool identifiers to `ToolGroupKind` cleanly.
pub(super) fn classify_mise_tool_group(name: &str) -> ToolGroupKind {
    if let Some(item) = CATALOG_ITEMS
        .iter()
        .find(|entry| entry.manager == ToolManagerKind::Mise && entry.id == name)
    {
        return item.group;
    }
    match name {
        "python" | "node" | "rust" | "go" | "bun" | "uv" | "ruby" | "java" | "deno" => {
            ToolGroupKind::MiseRuntimes
        }
        _ => ToolGroupKind::CliDev,
    }
}

/// Builds a filtered `tools.catalog` response for `container`.
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
    }
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
    }

    #[test]
    fn catalog_filters_by_group_and_query() {
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
    }
}
