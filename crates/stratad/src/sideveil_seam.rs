//! Event broadcast hub and optional auto-reconnecting SideVeil `uds-native` plugin bridge.

mod bridge;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use strata_protocol::EventEnvelope;
use strata_protocol::rpc::{HudSetParams, HudStateResult};
use tokio::sync::{Notify, broadcast, watch};

use crate::config_store::ConfigStore;
use crate::dispatcher::Dispatcher;
use crate::manager::ContainerManager;
use crate::sideveil_hud::HudPreferences;
use crate::task_runner::TaskRunner;

const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Transport-agnostic sink invoked whenever container state or streaming tasks emit events.
pub(crate) trait PluginEventSink: Send + Sync {
    fn emit(&self, event: EventEnvelope);
}

/// Broadcast hub fanning out `strata.sock` events and SideVeil `plugin.event` messages.
#[derive(Debug)]
pub(crate) struct EventHub {
    tx: broadcast::Sender<EventEnvelope>,
}

impl EventHub {
    #[must_use]
    pub(crate) fn new() -> Self {
        let (tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self { tx }
    }

    #[must_use]
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.tx.subscribe()
    }
}

impl PluginEventSink for EventHub {
    fn emit(&self, event: EventEnvelope) {
        let _ = self.tx.send(event);
    }
}

/// Optional SideVeil `hub.sock` bridge coordinator.
/// Keeps `stratad` 100% standalone when SideVeil is absent, stopped, or disabled.
#[derive(Debug)]
pub(crate) struct SideVeilBridgeSeam {
    connected: AtomicBool,
    hud: HudPreferences,
    config_wake: Notify,
}

impl SideVeilBridgeSeam {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            connected: AtomicBool::new(false),
            hud: HudPreferences::new(),
            config_wake: Notify::new(),
        }
    }

    #[must_use]
    pub(crate) fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub(crate) fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Relaxed);
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
