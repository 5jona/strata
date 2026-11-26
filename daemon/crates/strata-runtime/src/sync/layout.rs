//! Idempotent synchronization of Strata's container directory layout,
//! standard-XDG environment profiles (`/etc/profile.d/strata-env.sh`, `/etc/environment`),
//! Zsh/Fish environment hooks, ColorOS APT sandbox drop-in, and `/etc/strata/layout.json`.

use std::path::Path;

use serde_json::json;
use strata_protocol::constants::{
    CANONICAL_CONTAINER_ENV, CONTAINER_ETC_ENVIRONMENT_PATH, CONTAINER_LAYOUT_PATH,
    CONTAINER_LAYOUT_SCHEMA_VERSION, CONTAINER_PROFILE_ENV_PATH, CONTAINER_STRATA_ETC_DIR,
    CONTAINER_WORKSPACE_DIR, DEFAULT_CONTAINER_PATH,
};
use strata_sys::rootfs_io::RootDir;

use crate::error::{RuntimeError, RuntimeResult};

const STRATA_PROFILE_BLOCK_START: &str = "# >>> strata-env >>>";
const STRATA_PROFILE_BLOCK_END: &str = "# <<< strata-env <<<";
const CONTAINER_ZSH_ENV_PATH: &str = "/etc/strata/env.zsh";
const CONTAINER_FISH_ENV_REL: &str = "etc/fish/conf.d/00-strata-env.fish";

/// Synchronizes baseline directories, standard-XDG `/etc/profile.d/strata-env.sh`,
/// `/etc/environment`, shell startup hooks (Bash/Zsh/Fish), `/etc/strata/layout.json`,
/// and ColorOS `99strata-sandbox` without forcing `MISE_*` or `UV_*` directory overrides.
pub(crate) fn sync_container_layout(root: &RootDir) -> RuntimeResult<()> {
    for (rel, mode) in [
        ("workspace", 0o755),
        ("etc/strata", 0o755),
        ("etc/strata/mise", 0o755),
        ("etc/profile.d", 0o755),
        ("root", 0o700),
        ("root/.config", 0o700),
        ("root/.local", 0o755),
        ("root/.local/bin", 0o755),
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

    // Sync Zsh environment helper and attach non-invasively when Zsh config dir exists.
    let zsh_env = render_zsh_env_script();
    root.sync_bytes_if_changed(
        zsh_env.as_bytes(),
        Path::new(CONTAINER_ZSH_ENV_PATH.trim_start_matches('/')),
        0o644,
    )
    .map_err(RuntimeError::from_sys)?;
    if root.is_dir(Path::new("etc/zsh")) {
        ensure_zsh_hook(root, Path::new("etc/zsh/zshenv"))?;
    }

    // Sync Fish conf.d snippet when Fish is installed in the container.
    if root.is_dir(Path::new("etc/fish")) {
        root.ensure_dir(Path::new("etc/fish/conf.d"), 0o755)
            .map_err(RuntimeError::from_sys)?;
        let fish_env = render_fish_env_script();
        root.sync_bytes_if_changed(
            fish_env.as_bytes(),
            Path::new(CONTAINER_FISH_ENV_REL),
            0o644,
        )
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
/// Preserves user XDG defaults without overriding `MISE_*` or `UV_*` directories.
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
        "case \":${{PATH:-}}:\" in\n  *\":/root/.local/bin:\"*) ;;\n  *) export PATH=\"{DEFAULT_CONTAINER_PATH}:${{PATH:-}}\" ;;\nesac\n"
    ));
    out.push_str(
        "if [ -r /etc/strata/mise/mirrors.env ]; then\n  . /etc/strata/mise/mirrors.env\nfi\n",
    );
    out.push_str(
        "if [ -r /etc/strata/mise/secrets.env ]; then\n  . /etc/strata/mise/secrets.env\nfi\n",
    );
    out.push_str(
        "if [ -n \"${BASH_VERSION:-}\" ]; then\n  case \"$-\" in\n    *i*) if command -v mise >/dev/null 2>&1; then eval \"$(mise activate bash)\" 2>/dev/null || true; fi ;;\n  esac\nfi\n",
    );
    out
}

/// Renders `/etc/strata/env.zsh` for Zsh sessions (PATH, mirrors/secrets, optional `mise activate`).
#[must_use]
pub fn render_zsh_env_script() -> String {
    format!(
        "# Strata Zsh environment helper (do not edit manually)\n\
export COLORTERM=\"truecolor\"\n\
export XDG_CONFIG_HOME=\"${{XDG_CONFIG_HOME:-/root/.config}}\"\n\
export XDG_DATA_HOME=\"${{XDG_DATA_HOME:-/root/.local/share}}\"\n\
export XDG_CACHE_HOME=\"${{XDG_CACHE_HOME:-/root/.cache}}\"\n\
export XDG_STATE_HOME=\"${{XDG_STATE_HOME:-/root/.local/state}}\"\n\
case \":${{PATH:-}}:\" in\n\
  *\":/root/.local/bin:\"*) ;;\n\
  *) export PATH=\"{DEFAULT_CONTAINER_PATH}:${{PATH:-}}\" ;;\n\
esac\n\
[ -r /etc/strata/mise/mirrors.env ] && source /etc/strata/mise/mirrors.env\n\
[ -r /etc/strata/mise/secrets.env ] && source /etc/strata/mise/secrets.env\n\
if [[ -o interactive ]] && command -v mise >/dev/null 2>&1; then\n\
  eval \"$(mise activate zsh)\" 2>/dev/null || true\n\
fi\n"
    )
}

/// Renders `/etc/fish/conf.d/00-strata-env.fish` for Fish sessions.
#[must_use]
pub fn render_fish_env_script() -> String {
    "\
# Strata Fish environment helper (do not edit manually)
set -gx COLORTERM truecolor
set -q XDG_CONFIG_HOME; or set -gx XDG_CONFIG_HOME /root/.config
set -q XDG_DATA_HOME; or set -gx XDG_DATA_HOME /root/.local/share
set -q XDG_CACHE_HOME; or set -gx XDG_CACHE_HOME /root/.cache
set -q XDG_STATE_HOME; or set -gx XDG_STATE_HOME /root/.local/state
for p in /root/.local/share/mise/shims /root/.local/bin /root/.cargo/bin /opt/strata/mise/shims /opt/strata/bin /usr/local/bin
    if test -d $p
        fish_add_path -g $p
    end
end
if status is-interactive; and type -q mise
    mise activate fish | source
end
"
    .to_owned()
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
        "xdgStandard": true,
        "workspaceDir": CONTAINER_WORKSPACE_DIR,
        "etcDir": CONTAINER_STRATA_ETC_DIR,
        "profileEnv": CONTAINER_PROFILE_ENV_PATH,
        "zshEnv": CONTAINER_ZSH_ENV_PATH,
        "etcEnvironment": CONTAINER_ETC_ENVIRONMENT_PATH,
        "path": DEFAULT_CONTAINER_PATH,
    });
    let mut text = serde_json::to_string_pretty(&payload).unwrap_or_default();
    text.push('\n');
    text
}

fn ensure_profile_hook(root: &RootDir, rel_path: &Path) -> RuntimeResult<()> {
    let hook = format!(
        "{STRATA_PROFILE_BLOCK_START}\n[ -r {CONTAINER_PROFILE_ENV_PATH} ] && . {CONTAINER_PROFILE_ENV_PATH}\n{STRATA_PROFILE_BLOCK_END}\n"
    );
    prepend_idempotent_hook(root, rel_path, &hook)
}

fn ensure_zsh_hook(root: &RootDir, rel_path: &Path) -> RuntimeResult<()> {
    let hook = format!(
        "{STRATA_PROFILE_BLOCK_START}\n[ -r {CONTAINER_ZSH_ENV_PATH} ] && source {CONTAINER_ZSH_ENV_PATH}\n{STRATA_PROFILE_BLOCK_END}\n"
    );
    prepend_idempotent_hook(root, rel_path, &hook)
}

fn prepend_idempotent_hook(root: &RootDir, rel_path: &Path, hook: &str) -> RuntimeResult<()> {
    let existing = match root.read_file(rel_path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
    };
    if existing.starts_with(hook) {
        return Ok(());
    }
    let cleaned = strip_profile_hook(&existing);
    let next = if cleaned.is_empty() {
        hook.to_owned()
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
    fn syncs_xdg_layout_without_forcing_mise_or_uv_overrides() {
        let base = std::env::temp_dir().join(format!("strata-layout-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("root")).expect("create root home");
        fs::create_dir_all(base.join("etc/zsh")).expect("create etc/zsh");
        fs::create_dir_all(base.join("etc/fish")).expect("create etc/fish");
        fs::write(base.join("root/.bashrc"), "# existing bashrc\n").expect("seed bashrc");

        let root = RootDir::open(&base).expect("open root");
        sync_container_layout(&root).expect("first sync");
        sync_container_layout(&root).expect("second sync");

        assert!(base.join("workspace").is_dir());
        assert!(base.join("root/.local/bin").is_dir());
        assert!(base.join("etc/profile.d/strata-env.sh").is_file());
        assert!(base.join("etc/strata/env.zsh").is_file());
        assert!(base.join("etc/zsh/zshenv").is_file());
        assert!(base.join("etc/fish/conf.d/00-strata-env.fish").is_file());
        assert!(base.join("etc/environment").is_file());
        assert!(base.join("etc/strata/layout.json").is_file());
        assert!(!base.join("etc/strata/mise/config.toml").exists());

        let bashrc = fs::read_to_string(base.join("root/.bashrc")).expect("read bashrc");
        assert!(bashrc.contains("# existing bashrc"));
        assert_eq!(bashrc.matches(STRATA_PROFILE_BLOCK_START).count(), 1);

        let profile_env =
            fs::read_to_string(base.join("etc/profile.d/strata-env.sh")).expect("read profile env");
        assert!(profile_env.contains("export COLORTERM=\"truecolor\""));
        assert!(profile_env.contains("command -v mise"));
        assert!(!profile_env.contains("MISE_DATA_DIR"));
        assert!(!profile_env.contains("UV_TOOL_BIN_DIR"));

        let etc_env = fs::read_to_string(base.join("etc/environment")).expect("read etc/env");
        assert!(etc_env.contains("XDG_CONFIG_HOME=\"/root/.config\""));
        assert!(etc_env.contains("COLORTERM=\"truecolor\""));
        assert!(!etc_env.contains("MISE_GLOBAL_CONFIG_FILE"));

        let _ = fs::remove_dir_all(&base);
    }
}
