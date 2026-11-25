//! Single source of truth for Strata module, path, plugin, and runtime constants.

pub use crate::generated::constants::l5_diag_paths;
pub use crate::generated::constants::paths::*;
pub use crate::generated::constants::protocol::*;

/// Fallback public DNS resolvers written after active Android DNS properties.
pub const DEFAULT_DNS_SERVERS: [&str; 3] = ["223.5.5.5", "1.1.1.1", "8.8.8.8"];

/// Default character devices passed through to the primary container.
pub const DEFAULT_EXTRA_CHAR_DEVS: [&str; 1] = [DEFAULT_EXTRA_CHAR_DEV];

/// Canonical read-only L5 kernel diagnostic mount paths (opt-in via `strata config set-l5-diag on`).
pub const L5_DIAG_MOUNT_PATHS: [&str; 3] = [
    l5_diag_paths::PSTORE,
    l5_diag_paths::TRACING,
    l5_diag_paths::BPF,
];

/// Canonical baseline environment variables applied identically across container init,
/// `strata enter`/`exec`, `TaskRunner`, `ServiceSupervisor`, and Dropbear SSH.
pub const CANONICAL_CONTAINER_ENV: [(&str, &str); 19] = [
    ("HOME", "/root"),
    ("USER", "root"),
    ("LOGNAME", "root"),
    ("LANG", "C.UTF-8"),
    ("LC_ALL", "C.UTF-8"),
    ("PATH", DEFAULT_CONTAINER_PATH),
    ("XDG_CONFIG_HOME", "/root/.config"),
    ("XDG_DATA_HOME", "/root/.local/share"),
    ("XDG_CACHE_HOME", "/root/.cache"),
    ("XDG_STATE_HOME", "/root/.local/state"),
    ("MISE_DATA_DIR", CONTAINER_MISE_DATA_DIR),
    ("MISE_CACHE_DIR", CONTAINER_MISE_CACHE_DIR),
    ("MISE_CONFIG_DIR", CONTAINER_MISE_CONFIG_DIR),
    ("MISE_GLOBAL_CONFIG_FILE", CONTAINER_MISE_CONFIG_FILE),
    ("UV_CACHE_DIR", CONTAINER_UV_CACHE_DIR),
    ("UV_TOOL_DIR", CONTAINER_UV_TOOL_DIR),
    ("UV_TOOL_BIN_DIR", CONTAINER_OPT_BIN_DIR),
    ("UV_LINK_MODE", "copy"),
    ("PIP_DISABLE_PIP_VERSION_CHECK", "1"),
];
