//! Human-readable output formatters for `strata tools` subcommands.

use strata_protocol::rpc::{
    ToolDiagnosticStatus, ToolGroupKind, ToolManagerKind, ToolMirrorPreset, ToolsCacheCleanResult,
    ToolsCatalogResult, ToolsConfigResult, ToolsDoctorResult, ToolsListResult, ToolsStatusResult,
};

use super::format_bytes;

pub(crate) fn print_tools_status(r: &ToolsStatusResult) {
    println!("container         : {}", r.container);
    println!(
        "layout            : compliant={} (schema={})",
        r.layout_compliant,
        r.layout_schema_version
            .map_or_else(|| "missing".to_owned(), |v| v.to_string())
    );
    println!("workspace_dir     : {}", r.workspace_dir);
    println!("mise_config       : {}", r.mise_config_path);
    println!(
        "mise              : {}",
        format_health(&r.mise.installed, r.mise.version.as_deref())
    );
    println!(
        "uv                : {}",
        format_health(&r.uv.installed, r.uv.version.as_deref())
    );
    println!(
        "apt               : {}",
        format_health(&r.apt.installed, r.apt.version.as_deref())
    );
    println!(
        "host-exec         : {}",
        format_health(&r.host_exec.installed, r.host_exec.version.as_deref())
    );
    println!(
        "strata-debug      : {}",
        format_health(&r.strata_debug.installed, r.strata_debug.version.as_deref())
    );
    println!("installed_tools   : {}", r.installed_tool_count);
    println!("mirror_preset     : {}", preset_label(r.mirror_preset));
    println!("github_token      : {}", r.has_github_token);
    println!("cache_size        : {}", format_bytes(r.cache_bytes));
    println!("installs_size     : {}", format_bytes(r.installs_bytes));
}

pub(crate) fn print_tools_list(r: &ToolsListResult) {
    if let Some(warn) = &r.outdated_warning {
        eprintln!("warning: {warn}");
    }
    if r.items.is_empty() {
        println!("(no managed tools detected in '{}')", r.container);
        return;
    }
    println!(
        "{:<8} {:<14} {:<18} {:<14} {:<12} STATE",
        "MANAGER", "GROUP", "NAME", "VERSION", "LATEST"
    );
    for item in &r.items {
        let state = if item.outdated {
            "outdated"
        } else if item.broken {
            "broken"
        } else if item.active {
            "active"
        } else {
            "installed"
        };
        println!(
            "{:<8} {:<14} {:<18} {:<14} {:<12} {}",
            manager_label(item.manager),
            group_label(item.group),
            item.name,
            item.version,
            item.latest_version.as_deref().unwrap_or("-"),
            state
        );
    }
}

pub(crate) fn print_tools_catalog(r: &ToolsCatalogResult) {
    println!("=== Curated Bundles ===");
    for b in &r.bundles {
        println!("  {:<14} {} [{}]", b.id, b.summary, b.tools.join(", "));
    }
    println!("\n=== ARM64/GKI Safe Tools ===");
    println!(
        "{:<16} {:<8} {:<14} {:<10} SUMMARY",
        "ID", "MANAGER", "GROUP", "VERSION"
    );
    for e in &r.entries {
        println!(
            "{:<16} {:<8} {:<14} {:<10} {}",
            e.id,
            manager_label(e.manager),
            group_label(e.group),
            e.default_version,
            e.summary
        );
    }
}

pub(crate) fn print_tools_config(r: &ToolsConfigResult) {
    println!("container         : {}", r.container);
    println!("mirror_preset     : {}", preset_label(r.mirror_preset));
    println!("jobs              : {}", r.jobs);
    println!(
        "min_release_age   : {}",
        r.minimum_release_age.as_deref().unwrap_or("-")
    );
    println!(
        "apt_mirror        : {}",
        r.apt_mirror_url.as_deref().unwrap_or("-")
    );
    println!(
        "pypi_index        : {}",
        r.pypi_index_url.as_deref().unwrap_or("-")
    );
    println!(
        "npm_registry      : {}",
        r.npm_registry_url.as_deref().unwrap_or("-")
    );
    println!(
        "node_mirror       : {}",
        r.node_mirror_url.as_deref().unwrap_or("-")
    );
    println!(
        "github_token      : {}",
        r.github_token_masked.as_deref().unwrap_or("unset")
    );
    if !r.url_replacements.is_empty() {
        println!("url_replacements  : {:?}", r.url_replacements);
    }
}

pub(crate) fn print_tools_doctor(r: &ToolsDoctorResult) {
    println!("container : {} (ok={})", r.container, r.ok);
    for item in &r.items {
        let tag = match item.status {
            ToolDiagnosticStatus::Ok => "OK",
            ToolDiagnosticStatus::Warn => "WARN",
            ToolDiagnosticStatus::Error => "ERR",
        };
        println!(
            "[{tag:<4}] {:<20} ({}) {}",
            item.id, item.category, item.summary
        );
        if let Some(rem) = &item.remediation {
            println!("       fix: {rem}");
        }
    }
}

pub(crate) fn print_tools_cache_clean(r: &ToolsCacheCleanResult) {
    println!(
        "Cache cleaned for '{}': before={}, after={}, freed={}.",
        r.container,
        format_bytes(r.before_bytes),
        format_bytes(r.after_bytes),
        format_bytes(r.freed_bytes),
    );
}

fn format_health(installed: &bool, version: Option<&str>) -> String {
    if !installed {
        "missing".to_owned()
    } else {
        version.unwrap_or("installed").to_owned()
    }
}

fn preset_label(p: ToolMirrorPreset) -> &'static str {
    match p {
        ToolMirrorPreset::Tuna => "tuna",
        ToolMirrorPreset::Official => "official",
        ToolMirrorPreset::Custom => "custom",
    }
}

fn manager_label(m: ToolManagerKind) -> &'static str {
    match m {
        ToolManagerKind::Mise => "mise",
        ToolManagerKind::Apt => "apt",
        ToolManagerKind::Uv => "uv",
        ToolManagerKind::Bundle => "bundle",
        ToolManagerKind::System => "system",
        ToolManagerKind::Apk => "apk",
        ToolManagerKind::Pacman => "pacman",
        ToolManagerKind::Dnf => "dnf",
        ToolManagerKind::Manual => "manual",
    }
}

fn group_label(g: ToolGroupKind) -> &'static str {
    match g {
        ToolGroupKind::CoreBridge => "core-bridge",
        ToolGroupKind::MiseRuntimes => "mise-runtimes",
        ToolGroupKind::CliDev => "cli-dev",
        ToolGroupKind::DebugL3L5 => "debug-l3-l5",
        ToolGroupKind::PythonUv => "python-uv",
        ToolGroupKind::Shell => "shell",
        ToolGroupKind::Discovered => "discovered",
    }
}
