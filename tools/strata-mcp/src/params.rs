//! Compact, token-efficient MCP JSON Schema parameter types for the 14 Strata debugging tools.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::transport::TargetOverride;

/// Shared optional device targeting overrides embedded across device-bound tools.
#[derive(Debug, Default, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TargetArgs {
    /// Device alias, ADB serial, or model; specify explicitly when multiple devices are online.
    pub(crate) device: Option<String>,
    /// Explicit SSH host override.
    pub(crate) host: Option<String>,
    /// Explicit SSH port override.
    pub(crate) port: Option<u16>,
}

impl TargetArgs {
    pub(crate) fn to_override(&self) -> TargetOverride {
        TargetOverride {
            device: self.device.clone(),
            host: self.host.clone(),
            port: self.port,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceManageParams {
    /// Action: "list" | "select" | "health" | "containers" | "start_container" | "stop_container" | "start_daemon".
    pub(crate) action: String,
    /// Device alias/serial/model (for select, health, or container/daemon ops).
    pub(crate) device: Option<String>,
    /// Clear session selected device (when action="select").
    pub(crate) clear: Option<bool>,
    /// Package name or "auto" (when action="health").
    pub(crate) package: Option<String>,
    /// Optional container id for start_container/stop_container (defaults to default/first container).
    pub(crate) container: Option<String>,
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceExecParams {
    pub(crate) command: String,
    /// Execution scope: "host" (Android PID 1) or "container" (Strata Linux). Defaults to "host".
    pub(crate) scope: Option<String>,
    pub(crate) timeout_sec: Option<f64>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileTransferParams {
    /// Transfer direction: "push" | "pull".
    pub(crate) direction: String,
    pub(crate) local_path: String,
    pub(crate) remote_path: String,
    /// When true, reads/writes Android host filesystem via host-exec instead of container rootfs.
    pub(crate) host_scope: Option<bool>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppManageParams {
    /// Action: "inspect" | "start" | "stop" | "clear" | "install".
    pub(crate) action: String,
    pub(crate) package: Option<String>,
    pub(crate) top_only: Option<bool>,
    pub(crate) activity: Option<String>,
    pub(crate) intent_action: Option<String>,
    pub(crate) data_uri: Option<String>,
    pub(crate) local_path: Option<String>,
    pub(crate) remote_path: Option<String>,
    pub(crate) grant_permissions: Option<bool>,
    pub(crate) allow_downgrade: Option<bool>,
    pub(crate) replace: Option<bool>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UiInteractParams {
    /// Action: "dump" | "tap" | "gesture".
    pub(crate) action: String,
    pub(crate) query: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) resource_id: Option<String>,
    pub(crate) desc: Option<String>,
    pub(crate) clickable_only: Option<bool>,
    pub(crate) compact: Option<bool>,
    pub(crate) compressed: Option<bool>,
    pub(crate) limit: Option<usize>,
    pub(crate) index: Option<usize>,
    /// Tap backend for action="tap": "input" | "uinput".
    pub(crate) method: Option<String>,
    /// Gesture kind for action="gesture": "tap" | "long-press" | "swipe" | "edge-swipe".
    pub(crate) kind: Option<String>,
    pub(crate) side: Option<String>,
    pub(crate) y_percent: Option<f64>,
    pub(crate) distance_dp: Option<f64>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) cancel_back: Option<bool>,
    pub(crate) x: Option<i32>,
    pub(crate) y: Option<i32>,
    pub(crate) x0: Option<i32>,
    pub(crate) y0: Option<i32>,
    pub(crate) x1: Option<i32>,
    pub(crate) y1: Option<i32>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VisualCaptureParams {
    /// Optional crop ROI "x,y,w,h".
    pub(crate) roi: Option<String>,
    /// Image format: "jpeg" | "webp" | "png".
    pub(crate) format: Option<String>,
    pub(crate) quality: Option<u8>,
    pub(crate) max_width: Option<u32>,
    pub(crate) golden: Option<String>,
    pub(crate) burst_sheet: Option<bool>,
    pub(crate) pull_to_local: Option<bool>,
    pub(crate) local_out: Option<String>,
    /// Return a downscaled MCP ImageContent block directly in the tool response.
    pub(crate) return_image: Option<bool>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogcatReadParams {
    pub(crate) mark: Option<bool>,
    pub(crate) cursor: Option<String>,
    pub(crate) priority: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) pid: Option<u32>,
    pub(crate) limit: Option<usize>,
    pub(crate) no_advance: Option<bool>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CdpDebugParams {
    /// Action: "list" | "eval" | "audit" | "console" | "forward".
    pub(crate) action: String,
    pub(crate) expression: Option<String>,
    pub(crate) target_app: Option<String>,
    pub(crate) pid: Option<u32>,
    pub(crate) url_match: Option<String>,
    pub(crate) eval_js: Option<String>,
    pub(crate) duration_sec: Option<f64>,
    pub(crate) timeout_sec: Option<f64>,
    pub(crate) local_port: Option<u16>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObserveStepParams {
    pub(crate) action_rpc: Option<Vec<String>>,
    pub(crate) action_shell: Option<String>,
    pub(crate) action_cdp_eval: Option<String>,
    /// Optional gesture action as a JSON object, JSON string, or shorthand:
    /// {"kind":"tap","x":100,"y":200} | {"kind":"long-press","x":100,"y":200,"durationMs":600} |
    /// {"kind":"swipe","x0":10,"y0":336,"x1":900,"y1":336,"durationMs":240} |
    /// {"kind":"edge-swipe","side":"left"|"right","yPercent":50,"distanceDp":96,"durationMs":240,"cancelBack":false} |
    /// shorthands: "edge-swipe", "edge-swipe:left|right", "tap:x,y", "long-press:x,y[,ms]", "swipe:x0,y0,x1,y1[,ms]".
    pub(crate) action_gesture: Option<Value>,
    /// Optional sequential multi-step action array: [{"kind":"tap","x":100,"y":200}, {"kind":"swipe","x0":...,"y0":...,"x1":...,"y1":...,"durationMs":240}, {"kind":"shell","command":"..."}, {"kind":"sleep","ms":200}].
    pub(crate) actions: Option<Vec<Value>>,
    /// Optional UI text to wait for after actions complete (polls up to settleMs, max 5000ms).
    pub(crate) wait_for_text: Option<String>,
    pub(crate) cursor: Option<String>,
    pub(crate) settle_ms: Option<u64>,
    pub(crate) package: Option<String>,
    pub(crate) priority: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) include_cdp: Option<bool>,
    pub(crate) include_visual: Option<bool>,
    pub(crate) return_image: Option<bool>,
    pub(crate) roi: Option<String>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerfettoTraceParams {
    /// Action: "capture" | "start" | "status" | "snapshot" | "stop" | "analyze".
    pub(crate) action: Option<String>,
    /// Capture preset: "ui-jank" | "sched" | "binder" | "memory".
    pub(crate) preset: Option<String>,
    pub(crate) duration_sec: Option<u32>,
    pub(crate) buffer_kb: Option<u32>,
    pub(crate) session_key: Option<String>,
    pub(crate) max_duration_sec: Option<u32>,
    pub(crate) remote_path: Option<String>,
    pub(crate) local_path: Option<String>,
    pub(crate) pull_to_local: Option<bool>,
    pub(crate) local_out: Option<String>,
    /// PC trace_processor preset: "summary" | "ui-jank" | "startup" | "binder" | "cpu".
    pub(crate) analysis_preset: Option<String>,
    /// Optional custom read-only PerfettoSQL query run on PC trace_processor_shell.
    pub(crate) sql: Option<String>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SqliteInspectParams {
    pub(crate) db_path: String,
    /// Mode: "tables" | "query".
    pub(crate) mode: Option<String>,
    pub(crate) sql: Option<String>,
    pub(crate) limit: Option<usize>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemInspectParams {
    /// Mode: "ksu_modules" | "inputs" | "sysfs" | "lsposed_modules" | "lsposed_logs" | "pstore" | "dmesg" | "tracing".
    pub(crate) mode: String,
    pub(crate) path: Option<String>,
    pub(crate) module: Option<String>,
    pub(crate) grep: Option<String>,
    pub(crate) filter: Option<String>,
    pub(crate) dir: Option<String>,
    pub(crate) limit: Option<usize>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KernelProbeParams {
    /// Preset: "guard-audit" | "net-uid" | "sched-lat".
    pub(crate) preset: String,
    pub(crate) duration_sec: Option<u64>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReloadVerifyParams {
    pub(crate) skip_build: Option<bool>,
    pub(crate) ui_only: Option<bool>,
    #[serde(flatten)]
    pub(crate) target: TargetArgs,
}
