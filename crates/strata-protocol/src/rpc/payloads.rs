//! Typed JSON-RPC request parameters and results for daemon, container, and session operations.

pub use crate::generated::config::{
    AttachLease, ContainerConfigPatchParams, ContainerConfigResult, SessionAcquireParams,
    SessionMode, SessionReleaseParams, SessionReleaseResult,
};
pub use crate::generated::rpc::{
    ContainerLogsParams, ContainerLogsResult, ContainerSelectorParams, DaemonHealthResult,
    EventsSubscribeResult, PingResult, SshKeyAddParams, SshKeysResult,
};
pub use crate::generated::state::{
    BackupParams, BackupResult, CompactResult, FsckResult, StorageCleanupReport, SyncDnsResult,
};
