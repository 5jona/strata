//! Task streaming, output replay, async maintenance, and optional SideVeil HUD RPC payloads.

use serde::{Deserialize, Serialize};

use crate::state::{ContainerPhase, ContainerStatusSnapshot};

/// Parameters for `task.run` (headless streaming execution managed by `stratad`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRunParams {
    #[serde(default)]
    pub container: Option<String>,
    pub command: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// Result returned immediately by `task.run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRunResult {
    pub task_id: String,
    pub container: String,
    pub pid: u32,
}

/// Parameters for `task.cancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCancelParams {
    pub task_id: String,
}

/// Parameters for `task.logs` (replays buffered stdout/stderr lines for a tracked task).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLogsParams {
    pub task_id: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Summary descriptor returned by `task.list` and `task.logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub task_id: String,
    pub container: String,
    pub pid: u32,
    pub command: Vec<String>,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub started_at_ms: u64,
}

/// Output stream label for `task.output` events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStream {
    Stdout,
    Stderr,
}

/// Payload pushed with `task.output` events and returned by `task.logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskOutputPayload {
    pub task_id: String,
    pub container: String,
    pub stream: TaskStream,
    pub chunk: String,
    pub seq: u64,
}

/// Result returned by `task.logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLogsResult {
    pub summary: TaskSummary,
    pub entries: Vec<TaskOutputPayload>,
}

/// Payload pushed with `task.exit` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskExitPayload {
    pub task_id: String,
    pub container: String,
    pub exit_code: i32,
    pub signal: Option<i32>,
    pub timed_out: bool,
}

/// Payload pushed with `container.stateChanged` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStateChangedPayload {
    pub container: String,
    pub previous: ContainerPhase,
    pub current: ContainerPhase,
    pub snapshot: ContainerStatusSnapshot,
}

/// Immediate acknowledgement returned by `container.backupStart`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStartResult {
    pub started: bool,
    pub container: String,
    pub output_path: String,
}

/// Asynchronous completion event pushed for long-running maintenance operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceDonePayload {
    pub container: String,
    pub op: String,
    pub ok: bool,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Parameters for `hud.set` controlling Strata's optional SideVeil Native HUD summary strip.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HudSetParams {
    pub enabled: Option<bool>,
    pub click_through: Option<bool>,
    pub is_dark: Option<bool>,
}

/// Result returned by `hud.state` and `hud.set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HudStateResult {
    pub enabled: bool,
    pub click_through: bool,
    pub is_dark: bool,
    pub sideveil_connected: bool,
}
