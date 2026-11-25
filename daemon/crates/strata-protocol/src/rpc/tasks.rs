//! Task streaming, output replay, async maintenance, and optional SideVeil HUD RPC payloads.

pub use crate::generated::rpc::{
    HudSetParams, HudStateResult, TaskCancelParams, TaskCancelResult, TaskExitPayload,
    TaskLogsParams, TaskLogsResult, TaskOutputPayload, TaskRunParams, TaskRunResult, TaskStream,
    TaskSummary,
};
pub use crate::generated::state::{
    BackupStartResult, ContainerStateChangedPayload, MaintenanceDonePayload,
};
