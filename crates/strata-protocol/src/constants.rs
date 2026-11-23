//! Single source of truth for Strata module, path, plugin, and runtime constants.

pub use crate::generated::constants::l5_diag_paths;
pub use crate::generated::constants::paths::*;
pub use crate::generated::constants::protocol::*;

/// Default container PATH when entering or launching processes.
pub const DEFAULT_CONTAINER_PATH: &str =
    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

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
