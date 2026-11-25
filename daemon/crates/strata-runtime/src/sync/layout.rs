//! Idempotent synchronization of Strata's canonical in-container directory layout,
//! environment profiles (`/etc/profile.d/strata-env.sh`, `/etc/environment`),
//! `mise` baseline config (`/etc/strata/mise/config.toml`), and `/etc/strata/layout.json`.

use std::path::Path;

use serde_json::json;
use strata_protocol::constants::{
    CANONICAL_CONTAINER_ENV, CONTAINER_CACHE_ROOT, CONTAINER_ETC_ENVIRONMENT_PATH,
    CONTAINER_LAYOUT_PATH, CONTAINER_LAYOUT_SCHEMA_VERSION, CONTAINER_MISE_CACHE_DIR,
    CONTAINER_MISE_CONFIG_DIR, CONTAINER_MISE_CONFIG_FILE, CONTAINER_MISE_DATA_DIR,
    CONTAINER_MISE_SHIMS_DIR, CONTAINER_OPT_BIN_DIR, CONTAINER_OPT_ROOT,
    CONTAINER_PROFILE_ENV_PATH, CONTAINER_STRATA_ETC_DIR, CONTAINER_UV_CACHE_DIR,
    CONTAINER_UV_TOOL_DIR, CONTAINER_WORKSPACE_DIR, DEFAULT_CONTAINER_PATH,
};
use strata_sys::rootfs_io::RootDir;

use crate::error::{RuntimeError, RuntimeResult};

const STRATA_PROFILE_BLOCK_START: &str = "# >>> strata-env >>>";
const STRATA_PROFILE_BLOCK_END: &str = "# <<< strata-env <<<";

const DEFAULT_MISE_CONFIG_TOML: &str = "\
# Strata canonical global mise configuration (/etc/strata/mise/config.toml)
[settings]
jobs = 4
http_timeout = \"60s\"
fetch_remote_versions_cache = \"1h\"
yes = true
experimental = false
idiomatic_version_file_enable_tools = [\"node\", \"python\", \"rust\"]

[settings.python]
compile = false
uv_venv_auto = \"create|source\"

[settings.node]
compile = false

[tools]
";

/// Synchronizes canonical Strata directories, `/etc/profile.d/strata-env.sh`, `/etc/environment`,
/// shell startup hooks, `/etc/strata/mise/config.toml` (if absent), and `/etc/strata/layout.json`.
pub(crate) fn sync_container_layout(root: &RootDir) -> RuntimeResult<()> {
    for (rel, mode) in [
        ("workspace", 0o755),
        ("opt/strata", 0o755),
        ("opt/strata/bin", 0o755),
        ("opt/strata/mise", 0o755),
        ("opt/strata/mise/shims", 0o755),
        ("opt/strata/uv", 0o755),
        ("opt/strata/uv/tools", 0o755),
        ("etc/strata", 0o755),
        ("etc/strata/mise", 0o755),
        ("etc/profile.d", 0o755),
        ("var/cache/strata", 0o755),
        ("var/cache/strata/mise", 0o755),
        ("var/cache/strata/uv", 0o755),
        ("root", 0o700),
        ("root/.config", 0o700),
        ("root/.local", 0o755),
        ("root/.local/share", 0o755),
        ("root/.local/state", 0o700),
        ("root/.cache", 0o755),
    ] {
        root.ensure_dir(Path::new(rel), mode)
            .map_err(RuntimeError::from_sys)?;
    }

    let profile_script = render_profile_env_script();
    root.sync_bytes_if_changed(
        profile_script.as_bytes(),
        Path::new(CONTAINER_PROFILE_ENV_PATH.trim_start_matches('/')),
        0o644,
    )
    .map_err(RuntimeError::from_sys)?;

    let etc_env = render_etc_environment();
    root.sync_bytes_if_changed(
        etc_env.as_bytes(),
        Path::new(CONTAINER_ETC_ENVIRONMENT_PATH.trim_start_matches('/')),
        0o644,
    )
    .map_err(RuntimeError::from_sys)?;

    ensure_profile_hook(root, Path::new("etc/profile"))?;
    ensure_profile_hook(root, Path::new("etc/bash.bashrc"))?;
    ensure_profile_hook(root, Path::new("root/.bashrc"))?;

    let mise_cfg_rel = Path::new(CONTAINER_MISE_CONFIG_FILE.trim_start_matches('/'));
    if root.open_file_ro_nofollow(mise_cfg_rel).is_err() {
        root.write_atomic(mise_cfg_rel, DEFAULT_MISE_CONFIG_TOML.as_bytes(), 0o644)
            .map_err(RuntimeError::from_sys)?;
    }

    let layout_json = render_layout_json();
    root.sync_bytes_if_changed(
        layout_json.as_bytes(),
        Path::new(CONTAINER_LAYOUT_PATH.trim_start_matches('/')),
        0o644,
    )
    .map_err(RuntimeError::from_sys)?;

    // Keep APT sandbox root drop-in synced for Debian/Ubuntu rootfs (required on Android paranoid_network).
    if root.is_dir(Path::new("etc/apt/apt.conf.d")) {
        root.sync_bytes_if_changed(
            b"APT::Sandbox::User \"root\";\n",
            Path::new("etc/apt/apt.conf.d/99strata-sandbox"),
            0o644,
        )
        .map_err(RuntimeError::from_sys)?;
    }

    Ok(())
}

/// Renders `/etc/profile.d/strata-env.sh` (POSIX sh + Bash interactive `mise activate`).
#[must_use]
pub fn render_profile_env_script() -> String {
    let mut out = String::from("# Strata managed container environment (do not edit manually)\n");
    for (key, val) in CANONICAL_CONTAINER_ENV {
        if key == "PATH" {
            continue;
        }
        out.push_str(&format!("export {key}=\"{val}\"\n"));
    }
    out.push_str("export BASH_ENV=\"/etc/profile.d/strata-env.sh\"\n");
    out.push_str(&format!(
        "case \":${{PATH:-}}:\" in\n  *\":{CONTAINER_MISE_SHIMS_DIR}:\"*) ;;\n  *) export PATH=\"{DEFAULT_CONTAINER_PATH}\" ;;\nesac\n"
    ));
    out.push_str(
        "if [ -r /etc/strata/mise/mirrors.env ]; then\n  . /etc/strata/mise/mirrors.env\nfi\n",
    );
    out.push_str(
        "if [ -r /etc/strata/mise/secrets.env ]; then\n  . /etc/strata/mise/secrets.env\nfi\n",
    );
    out.push_str(
        "if [ -n \"${BASH_VERSION:-}\" ]; then\n  case \"$-\" in\n    *i*) if [ -x /opt/strata/bin/mise ]; then eval \"$(/opt/strata/bin/mise activate bash)\" 2>/dev/null || true; fi ;;\n  esac\nfi\n",
    );
    out
}

/// Renders `/etc/environment` containing canonical container environment assignments.
#[must_use]
pub fn render_etc_environment() -> String {
    let mut out = String::new();
    for (key, val) in CANONICAL_CONTAINER_ENV {
        out.push_str(&format!("{key}=\"{val}\"\n"));
    }
    out.push_str("BASH_ENV=\"/etc/profile.d/strata-env.sh\"\n");
    out
}

/// Renders `/etc/strata/layout.json` documenting the standardized container layout schema.
#[must_use]
pub fn render_layout_json() -> String {
    let payload = json!({
        "schemaVersion": CONTAINER_LAYOUT_SCHEMA_VERSION,
        "workspaceDir": CONTAINER_WORKSPACE_DIR,
        "optRoot": CONTAINER_OPT_ROOT,
        "optBinDir": CONTAINER_OPT_BIN_DIR,
        "miseDataDir": CONTAINER_MISE_DATA_DIR,
        "miseShimsDir": CONTAINER_MISE_SHIMS_DIR,
        "miseConfigDir": CONTAINER_MISE_CONFIG_DIR,
        "miseConfigFile": CONTAINER_MISE_CONFIG_FILE,
        "uvToolDir": CONTAINER_UV_TOOL_DIR,
        "cacheRoot": CONTAINER_CACHE_ROOT,
        "miseCacheDir": CONTAINER_MISE_CACHE_DIR,
        "uvCacheDir": CONTAINER_UV_CACHE_DIR,
        "etcDir": CONTAINER_STRATA_ETC_DIR,
        "profileEnv": CONTAINER_PROFILE_ENV_PATH,
        "etcEnvironment": CONTAINER_ETC_ENVIRONMENT_PATH,
        "path": DEFAULT_CONTAINER_PATH,
    });
    let mut text = serde_json::to_string_pretty(&payload).unwrap_or_default();
    text.push('\n');
    text
}

fn ensure_profile_hook(root: &RootDir, rel_path: &Path) -> RuntimeResult<()> {
    let existing = match root.read_file(rel_path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
    };
    let hook = format!(
        "{STRATA_PROFILE_BLOCK_START}\n[ -r {CONTAINER_PROFILE_ENV_PATH} ] && . {CONTAINER_PROFILE_ENV_PATH}\n{STRATA_PROFILE_BLOCK_END}\n"
    );
    // Prepend so the hook runs before Debian/Ubuntu's early `[ -z "$PS1" ] && return`
    // in `/root/.bashrc` and `/etc/bash.bashrc` (required for non-interactive SSH).
    if existing.starts_with(&hook) {
        return Ok(());
    }
    let cleaned = strip_profile_hook(&existing);
    let next = if cleaned.is_empty() {
        hook
    } else {
        format!("{hook}\n{cleaned}")
    };
    root.sync_bytes_if_changed(next.as_bytes(), rel_path, 0o644)
        .map_err(RuntimeError::from_sys)?;
    Ok(())
}

fn strip_profile_hook(raw: &str) -> String {
    let mut out = Vec::new();
    let mut skipping = false;
    for line in raw.lines() {
        if line.trim() == STRATA_PROFILE_BLOCK_START {
            skipping = true;
            continue;
        }
        if skipping {
            if line.trim() == STRATA_PROFILE_BLOCK_END {
                skipping = false;
            }
            continue;
        }
        out.push(line);
    }
    let joined = out.join("\n").trim_matches('\n').to_owned();
    if joined.is_empty() {
        joined
    } else {
        format!("{joined}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn syncs_canonical_layout_idempotently_and_preserves_existing_mise_config() {
        let base = std::env::temp_dir().join(format!("strata-layout-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create root");
        fs::create_dir_all(base.join("root")).expect("create root home");
        fs::write(base.join("root/.bashrc"), "# existing bashrc\n").expect("seed bashrc");

        let root = RootDir::open(&base).expect("open root");
        sync_container_layout(&root).expect("first sync");

        assert!(base.join("workspace").is_dir());
        assert!(base.join("opt/strata/mise/shims").is_dir());
        assert!(base.join("opt/strata/uv/tools").is_dir());
        assert!(base.join("var/cache/strata/mise").is_dir());
        assert!(base.join("var/cache/strata/uv").is_dir());
        assert!(base.join("etc/profile.d/strata-env.sh").is_file());
        assert!(base.join("etc/environment").is_file());
        assert!(base.join("etc/strata/layout.json").is_file());
        assert!(base.join("etc/strata/mise/config.toml").is_file());

        let bashrc = fs::read_to_string(base.join("root/.bashrc")).expect("read bashrc");
        assert!(bashrc.contains("# existing bashrc"));
        assert!(bashrc.starts_with(STRATA_PROFILE_BLOCK_START));

        // Customize mise config and re-sync: must NOT overwrite user/tool edits.
        fs::write(
            base.join("etc/strata/mise/config.toml"),
            "[tools]\nripgrep = \"14\"\n",
        )
        .expect("customize mise config");
        sync_container_layout(&root).expect("second sync");

        let preserved =
            fs::read_to_string(base.join("etc/strata/mise/config.toml")).expect("read mise");
        assert!(preserved.contains("ripgrep = \"14\""));

        let bashrc_again = fs::read_to_string(base.join("root/.bashrc")).expect("read bashrc");
        assert_eq!(
            bashrc_again.matches(STRATA_PROFILE_BLOCK_START).count(),
            1,
            "profile hook must be idempotent"
        );

        let profile_env =
            fs::read_to_string(base.join("etc/profile.d/strata-env.sh")).expect("read profile env");
        assert!(profile_env.contains("case \"$-\" in"));
        assert!(profile_env.contains("export MISE_DATA_DIR=\"/opt/strata/mise\""));
        assert!(profile_env.contains("export UV_TOOL_BIN_DIR=\"/opt/strata/bin\""));

        let etc_env = fs::read_to_string(base.join("etc/environment")).expect("read etc/env");
        assert!(etc_env.contains("XDG_CONFIG_HOME=\"/root/.config\""));
        assert!(etc_env.contains("MISE_GLOBAL_CONFIG_FILE=\"/etc/strata/mise/config.toml\""));

        let _ = fs::remove_dir_all(&base);
    }
}
