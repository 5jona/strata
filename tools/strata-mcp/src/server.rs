//! Official `rmcp` MCP ServerHandler exposing 14 converged Strata Android debugging tools.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use serde_json::Value;

use crate::devices::DeviceRegistry;
use crate::handlers::{self, extra, perfetto_reload};
use crate::params::{
    AppManageParams, CdpDebugParams, DeviceExecParams, DeviceManageParams, FileTransferParams,
    KernelProbeParams, LogcatReadParams, ObserveStepParams, PerfettoTraceParams,
    ReloadVerifyParams, SqliteInspectParams, SystemInspectParams, UiInteractParams,
    VisualCaptureParams,
};

#[derive(Debug, Clone)]
pub(crate) struct StrataMcpServer {
    registry: Arc<DeviceRegistry>,
    tool_router: ToolRouter<Self>,
}

impl StrataMcpServer {
    pub(crate) fn new() -> Self {
        Self {
            registry: Arc::new(DeviceRegistry::default()),
            tool_router: Self::tool_router(),
        }
    }
}

fn to_tool_result(mut payload: Value) -> Result<CallToolResult, McpError> {
    let is_err = payload.get("ok").and_then(Value::as_bool) == Some(false);
    let mut blocks: Vec<ContentBlock> = Vec::new();
    let inline = payload
        .as_object_mut()
        .and_then(|o| o.remove("inlineImage"))
        .or_else(|| {
            payload
                .get_mut("visual")
                .and_then(Value::as_object_mut)
                .and_then(|o| o.remove("inlineImage"))
        });

    let text = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_owned());
    blocks.push(ContentBlock::text(text));
    if let Some(img) = inline
        && let (Some(data), Some(mime)) = (img["data"].as_str(), img["mimeType"].as_str())
    {
        blocks.push(ContentBlock::image(data.to_owned(), mime.to_owned()));
    }
    Ok(if is_err {
        CallToolResult::error(blocks)
    } else {
        CallToolResult::success(blocks)
    })
}

#[tool_router]
impl StrataMcpServer {
    #[tool(
        name = "device_manage",
        description = "List discovered devices, select/clear session target, or snapshot Strata daemon + Android window/gfxinfo health.",
        annotations(idempotent_hint = true, open_world_hint = false)
    )]
    async fn device_manage(
        &self,
        Parameters(p): Parameters<DeviceManageParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(handlers::handle_device_manage(&self.registry, p).await)
    }

    #[tool(
        name = "device_exec",
        description = "Execute a shell command on Android host (scope='host', default) or Strata container (scope='container').",
        annotations(destructive_hint = true, open_world_hint = true)
    )]
    async fn device_exec(
        &self,
        Parameters(p): Parameters<DeviceExecParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(handlers::handle_device_exec(&self.registry, p).await)
    }

    #[tool(
        name = "file_transfer",
        description = "Push or pull files over SSH ControlMaster between PC and Strata container or Android host (hostScope=true).",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn file_transfer(
        &self,
        Parameters(p): Parameters<FileTransferParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(handlers::handle_file_transfer(&self.registry, p).await)
    }

    #[tool(
        name = "app_manage",
        description = "Inspect app metadata/memory/gfxinfo, start/stop/clear a package, or push and install an APK via pm install.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn app_manage(
        &self,
        Parameters(p): Parameters<AppManageParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(handlers::handle_app_manage(&self.registry, p).await)
    }

    #[tool(
        name = "ui_interact",
        description = "Dump Android UI tree (compact default), tap a matched element, or replay /dev/uinput multitouch gestures.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn ui_interact(
        &self,
        Parameters(p): Parameters<UiInteractParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(handlers::handle_ui_interact(&self.registry, p).await)
    }

    #[tool(
        name = "visual_capture",
        description = "L3 raw screencap health check (black/blank/solid/blur/SSIM) with optional local pull or direct inline ImageContent.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn visual_capture(
        &self,
        Parameters(p): Parameters<VisualCaptureParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(extra::handle_visual(&self.registry, p).await)
    }

    #[tool(
        name = "logcat_read",
        description = "L2a incremental logcat cursor: set watermark (mark=true) or read deduplicated new entries with crash highlights.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn logcat_read(
        &self,
        Parameters(p): Parameters<LogcatReadParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(handlers::handle_logcat(&self.registry, p).await)
    }

    #[tool(
        name = "cdp_debug",
        description = "L1 WebView CDP over abstract UDS: list targets, eval JS, DOM health audit, console capture, or localhost forward.",
        annotations(open_world_hint = false)
    )]
    async fn cdp_debug(
        &self,
        Parameters(p): Parameters<CdpDebugParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(extra::handle_cdp(&self.registry, p).await)
    }

    #[tool(
        name = "observe_step",
        description = "Atomic L4 step: mark logcat -> action (shell/gesture/cdp/rpc) -> settle -> concurrent logcat, health, CDP, and visual.",
        annotations(open_world_hint = false)
    )]
    async fn observe_step(
        &self,
        Parameters(p): Parameters<ObserveStepParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(extra::handle_observe(&self.registry, p).await)
    }

    #[tool(
        name = "perfetto_trace",
        description = "Capture or manage Android Perfetto ring-buffer traces and run PerfettoSQL analysis on PC via trace_processor_shell.",
        annotations(open_world_hint = false)
    )]
    async fn perfetto_trace(
        &self,
        Parameters(p): Parameters<PerfettoTraceParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(perfetto_reload::handle_perfetto(&self.registry, p).await)
    }

    #[tool(
        name = "sqlite_inspect",
        description = "Read-only SQLite inspection (tables or SELECT/PRAGMA/EXPLAIN/WITH query) on a temporary snapshot.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn sqlite_inspect(
        &self,
        Parameters(p): Parameters<SqliteInspectParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(handlers::handle_sqlite(&self.registry, p).await)
    }

    #[tool(
        name = "system_inspect",
        description = "Read-only L5/system diagnostics: ksu_modules, inputs, sysfs, lsposed_modules, lsposed_logs, pstore, dmesg, tracing.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn system_inspect(
        &self,
        Parameters(p): Parameters<SystemInspectParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(handlers::handle_system(&self.registry, p).await)
    }

    #[tool(
        name = "kernel_probe",
        description = "L5 bounded read-only bpftrace kernel probe preset: guard-audit, net-uid, or sched-lat.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn kernel_probe(
        &self,
        Parameters(p): Parameters<KernelProbeParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(handlers::handle_kernel_probe(&self.registry, p).await)
    }

    #[tool(
        name = "reload_verify",
        description = "SSH hot-reload Strata module (scripts/reload.sh --ssh) followed by non-destructive live verification.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn reload_verify(
        &self,
        Parameters(p): Parameters<ReloadVerifyParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(perfetto_reload::handle_reload_verify(&self.registry, p).await)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for StrataMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("strata-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions("Strata Android device debugging MCP server (14 converged tools).")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_router_exposes_exact_14_converged_tools() {
        let srv = StrataMcpServer::new();
        let tools = srv.tool_router.list_all();
        assert_eq!(tools.len(), 14);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        for expected in [
            "device_manage",
            "device_exec",
            "file_transfer",
            "app_manage",
            "ui_interact",
            "visual_capture",
            "logcat_read",
            "cdp_debug",
            "observe_step",
            "perfetto_trace",
            "sqlite_inspect",
            "system_inspect",
            "kernel_probe",
            "reload_verify",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
    }

    #[test]
    fn lifts_inline_image_into_mcp_image_content_block() {
        let res = to_tool_result(json!({
            "ok": true, "health": {"width": 720},
            "inlineImage": {"mimeType": "image/jpeg", "data": "AAAA"}
        }))
        .expect("result");
        assert_eq!(res.content.len(), 2);
        assert!(res.content[1].as_image().is_some());
    }
}
