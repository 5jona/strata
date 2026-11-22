//! Typed JSON-RPC request parameters and results for daemon, container, and session operations.

use serde::{Deserialize, Serialize};

use crate::capabilities::CapProfile;
use crate::config::{BindMount, ContainerConfig};

/// Shared selector parameter targeting an explicit or default container.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContainerSelectorParams {
    pub container: Option<String>,
}

/// Result for `daemon.ping`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub pong: bool,
    pub pid: u32,
    pub uptime_ms: u64,
}

/// Result for `daemon.health`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonHealthResult {
    pub ok: bool,
    pub pid: u32,
    pub uptime_ms: u64,
    pub running_containers: usize,
}

/// Verification counters returned after stopping or tearing down a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCleanupReport {
    pub drained_pids: usize,
    pub unmounted_paths: usize,
    pub detached_loops: usize,
    pub residual_mounts: usize,
    pub residual_loops: usize,
}

/// Parameters for `container.logs`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContainerLogsParams {
    pub container: Option<String>,
    pub lines: Option<usize>,
}

/// Result for `container.logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerLogsResult {
    pub container: String,
    pub log_path: String,
    pub lines: Vec<String>,
}

/// Result for `container.compact`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactResult {
    pub container: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub trimmed_bytes: u64,
}

/// Result for `container.fsck`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsckResult {
    pub container: String,
    pub exit_code: i32,
    pub output: String,
}

/// Parameters for `container.backup`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupParams {
    #[serde(default)]
    pub container: Option<String>,
    pub output_path: String,
}

/// Result for `container.backup`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub container: String,
    pub output_path: String,
    pub bytes_written: u64,
    pub restarted: bool,
}

/// Parameters for `container.configPatch`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContainerConfigPatchParams {
    pub container: Option<String>,
    pub enabled: Option<bool>,
    pub auto_start: Option<bool>,
    pub image_path: Option<String>,
    pub mount_dir: Option<String>,
    pub hostname: Option<String>,
    pub ssh_port: Option<u16>,
    pub host_bridge: Option<bool>,
    pub host_runtime: Option<bool>,
    pub extra_char_devs: Option<Vec<String>>,
    pub extra_mounts: Option<Vec<BindMount>>,
}

/// Result for `container.configGet` and `container.configPatch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerConfigResult {
    pub container: String,
    pub config: ContainerConfig,
}

/// Parameters for `container.sshKeyAdd`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyAddParams {
    #[serde(default)]
    pub container: Option<String>,
    pub public_key: String,
}

/// Result for `container.sshKeyList` and `container.sshKeyAdd`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeysResult {
    pub container: String,
    pub keys: Vec<String>,
    pub added: usize,
}

/// Execution mode for an interactive CLI attach lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    /// Join an already-running container's namespaces via `setns`.
    Attached,
    /// Temporarily mount a stopped container for a foreground oneshot session.
    Oneshot,
    /// Temporarily mount a stopped container with full host `/dev`, `/sys`, `/proc`.
    Debug,
}

/// Parameters for `session.acquire`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionAcquireParams {
    pub container: Option<String>,
    pub debug: bool,
}

/// Lease granted by `stratad` to `strata` CLI for local single-threaded `setns` or `stage2`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachLease {
    pub lease_id: String,
    pub container: String,
    pub mode: SessionMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ns_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mnt_ns_inode: Option<u64>,
    pub cgroup_v2_path: String,
    pub mount_dir: String,
    pub hostname: String,
    pub cap_profile: CapProfile,
    pub host_runtime: bool,
    pub bind_data_local_tmp: bool,
    pub extra_char_devs: Vec<String>,
    pub extra_mounts: Vec<BindMount>,
}

/// Parameters for `session.release`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionReleaseParams {
    pub lease_id: String,
}
