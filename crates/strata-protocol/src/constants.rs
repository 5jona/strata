//! Single source of truth for Strata module, path, plugin, and runtime constants.

/// KernelSU module identifier.
pub const MODULE_ID: &str = "strata";

/// Default primary container name when omitted by CLI or RPC callers.
pub const DEFAULT_CONTAINER_ID: &str = "main";

/// SideVeil plugin identifier reserved for Phase 2 `uds-native` integration.
pub const SIDEVEIL_PLUGIN_ID: &str = "strata.manager";

/// Default SideVeil hub Unix domain socket path.
pub const DEFAULT_SIDEVEIL_HUB_SOCK: &str = "/data/system/sideveil/run/hub.sock";

/// Authoritative multi-container configuration file on the Android host.
pub const DEFAULT_CONFIG_PATH: &str = "/data/adb/strata/config.json";

/// Root-only (`0700`) runtime state directory on the Android host.
pub const DEFAULT_RUN_DIR: &str = "/data/adb/strata/run";

/// Control plane Unix domain socket path (`SO_PEERCRED` UID 0 authenticated).
pub const DEFAULT_SOCKET_PATH: &str = "/data/adb/strata/run/strata.sock";

/// Host directory storing rotated per-container init/Dropbear logs.
pub const DEFAULT_LOGS_DIR: &str = "/data/adb/strata/run/logs";

/// Neutral host directory storing container rootfs images and mountpoints.
pub const DEFAULT_CONTAINERS_ROOT: &str = "/data/local/strata/containers";

/// In-container runtime directory for host environment snapshots and PID files.
pub const CONTAINER_RUN_DIR: &str = "/run/strata";

/// In-container path for the synchronized Android PID 1 environment snapshot.
pub const CONTAINER_HOST_ENV_PATH: &str = "/run/strata/host.env";

/// In-container PID file written by Dropbear.
pub const CONTAINER_DROPBEAR_PID_PATH: &str = "/run/strata/dropbear.pid";

/// SELinux label required on Android `/data` for ext4 loop-backed images.
pub const SELINUX_IMAGE_CONTEXT: &str = "u:object_r:vold_data_file:s0";

/// Unified cgroup v2 mount root on Android 16+.
pub const CGROUP_V2_ROOT: &str = "/sys/fs/cgroup";

/// Subtree name under cgroup v2 dedicated to Strata containers.
pub const CGROUP_V2_NAMESPACE: &str = "strata";

/// Default container PATH when entering or launching processes.
pub const DEFAULT_CONTAINER_PATH: &str =
    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Fallback public DNS resolvers written after active Android DNS properties.
pub const DEFAULT_DNS_SERVERS: [&str; 3] = ["223.5.5.5", "1.1.1.1", "8.8.8.8"];

/// Maximum log file size in bytes before rotation (256 KiB).
pub const DEFAULT_LOG_MAX_BYTES: u64 = 256 * 1024;

/// Configuration schema version.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;
