//! Event broadcast hub and optional auto-reconnecting SideVeil `uds-native` plugin bridge.

mod bridge;
mod session;
#[cfg(test)]
mod tests;

use std::sync::{Arc, RwLock};

use sideveil_plugin_sdk::PluginBackendHandle;
use strata_protocol::EventEnvelope;
use strata_protocol::rpc::{HudSetParams, HudStateResult, events};
use tokio::sync::{Notify, broadcast, watch};

use crate::config_store::ConfigStore;
use crate::dispatcher::Dispatcher;
use crate::manager::ContainerManager;
use crate::sideveil_hud::HudPreferences;
use crate::task_runner::TaskRunner;

const RELIABLE_EVENT_CAPACITY: usize = 256;
const EPHEMERAL_EVENT_CAPACITY: usize = 256;

/// Returns true for high-volume streaming events that may drop under backpressure.
#[must_use]
pub(crate) fn is_ephemeral_event(event: &str) -> bool {
    event == events::TASK_OUTPUT
}

/// Transport-agnostic sink invoked whenever container state or streaming tasks emit events.
pub(crate) trait PluginEventSink: Send + Sync {
    fn emit(&self, event: EventEnvelope);
}

/// Dual-channel subscription separating reliable control events from ephemeral streaming output.
pub(crate) struct EventSubscription {
    pub(crate) reliable: broadcast::Receiver<EventEnvelope>,
    pub(crate) ephemeral: broadcast::Receiver<EventEnvelope>,
}

/// Broadcast hub fanning out `strata.sock` events and SideVeil `plugin.event` messages.
#[derive(Debug)]
pub(crate) struct EventHub {
    reliable_tx: broadcast::Sender<EventEnvelope>,
    ephemeral_tx: broadcast::Sender<EventEnvelope>,
}

impl EventHub {
    #[must_use]
    pub(crate) fn new() -> Self {
        let (reliable_tx, _) = broadcast::channel(RELIABLE_EVENT_CAPACITY);
        let (ephemeral_tx, _) = broadcast::channel(EPHEMERAL_EVENT_CAPACITY);
        Self {
            reliable_tx,
            ephemeral_tx,
        }
    }

    #[must_use]
    pub(crate) fn subscribe(&self) -> EventSubscription {
        EventSubscription {
            reliable: self.reliable_tx.subscribe(),
            ephemeral: self.ephemeral_tx.subscribe(),
        }
    }
}

impl PluginEventSink for EventHub {
    fn emit(&self, event: EventEnvelope) {
        if is_ephemeral_event(&event.event) {
            let _ = self.ephemeral_tx.send(event);
        } else {
            let _ = self.reliable_tx.send(event);
        }
    }
}

/// Optional SideVeil `hub.sock` bridge coordinator.
/// Keeps `stratad` 100% standalone when SideVeil is absent, stopped, or disabled.
#[derive(Debug)]
pub(crate) struct SideVeilBridgeSeam {
    active_handle: RwLock<Option<PluginBackendHandle>>,
    hud: HudPreferences,
    config_wake: Notify,
}

impl SideVeilBridgeSeam {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            active_handle: RwLock::new(None),
            hud: HudPreferences::new(),
            config_wake: Notify::new(),
        }
    }

    #[must_use]
    pub(crate) fn is_connected(&self) -> bool {
        self.active_handle
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().map(PluginBackendHandle::is_connected))
            .unwrap_or(false)
    }

    pub(crate) fn set_active_handle(&self, handle: Option<PluginBackendHandle>) {
        if let Ok(mut guard) = self.active_handle.write() {
            *guard = handle;
        }
    }

    #[must_use]
    pub(crate) fn hud(&self) -> &HudPreferences {
        &self.hud
    }

    #[must_use]
    pub(crate) fn hud_state(&self) -> HudStateResult {
        self.hud.snapshot(self.is_connected())
    }

    #[must_use]
    pub(crate) fn update_hud(&self, params: HudSetParams) -> HudStateResult {
        self.hud.apply(params, self.is_connected())
    }

    /// Wakes the bridge task after configuration reloads so `sideveil.enabled` takes effect promptly.
    pub(crate) fn notify_config_changed(&self) {
        self.config_wake.notify_one();
    }

    pub(crate) async fn wait_config_wake(&self) {
        self.config_wake.notified().await;
    }

    /// Spawns the optional auto-reconnecting `hub.sock` plugin backend bridge task.
    pub(crate) fn spawn_bridge(
        self: &Arc<Self>,
        config_store: Arc<ConfigStore>,
        dispatcher: Arc<Dispatcher>,
        manager: Arc<ContainerManager>,
        task_runner: Arc<TaskRunner>,
        event_hub: Arc<EventHub>,
        shutdown: watch::Receiver<bool>,
    ) -> tokio::task::JoinHandle<()> {
        let deps = bridge::BridgeDeps {
            seam: Arc::clone(self),
            config_store,
            dispatcher,
            manager,
            task_runner,
            event_hub,
            shutdown,
        };
        tokio::spawn(bridge::run_bridge_loop(deps))
    }
}
