//! Facade for the optional auto-reconnecting `uds-native` SideVeil `hub.sock` bridge.

use std::sync::Arc;

use tokio::sync::watch;

use crate::config_store::ConfigStore;
use crate::dispatcher::Dispatcher;
use crate::manager::ContainerManager;
use crate::sideveil_seam::{EventHub, SideVeilBridgeSeam};
use crate::task_runner::TaskRunner;

pub(crate) struct BridgeDeps {
    pub(crate) seam: Arc<SideVeilBridgeSeam>,
    pub(crate) config_store: Arc<ConfigStore>,
    pub(crate) dispatcher: Arc<Dispatcher>,
    pub(crate) manager: Arc<ContainerManager>,
    pub(crate) task_runner: Arc<TaskRunner>,
    pub(crate) event_hub: Arc<EventHub>,
    pub(crate) shutdown: watch::Receiver<bool>,
}

/// Runs the resilient, non-fatal reconnect loop targeting SideVeil's `hub.sock`.
pub(crate) async fn run_bridge_loop(deps: BridgeDeps) {
    super::reconnect::run_reconnect_loop(deps).await;
}
