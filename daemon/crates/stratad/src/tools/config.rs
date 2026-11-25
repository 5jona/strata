//! Reads and updates sanitized container mirror, proxy, mise, and secret configuration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use strata_protocol::ContainerConfig;
use strata_protocol::constants::CONTAINER_MISE_CONFIG_FILE;
use strata_protocol::rpc::{
    ToolMirrorPreset, ToolsConfigGetParams, ToolsConfigResult, ToolsConfigSetParams,
};
use strata_protocol::tools::redact_secret;
use strata_sys::mount;
use strata_sys::rootfs_io::RootDir;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::ToolsManager;
use crate::tools::config_patch::{
    parse_mise_toml_summary, patch_mise_toml, sync_apt_mirror_sources, validate_config_params,
};
use crate::tools::exec::{
    MISE_MIRRORS_ENV_REL, MISE_SECRETS_ENV_REL, parse_env_assignments, quote_posix_env,
    read_root_utf8,
};

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
    let mount_dir = PathBuf::from(&entry.mount_dir);
    let cid = container.clone();

    tokio::task::spawn_blocking(move || {
        if !mount::is_ext4_mounted(&mount_dir).unwrap_or(false) {
            return Err(DaemonError::ContainerNotRunning(format!(
                "container '{cid}' must be running to inspect tool config"
            )));
        }
        let root = RootDir::open(&mount_dir).map_err(strata_runtime::RuntimeError::from_sys)?;
        read_config_from_root(&cid, &root)
    })
    .await
    .map_err(|e| DaemonError::join("tools.configGet join", e))?
}

pub(super) async fn set_config(
    _tools: &ToolsManager,
    container: &str,
    entry: &ContainerConfig,
    params: ToolsConfigSetParams,
) -> DaemonResult<ToolsConfigResult> {
    validate_config_params(&params)?;
    let mount_dir = PathBuf::from(&entry.mount_dir);
    let cid = container.to_owned();

    tokio::task::spawn_blocking(move || {
        if !mount::is_ext4_mounted(&mount_dir).unwrap_or(false) {
            return Err(DaemonError::ContainerNotRunning(format!(
                "container '{cid}' must be running to update tool config"
            )));
        }
        let root = RootDir::open(&mount_dir).map_err(strata_runtime::RuntimeError::from_sys)?;
        apply_config_update(&cid, &root, params)?;
        read_config_from_root(&cid, &root)
    })
    .await
    .map_err(|e| DaemonError::join("tools.configSet join", e))?
}

pub(super) fn read_config_from_root(
    container: &str,
    root: &RootDir,
) -> DaemonResult<ToolsConfigResult> {
    let mirrors_raw = read_root_utf8(root, MISE_MIRRORS_ENV_REL).unwrap_or_default();
    let mirrors = parse_env_assignments(&mirrors_raw);
    let secrets_raw = read_root_utf8(root, MISE_SECRETS_ENV_REL).unwrap_or_default();
    let secrets = parse_env_assignments(&secrets_raw);
    let mise_config_toml = read_root_utf8(root, CONTAINER_MISE_CONFIG_FILE.trim_start_matches('/'))
        .unwrap_or_default();
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
    })
}

pub(super) fn apply_config_update(
    _container: &str,
    root: &RootDir,
    params: ToolsConfigSetParams,
) -> DaemonResult<()> {
    let mirrors_raw = read_root_utf8(root, MISE_MIRRORS_ENV_REL).unwrap_or_default();
    let mut env = parse_env_assignments(&mirrors_raw);
    let os_rel = read_root_utf8(root, "usr/lib/os-release")
        .or_else(|| read_root_utf8(root, "etc/os-release"))
        .unwrap_or_default();
    let is_ubuntu = os_rel.to_ascii_lowercase().contains("id=ubuntu");

    if let Some(preset) = params.mirror_preset {
        apply_preset_defaults(&mut env, preset, is_ubuntu);
    }

    set_opt_env(&mut env, KEY_APT_MIRROR, params.apt_mirror_url.as_deref());
    set_opt_env(&mut env, KEY_NODE_MIRROR, params.node_mirror_url.as_deref());
    if let Some(pypi) = params.pypi_index_url.as_deref() {
        set_opt_env(&mut env, KEY_UV_INDEX, Some(pypi));
        set_opt_env(&mut env, KEY_PIP_INDEX, Some(pypi));
    }
    set_opt_env(
        &mut env,
        KEY_NPM_REGISTRY,
        params.npm_registry_url.as_deref(),
    );
    if let Some(rustup) = params.rustup_dist_server.as_deref() {
        set_opt_env(&mut env, KEY_RUSTUP_DIST, Some(rustup));
        let root_url = rustup.trim().trim_end_matches('/').to_owned();
        if root_url.is_empty() {
            env.remove(KEY_RUSTUP_ROOT);
        } else {
            env.insert(KEY_RUSTUP_ROOT.to_owned(), format!("{root_url}/rustup"));
        }
    }
    set_opt_env(&mut env, KEY_GOPROXY, params.goproxy.as_deref());
    set_opt_env(&mut env, KEY_HTTP_PROXY, params.http_proxy.as_deref());
    set_opt_env(&mut env, KEY_HTTPS_PROXY, params.https_proxy.as_deref());
    set_opt_env(&mut env, KEY_NO_PROXY, params.no_proxy.as_deref());

    let mut rendered = String::from("# Strata managed tool mirror & proxy overrides\n");
    for (k, v) in &env {
        rendered.push_str(&format!("export {k}={}\n", quote_posix_env(v)));
    }
    root.write_atomic(Path::new(MISE_MIRRORS_ENV_REL), rendered.as_bytes(), 0o644)
        .map_err(strata_runtime::RuntimeError::from_sys)?;

    if params.mirror_preset.is_some() || params.apt_mirror_url.is_some() {
        sync_apt_mirror_sources(root, env.get(KEY_APT_MIRROR).map(String::as_str));
    }

    if let Some(token_raw) = params.github_token {
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

    let toml_rel = Path::new(CONTAINER_MISE_CONFIG_FILE.trim_start_matches('/'));
    if let Some(full_toml) = params.mise_config_toml {
        root.write_atomic(toml_rel, full_toml.as_bytes(), 0o644)
            .map_err(strata_runtime::RuntimeError::from_sys)?;
    } else if params.jobs.is_some()
        || params.minimum_release_age.is_some()
        || params.url_replacements.is_some()
    {
        let existing = read_root_utf8(root, CONTAINER_MISE_CONFIG_FILE.trim_start_matches('/'))
            .unwrap_or_default();
        let age_patch = params.minimum_release_age.as_ref().map(|s| {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_owned())
        });
        let updated = patch_mise_toml(
            &existing,
            params.jobs,
            age_patch.as_ref(),
            params.url_replacements.as_ref(),
        );
        root.write_atomic(toml_rel, updated.as_bytes(), 0o644)
            .map_err(strata_runtime::RuntimeError::from_sys)?;
    }
    Ok(())
}

pub(super) fn apply_preset_defaults(
    env: &mut BTreeMap<String, String>,
    preset: ToolMirrorPreset,
    is_ubuntu: bool,
) {
    match preset {
        ToolMirrorPreset::Tuna => {
            env.insert(KEY_PRESET.to_owned(), "tuna".to_owned());
            env.insert(
                KEY_APT_MIRROR.to_owned(),
                if is_ubuntu {
                    "https://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports".to_owned()
                } else {
                    "https://mirrors.tuna.tsinghua.edu.cn/debian".to_owned()
                },
            );
            env.insert(
                KEY_NODE_MIRROR.to_owned(),
                "https://mirrors.tuna.tsinghua.edu.cn/nodejs-release/".to_owned(),
            );
            env.insert(
                KEY_UV_INDEX.to_owned(),
                "https://pypi.tuna.tsinghua.edu.cn/simple".to_owned(),
            );
            env.insert(
                KEY_PIP_INDEX.to_owned(),
                "https://pypi.tuna.tsinghua.edu.cn/simple".to_owned(),
            );
            env.insert(
                KEY_NPM_REGISTRY.to_owned(),
                "https://registry.npmmirror.com".to_owned(),
            );
            env.insert(
                KEY_RUSTUP_DIST.to_owned(),
                "https://mirrors.tuna.tsinghua.edu.cn/rustup".to_owned(),
            );
            env.insert(
                KEY_RUSTUP_ROOT.to_owned(),
                "https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup".to_owned(),
            );
            env.insert(
                KEY_GOPROXY.to_owned(),
                "https://goproxy.cn,direct".to_owned(),
            );
        }
        ToolMirrorPreset::Official => {
            env.insert(KEY_PRESET.to_owned(), "official".to_owned());
            for k in [
                KEY_APT_MIRROR,
                KEY_NODE_MIRROR,
                KEY_UV_INDEX,
                KEY_PIP_INDEX,
                KEY_NPM_REGISTRY,
                KEY_RUSTUP_DIST,
                KEY_RUSTUP_ROOT,
                KEY_GOPROXY,
            ] {
                env.remove(k);
            }
        }
        ToolMirrorPreset::Custom => {
            env.insert(KEY_PRESET.to_owned(), "custom".to_owned());
        }
    }
}

pub(super) fn parse_preset(raw: Option<&str>) -> ToolMirrorPreset {
    match raw {
        Some("tuna") => ToolMirrorPreset::Tuna,
        Some("custom") => ToolMirrorPreset::Custom,
        _ => ToolMirrorPreset::Official,
    }
}

fn non_empty(map: &BTreeMap<String, String>, key: &str) -> Option<String> {
    map.get(key)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn set_opt_env(map: &mut BTreeMap<String, String>, key: &str, value: Option<&str>) {
    if let Some(raw) = value {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            map.remove(key);
        } else {
            map.insert(key.to_owned(), trimmed.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn secret_token_is_written_0600_and_never_returned_plaintext() {
        let base = std::env::temp_dir().join(format!("strata-tools-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("etc/strata/mise")).expect("mkdir");
        std::fs::write(base.join("etc/os-release"), "ID=debian\n").expect("os-release");
        let root = RootDir::open(&base).expect("open root");

        apply_config_update(
            "main",
            &root,
            ToolsConfigSetParams {
                mirror_preset: Some(ToolMirrorPreset::Tuna),
                github_token: Some("ghp_supersecret1234".to_owned()),
                jobs: Some(6),
                ..Default::default()
            },
        )
        .expect("apply");

        let meta =
            std::fs::metadata(base.join("etc/strata/mise/secrets.env")).expect("secret meta");
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);

        let res = read_config_from_root("main", &root).expect("read config");
        assert!(res.has_github_token);
        assert_eq!(res.mirror_preset, ToolMirrorPreset::Tuna);
        assert_eq!(res.jobs, 6);
        let serialized = serde_json::to_string(&res).expect("serialize");
        assert!(!serialized.contains("ghp_supersecret1234"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ubuntu_preset_detects_usr_lib_os_release_when_etc_is_symlink() {
        let base = std::env::temp_dir().join(format!("strata-tools-ubuntu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("etc/strata/mise")).expect("mkdir etc");
        std::fs::create_dir_all(base.join("usr/lib")).expect("mkdir usr/lib");
        std::fs::write(base.join("usr/lib/os-release"), "ID=ubuntu\n").expect("write os-release");
        std::os::unix::fs::symlink("../usr/lib/os-release", base.join("etc/os-release"))
            .expect("symlink");
        let root = RootDir::open(&base).expect("open root");

        apply_config_update(
            "main",
            &root,
            ToolsConfigSetParams {
                mirror_preset: Some(ToolMirrorPreset::Tuna),
                ..Default::default()
            },
        )
        .expect("apply");

        let res = read_config_from_root("main", &root).expect("read config");
        assert_eq!(
            res.apt_mirror_url.as_deref(),
            Some("https://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports")
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
