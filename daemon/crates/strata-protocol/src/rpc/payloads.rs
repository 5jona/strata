//! Typed JSON-RPC request parameters and results for daemon, container, and session operations.

pub use crate::generated::config::{
    AttachLease, ContainerConfigPatchParams, ContainerConfigResult, ContainerServiceRuntime,
    ContainerServiceSpec, ServiceDeleteResult, ServiceLogsParams, ServiceLogsResult,
    ServiceRestartPolicy, ServiceSelectorParams, ServiceStateChangedPayload, ServiceStatus,
    ServiceUpsertParams, SessionAcquireParams, SessionMode, SessionReleaseParams,
    SessionReleaseResult,
};
pub use crate::generated::rpc::{
    ContainerCloneParams, ContainerCloneResult, ContainerCreateParams, ContainerJobCancelResult,
    ContainerJobKind, ContainerJobProgressPayload, ContainerJobSelectorParams,
    ContainerJobStartResult, ContainerJobState, ContainerLogsParams, ContainerLogsResult,
    ContainerResizeParams, ContainerResizeResult, ContainerSelectorParams, DaemonHealthResult,
    DaemonLogsParams, DaemonLogsResult, DistroCatalogParams, DistroCatalogResult, DistroEntry,
    DistroMirror, EventsSubscribeResult, InstalledToolItem, KernelCapabilitiesResult, PingResult,
    SshKeyAddParams, SshKeyRemoveParams, SshKeyRemoveResult, SshKeysResult, ToolBundleSpec,
    ToolCatalogEntry, ToolDiagnosticItem, ToolDiagnosticStatus, ToolGroupKind, ToolManagerHealth,
    ToolManagerKind, ToolMirrorPreset, ToolsBootstrapParams, ToolsBootstrapProfile,
    ToolsCacheCleanParams, ToolsCacheCleanResult, ToolsCatalogParams, ToolsCatalogResult,
    ToolsConfigGetParams, ToolsConfigResult, ToolsConfigSetParams, ToolsDoctorParams,
    ToolsDoctorResult, ToolsInstallParams, ToolsListParams, ToolsListResult,
    ToolsStateChangedPayload, ToolsStatusParams, ToolsStatusResult, ToolsUninstallParams,
    ToolsUpgradeParams, WebConfigureParams, WebRotateTokenResult, WebStatusResult,
};
pub use crate::generated::state::{
    BackupParams, BackupResult, CompactResult, ContainerDeleteParams, ContainerDeleteResult,
    FsckResult, StorageCleanupReport, SyncDnsResult,
};
