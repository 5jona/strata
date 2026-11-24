//! SideVeil SDK RPC/system-event handler and dual-channel EventHub + Native HUD publisher.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use sideveil_plugin_sdk::{
    CancellationToken, PluginBackendHandle, PluginRpcContext, PluginRpcError, PluginRpcHandler,
    SdkError,
};
use sideveil_protocol::rpc::RpcRequest as SideVeilRpcRequest;
use strata_protocol::RpcRequest as StrataRpcRequest;
use strata_protocol::rpc::events;
use strata_protocol::sideveil::{
    HUB_EVENT_APPEARANCE_CHANGED, HUB_EVENT_NETWORK_CONTEXT_CHANGED,
    HUB_EVENT_PANEL_VISIBILITY_CHANGED, SIDEVEIL_HUD_DISMISS, SIDEVEIL_HUD_UPDATE_SCENE,
};
use tokio::sync::Notify;
use tokio::sync::broadcast::error::RecvError;

use crate::dispatcher::RpcCaller;
use crate::sideveil_hud::build_strata_hud_scene;
use crate::sideveil_seam::EventSubscription;
use crate::sideveil_seam::bridge::BridgeContext;

const SIDEVEIL_BRIDGE_CONN_ID: u64 = 0;
const HUD_REFRESH_INTERVAL: Duration = Duration::from_secs(3);

/// `sideveil-plugin-sdk` handler routing Hub RPCs and system notifications into `stratad`.
pub(super) struct StrataPluginHandler {
    pub(super) ctx: BridgeContext,
    pub(super) hub_sock: String,
    pub(super) connected_notify: Arc<Notify>,
    pub(super) run_cancel: CancellationToken,
}

impl PluginRpcHandler for StrataPluginHandler {
    async fn handle_rpc(
        &self,
        req: SideVeilRpcRequest,
        _ctx: PluginRpcContext,
    ) -> Result<Value, PluginRpcError> {
        let strata_req = StrataRpcRequest::new(req.id, req.method, req.params);
        let resp = self
            .ctx
            .dispatcher
            .handle(RpcCaller::SideVeilBridge, strata_req)
            .await;
        if let Some(err) = resp.error {
            return Err(PluginRpcError::new(err.code, err.message));
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    async fn on_system_event(&self, event: String, payload: Value, _ctx: PluginRpcContext) {
        match event.as_str() {
            HUB_EVENT_APPEARANCE_CHANGED => {
                if let Some(is_dark) = extract_appearance_is_dark(&payload) {
                    self.ctx.seam.hud().set_dark(is_dark);
                }
            }
            HUB_EVENT_PANEL_VISIBILITY_CHANGED => {}
            HUB_EVENT_NETWORK_CONTEXT_CHANGED => {
                eprintln!(
                    "stratad: SideVeil reported network context change; refreshing container DNS"
                );
                let manager = Arc::clone(&self.ctx.manager);
                tokio::spawn(async move { manager.sync_dns_all().await });
            }
            _ => {}
        }
    }

    async fn on_connected(&self, ctx: PluginRpcContext) {
        eprintln!(
            "stratad: connected to SideVeil hub '{}' as plugin '{}'",
            self.hub_sock,
            ctx.plugin_id()
        );
        self.connected_notify.notify_one();
    }

    async fn on_disconnected(&self, _plugin_id: String) {
        self.ctx
            .manager
            .release_connection_leases(SIDEVEIL_BRIDGE_CONN_ID)
            .await;
        if !self.run_cancel.is_cancelled() {
            eprintln!("stratad: SideVeil hub session closed; will reconnect automatically");
        }
    }
}

/// Forwards dual-channel `EventHub` notifications and Native HUD updates over `PluginBackendHandle`.
pub(super) async fn run_event_and_hud_pump(
    plugin_id: String,
    ctx: BridgeContext,
    handle: PluginBackendHandle,
    mut events_sub: EventSubscription,
    connected_notify: Arc<Notify>,
    cancel: CancellationToken,
) {
    let mut hud_was_enabled = false;
    let mut hud_ticker = tokio::time::interval(HUD_REFRESH_INTERVAL);
    hud_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            () = connected_notify.notified() => {
                if handle.is_connected() {
                    hud_was_enabled = ctx.seam.hud().is_enabled();
                    if hud_was_enabled {
                        push_hud_snapshot(&plugin_id, &ctx, &handle, false).await;
                    }
                }
            }
            () = ctx.seam.hud().wait_change() => {
                if handle.is_connected() {
                    sync_hud_state(&plugin_id, &ctx, &handle, &mut hud_was_enabled).await;
                }
            }
            _ = hud_ticker.tick(), if hud_was_enabled && handle.is_connected() => {
                push_hud_snapshot(&plugin_id, &ctx, &handle, true).await;
            }
            evt = events_sub.reliable.recv() => match evt {
                Ok(env) if handle.is_connected() => {
                    let refresh_hud = hud_was_enabled
                        && (env.event == events::CONTAINER_STATE_CHANGED
                            || env.event == events::TASK_EXIT);
                    if matches!(
                        handle.emit_event(env.event, env.payload, false).await,
                        Err(SdkError::Cancelled | SdkError::ChannelClosed)
                    ) {
                        break;
                    }
                    if refresh_hud {
                        push_hud_snapshot(&plugin_id, &ctx, &handle, false).await;
                    }
                }
                Ok(_) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
            evt = events_sub.ephemeral.recv() => match evt {
                Ok(env) if handle.is_connected() => {
                    let _ = handle.try_emit_event(env.event, env.payload, true);
                }
                Ok(_) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
        }
    }
}

async fn sync_hud_state(
    plugin_id: &str,
    ctx: &BridgeContext,
    handle: &PluginBackendHandle,
    hud_was_enabled: &mut bool,
) {
    if ctx.seam.hud().is_enabled() {
        *hud_was_enabled = true;
        push_hud_snapshot(plugin_id, ctx, handle, false).await;
    } else if *hud_was_enabled {
        *hud_was_enabled = false;
        let _ = handle.emit_event(SIDEVEIL_HUD_DISMISS, None, false).await;
    }
}

async fn push_hud_snapshot(
    plugin_id: &str,
    ctx: &BridgeContext,
    handle: &PluginBackendHandle,
    ephemeral: bool,
) {
    let containers = ctx.manager.list_containers().await;
    let active_tasks = ctx.task_runner.active_count().await;
    let hud = ctx.seam.hud();
    let scene = build_strata_hud_scene(
        plugin_id,
        &containers,
        active_tasks,
        hud.is_click_through(),
        hud.is_dark(),
    );
    let _ = handle
        .emit_event(
            SIDEVEIL_HUD_UPDATE_SCENE,
            serde_json::to_value(scene).ok(),
            ephemeral,
        )
        .await;
}

/// Extracts dark/light theme preference from a SideVeil `appearance.changed` event payload.
pub(super) fn extract_appearance_is_dark(val: &Value) -> Option<bool> {
    if let Some(flag) = val
        .get("isDark")
        .or_else(|| val.get("dark"))
        .and_then(Value::as_bool)
    {
        return Some(flag);
    }
    for key in ["mode", "theme", "appearance"] {
        if let Some(raw) = val.get(key).and_then(Value::as_str) {
            if raw.eq_ignore_ascii_case("dark") {
                return Some(true);
            }
            if raw.eq_ignore_ascii_case("light") {
                return Some(false);
            }
        }
    }
    for nested_key in ["appearance", "theme"] {
        if let Some(nested) = val.get(nested_key).filter(|v| v.is_object())
            && let Some(flag) = extract_appearance_is_dark(nested)
        {
            return Some(flag);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sideveil_plugin_sdk::codec::decode_json_line;
    use sideveil_protocol::uds::BackendOutboundFrame;

    use super::*;

    #[test]
    fn decodes_sdk_hub_frames_and_extracts_appearance_payloads() {
        let rpc_line = r#"{"id":"rpc_1","method":"daemon.ping"}"#;
        assert!(matches!(
            decode_json_line::<BackendOutboundFrame>(rpc_line),
            Ok(BackendOutboundFrame::Rpc(req)) if req.id == "rpc_1" && req.method == "daemon.ping"
        ));

        for (raw, expected_event) in [
            (
                r#"{"event":"appearance.changed","payload":{"isDark":false}}"#,
                HUB_EVENT_APPEARANCE_CHANGED,
            ),
            (
                r#"{"event":"network.contextChanged","payload":{"transport":"wifi"}}"#,
                HUB_EVENT_NETWORK_CONTEXT_CHANGED,
            ),
            (
                r#"{"event":"panel.visibilityChanged","payload":{"visible":true}}"#,
                HUB_EVENT_PANEL_VISIBILITY_CHANGED,
            ),
        ] {
            match decode_json_line::<BackendOutboundFrame>(raw) {
                Ok(BackendOutboundFrame::SystemEvent { event, payload }) => {
                    assert_eq!(event, expected_event);
                    if event == HUB_EVENT_APPEARANCE_CHANGED {
                        assert_eq!(extract_appearance_is_dark(&payload), Some(false));
                    }
                }
                other => panic!("expected SystemEvent for {raw}, got {other:?}"),
            }
        }

        assert_eq!(
            extract_appearance_is_dark(&json!({ "mode": "dark" })),
            Some(true)
        );
        assert_eq!(
            extract_appearance_is_dark(&json!({ "appearance": { "isDark": false } })),
            Some(false)
        );
        assert_eq!(extract_appearance_is_dark(&json!({})), None);
    }
}
