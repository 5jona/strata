//! Reads and updates sanitized container mirror, proxy, mise, and secret configuration.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use strata_protocol::ContainerConfig;
use strata_protocol::rpc::{
    ToolMirrorPreset, ToolsConfigGetParams, ToolsConfigResult, ToolsConfigSetParams,
};
use strata_protocol::tools::redact_secret;
use strata_sys::mount;
use strata_sys::rootfs_io::RootDir;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::ToolsManager;
use crate::tools::config_patch::{
    is_ubuntu_os_release, parse_mise_toml_summary, patch_mise_toml, sync_apt_mirror_sources,
    validate_config_params,
};
use crate::tools::config_presets::{
    apply_preset_defaults, non_empty, parse_preset, set_mirror_env, set_opt_env,
};
use crate::tools::discovery::discover_runtime_managers;
use crate::tools::exec::{
    MISE_MIRRORS_ENV_REL, MISE_SECRETS_ENV_REL, load_container_tool_env_from_mount,
    parse_env_assignments, quote_posix_env, read_root_utf8,
};
use crate::tools::versions_parse::validate_safe_container_path;

pub(super) const DEFAULT_XDG_MISE_CONFIG: &str = "/root/.config/mise/config.toml";
const LEGACY_MISE_CONFIG: &str = "/etc/strata/mise/config.toml";
const WORKSPACE_MISE_CONFIG: &str = "/workspace/mise.toml";

pub(super) const KEY_PRESET: &str = "STRATA_MIRROR_PRESET";
pub(super) const KEY_APT_MIRROR: &str = "STRATA_APT_MIRROR_URL";
pub(super) const KEY_NODE_MIRROR: &str = "MISE_NODE_MIRROR_URL";
pub(super) const KEY_UV_INDEX: &str = "UV_DEFAULT_INDEX";
pub(super) const KEY_PIP_INDEX: &str = "PIP_INDEX_URL";
pub(super) const KEY_NPM_REGISTRY: &str = "NPM_CONFIG_REGISTRY";
pub(super) const KEY_RUSTUP_DIST: &str = "RUSTUP_DIST_SERVER";
pub(super) const KEY_RUSTUP_ROOT: &str = "RUSTUP_UPDATE_ROOT";
pub(super) const KEY_GOPROXY: &str = "GOPROXY";
pub(super) const KEY_HTTP_PROXY: &str = "HTTP_PROXY";
pub(super) const KEY_HTTPS_PROXY: &str = "HTTPS_PROXY";
pub(super) const KEY_NO_PROXY: &str = "NO_PROXY";
const KEY_GITHUB_TOKEN: &str = "MISE_GITHUB_TOKEN";

pub(super) async fn get_config(
    tools: &ToolsManager,
    params: ToolsConfigGetParams,
) -> DaemonResult<ToolsConfigResult> {
    let (container, entry, _) = tools
        .manager
        .resolve_container(params.container.as_deref())
        .await?;
    let requested_path = match params.config_path.as_deref() {
        Some(raw) if !raw.trim().is_empty() => Some(validate_safe_container_path(raw)?),
        _ => None,
    };
    let discovered_paths = discover_live_config_paths(tools, &container).await;
    let mount_dir = PathBuf::from(&entry.mount_dir);
    let cid = container.clone();

    tokio::task::spawn_blocking(move || {
        if !mount::is_ext4_mounted(&mount_dir).unwrap_or(false) {
            return Err(DaemonError::ContainerNotRunning(format!(
                "container '{cid}' must be running to inspect tool config"
            )));
        }
        let root = RootDir::open(&mount_dir).map_err(strata_runtime::RuntimeError::from_sys)?;
        read_config_from_root(&cid, &root, requested_path.as_deref(), discovered_paths)
    })
    .await
    .map_err(|e| DaemonError::join("tools.configGet join", e))?
}

pub(super) async fn set_config(
    tools: &ToolsManager,
    container: &str,
    entry: &ContainerConfig,
    params: ToolsConfigSetParams,
) -> DaemonResult<ToolsConfigResult> {
    validate_config_params(&params)?;
    let requested_path = match params.config_path.as_deref() {
        Some(raw) if !raw.trim().is_empty() => Some(validate_safe_container_path(raw)?),
        _ => None,
    };
    let discovered_paths = discover_live_config_paths(tools, container).await;
    let mount_dir = PathBuf::from(&entry.mount_dir);
    let cid = container.to_owned();

    tokio::task::spawn_blocking(move || {
        if !mount::is_ext4_mounted(&mount_dir).unwrap_or(false) {
            return Err(DaemonError::ContainerNotRunning(format!(
                "container '{cid}' must be running to update tool config"
            )));
        }
        let root = RootDir::open(&mount_dir).map_err(strata_runtime::RuntimeError::from_sys)?;
        let active_path =
            resolve_active_config_path(&root, requested_path.as_deref(), &discovered_paths);
        apply_config_update_at(&cid, &root, params, &active_path)?;
        read_config_from_root(&cid, &root, Some(&active_path), discovered_paths)
    })
    .await
    .map_err(|e| DaemonError::join("tools.configSet join", e))?
}

async fn discover_live_config_paths(tools: &ToolsManager, container: &str) -> Vec<String> {
    let Ok((_, ns_pid, cfg)) = tools.manager.running_target(Some(container)).await else {
        return Vec::new();
    };
    let mdir = PathBuf::from(&cfg.mount_dir);
    let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mdir))
        .await
        .unwrap_or_default();
    discover_runtime_managers(tools, container, ns_pid, &cfg, &env)
        .await
        .mise_config_paths
}

fn collect_config_paths(root: &RootDir, discovered: &[String]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for p in discovered {
        if validate_safe_container_path(p).is_ok() {
            set.insert(p.clone());
        }
    }
    for cand in [
        DEFAULT_XDG_MISE_CONFIG,
        LEGACY_MISE_CONFIG,
        WORKSPACE_MISE_CONFIG,
    ] {
        if root
            .open_file_ro_nofollow(Path::new(cand.trim_start_matches('/')))
            .is_ok()
        {
            set.insert(cand.to_owned());
        }
    }
    set.into_iter().collect()
}

fn resolve_active_config_path(
    root: &RootDir,
    requested: Option<&str>,
    discovered: &[String],
) -> String {
    if let Some(req) = requested {
        return req.to_owned();
    }
    for p in discovered {
        if validate_safe_container_path(p).is_ok() {
            return p.clone();
        }
    }
    for cand in [DEFAULT_XDG_MISE_CONFIG, LEGACY_MISE_CONFIG] {
        if root
            .open_file_ro_nofollow(Path::new(cand.trim_start_matches('/')))
            .is_ok()
        {
            return cand.to_owned();
        }
    }
    DEFAULT_XDG_MISE_CONFIG.to_owned()
}

pub(super) fn read_config_from_root(
    container: &str,
    root: &RootDir,
    requested_path: Option<&str>,
    discovered_paths: Vec<String>,
) -> DaemonResult<ToolsConfigResult> {
    let mirrors_raw = read_root_utf8(root, MISE_MIRRORS_ENV_REL).unwrap_or_default();
    let mirrors = parse_env_assignments(&mirrors_raw);
    let secrets_raw = read_root_utf8(root, MISE_SECRETS_ENV_REL).unwrap_or_default();
    let secrets = parse_env_assignments(&secrets_raw);
    let config_paths = collect_config_paths(root, &discovered_paths);
    let active_path = resolve_active_config_path(root, requested_path, &discovered_paths);
    let mise_config_toml =
        read_root_utf8(root, active_path.trim_start_matches('/')).unwrap_or_default();
    let toml_summary = parse_mise_toml_summary(&mise_config_toml);

    let github_tok = secrets
        .get(KEY_GITHUB_TOKEN)
        .map(String::as_str)
        .filter(|t| !t.is_empty());
    let has_github_token = github_tok.is_some();
    let github_token_masked = github_tok.map(redact_secret);

    Ok(ToolsConfigResult {
        container: container.to_owned(),
        mirror_preset: parse_preset(mirrors.get(KEY_PRESET).map(String::as_str)),
        apt_mirror_url: non_empty(&mirrors, KEY_APT_MIRROR),
        node_mirror_url: non_empty(&mirrors, KEY_NODE_MIRROR),
        pypi_index_url: non_empty(&mirrors, KEY_UV_INDEX),
        npm_registry_url: non_empty(&mirrors, KEY_NPM_REGISTRY),
        rustup_dist_server: non_empty(&mirrors, KEY_RUSTUP_DIST),
        goproxy: non_empty(&mirrors, KEY_GOPROXY),
        http_proxy: non_empty(&mirrors, KEY_HTTP_PROXY),
        https_proxy: non_empty(&mirrors, KEY_HTTPS_PROXY),
        no_proxy: non_empty(&mirrors, KEY_NO_PROXY),
        url_replacements: toml_summary.url_replacements,
        jobs: toml_summary.jobs,
        minimum_release_age: toml_summary.minimum_release_age,
        has_github_token,
        github_token_masked,
        mise_config_toml,
        config_path: Some(active_path),
        config_paths,
    })
}

pub(super) fn apply_config_update(
    container: &str,
    root: &RootDir,
    params: ToolsConfigSetParams,
) -> DaemonResult<()> {
    let active_path = resolve_active_config_path(root, params.config_path.as_deref(), &[]);
    apply_config_update_at(container, root, params, &active_path)
}

fn apply_config_update_at(
    _container: &str,
    root: &RootDir,
    params: ToolsConfigSetParams,
    active_toml_path: &str,
) -> DaemonResult<()> {
    let mirrors_raw = read_root_utf8(root, MISE_MIRRORS_ENV_REL).unwrap_or_default();
    let mut env = parse_env_assignments(&mirrors_raw);
    let os_rel = read_root_utf8(root, "usr/lib/os-release")
        .or_else(|| read_root_utf8(root, "etc/os-release"))
        .unwrap_or_default();
    let is_ubuntu = is_ubuntu_os_release(&os_rel);

    let ignore_empty_mirrors = matches!(
        params.mirror_preset,
        Some(ToolMirrorPreset::Tuna | ToolMirrorPreset::Official)
    );
    if let Some(preset) = params.mirror_preset {
        apply_preset_defaults(&mut env, preset, is_ubuntu);
    }

    set_mirror_env(
        &mut env,
        KEY_APT_MIRROR,
        params.apt_mirror_url.as_deref(),
        ignore_empty_mirrors,
    );
    set_mirror_env(
        &mut env,
        KEY_NODE_MIRROR,
        params.node_mirror_url.as_deref(),
        ignore_empty_mirrors,
    );
    if let Some(pypi) = params.pypi_index_url.as_deref()
        && (!ignore_empty_mirrors || !pypi.trim().is_empty())
    {
        set_opt_env(&mut env, KEY_UV_INDEX, Some(pypi));
        set_opt_env(&mut env, KEY_PIP_INDEX, Some(pypi));
    }
    set_mirror_env(
        &mut env,
        KEY_NPM_REGISTRY,
        params.npm_registry_url.as_deref(),
        ignore_empty_mirrors,
    );
    if let Some(rustup) = params.rustup_dist_server.as_deref()
        && (!ignore_empty_mirrors || !rustup.trim().is_empty())
    {
        set_opt_env(&mut env, KEY_RUSTUP_DIST, Some(rustup));
        let root_url = rustup.trim().trim_end_matches('/').to_owned();
        if root_url.is_empty() {
            env.remove(KEY_RUSTUP_ROOT);
        } else {
            env.insert(KEY_RUSTUP_ROOT.to_owned(), format!("{root_url}/rustup"));
        }
    }
    set_mirror_env(
        &mut env,
        KEY_GOPROXY,
        params.goproxy.as_deref(),
        ignore_empty_mirrors,
    );
    set_opt_env(&mut env, KEY_HTTP_PROXY, params.http_proxy.as_deref());
    set_opt_env(&mut env, KEY_HTTPS_PROXY, params.https_proxy.as_deref());
    set_opt_env(&mut env, KEY_NO_PROXY, params.no_proxy.as_deref());

    root.ensure_dir(Path::new("etc/strata/mise"), 0o755)
        .map_err(strata_runtime::RuntimeError::from_sys)?;
    let mut rendered = String::from("# Strata managed tool mirror & proxy overrides\n");
    for (k, v) in &env {
        rendered.push_str(&format!("export {k}={}\n", quote_posix_env(v)));
    }
    root.write_atomic(Path::new(MISE_MIRRORS_ENV_REL), rendered.as_bytes(), 0o644)
        .map_err(strata_runtime::RuntimeError::from_sys)?;

    if params.mirror_preset.is_some() || params.apt_mirror_url.is_some() {
        sync_apt_mirror_sources(root, env.get(KEY_APT_MIRROR).map(String::as_str));
    }

    if let Some(ref token_raw) = params.github_token {
        let trimmed = token_raw.trim();
        let rel = Path::new(MISE_SECRETS_ENV_REL);
        if trimmed.is_empty() {
            root.remove_file(rel)
                .map_err(strata_runtime::RuntimeError::from_sys)?;
        } else {
            let line = format!(
                "# Strata secret token overrides (mode 0600)\nexport {KEY_GITHUB_TOKEN}={}\n",
                quote_posix_env(trimmed)
            );
            root.write_atomic(rel, line.as_bytes(), 0o600)
                .map_err(strata_runtime::RuntimeError::from_sys)?;
        }
    }

    update_mise_toml(root, active_toml_path, &params)?;
    Ok(())
}

fn update_mise_toml(
    root: &RootDir,
    active_toml_path: &str,
    params: &ToolsConfigSetParams,
) -> DaemonResult<()> {
    let has_structured = params.jobs.is_some()
        || params.minimum_release_age.is_some()
        || params.url_replacements.is_some();
    if params.mise_config_toml.is_none() && !has_structured {
        return Ok(());
    }

    let rel_str = active_toml_path.trim_start_matches('/');
    let toml_rel = Path::new(rel_str);
    if let Some(parent) = toml_rel.parent()
        && !parent.as_os_str().is_empty()
    {
        root.ensure_dir(parent, 0o755)
            .map_err(strata_runtime::RuntimeError::from_sys)?;
    }

    let existing = read_root_utf8(root, rel_str).unwrap_or_default();
    let old_summary = parse_mise_toml_summary(&existing);
    let age_patch = params.minimum_release_age.as_ref().map(|s| {
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_owned())
    });

    let final_toml = if let Some(ref raw_toml) = params.mise_config_toml {
        let raw_unchanged = raw_toml.trim() == existing.trim();
        let jobs_patch = params
            .jobs
            .filter(|j| raw_unchanged || *j != old_summary.jobs);
        let age_eff = age_patch
            .as_ref()
            .filter(|a| raw_unchanged || *a != &old_summary.minimum_release_age);
        let repls_eff = params
            .url_replacements
            .as_ref()
            .filter(|r| raw_unchanged || *r != &old_summary.url_replacements);
        if jobs_patch.is_some() || age_eff.is_some() || repls_eff.is_some() {
            patch_mise_toml(raw_toml, jobs_patch, age_eff, repls_eff)
        } else {
            raw_toml.clone()
        }
    } else {
        patch_mise_toml(
            &existing,
            params.jobs,
            age_patch.as_ref(),
            params.url_replacements.as_ref(),
        )
    };

    root.write_atomic(toml_rel, final_toml.as_bytes(), 0o644)
        .map_err(strata_runtime::RuntimeError::from_sys)?;
    Ok(())
}
