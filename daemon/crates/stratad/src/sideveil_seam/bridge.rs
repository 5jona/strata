//! Config-aware SideVeil `hub.sock` supervisor backed by `sideveil-plugin-sdk`.

use std::sync::Arc;
use std::time::Duration;

use sideveil_plugin_sdk::{CancellationToken, PluginBackendConfig, PluginBackendRunner};
use strata_protocol::sideveil::SIDEVEIL_HUD_DISMISS;
use tokio::sync::{Notify, watch};

use crate::config_store::ConfigStore;
use crate::dispatcher::Dispatcher;
use crate::manager::ContainerManager;
use crate::sideveil_seam::session::{StrataPluginHandler, run_event_and_hud_pump};
use crate::sideveil_seam::{EventHub, SideVeilBridgeSeam};
use crate::task_runner::TaskRunner;

const MIN_RECONNECT_BACKOFF: Duration = Duration::from_millis(250);
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(15);
const DISABLED_POLL_INTERVAL: Duration = Duration::from_secs(10);
const HUD_DISMISS_FLUSH_GRACE: Duration = Duration::from_millis(25);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
const RELIABLE_QUEUE_CAPACITY: usize = 256;
const EPHEMERAL_QUEUE_CAPACITY: usize = 128;

pub(crate) struct BridgeDeps {
    pub(crate) seam: Arc<SideVeilBridgeSeam>,
    pub(crate) config_store: Arc<ConfigStore>,
    pub(crate) dispatcher: Arc<Dispatcher>,
    pub(crate) manager: Arc<ContainerManager>,
    pub(crate) task_runner: Arc<TaskRunner>,
    pub(crate) event_hub: Arc<EventHub>,
    pub(crate) shutdown: watch::Receiver<bool>,
}

/// Shared daemon handles cloned into the SDK RPC handler and event/HUD pump.
#[derive(Clone)]
pub(super) struct BridgeContext {
    pub(super) seam: Arc<SideVeilBridgeSeam>,
    pub(super) dispatcher: Arc<Dispatcher>,
    pub(super) manager: Arc<ContainerManager>,
    pub(super) task_runner: Arc<TaskRunner>,
}

impl BridgeDeps {
    fn context(&self) -> BridgeContext {
        BridgeContext {
            seam: Arc::clone(&self.seam),
            dispatcher: Arc::clone(&self.dispatcher),
            manager: Arc::clone(&self.manager),
            task_runner: Arc::clone(&self.task_runner),
        }
    }
}

/// Runs the standalone-safe SideVeil plugin bridge lifecycle using `sideveil-plugin-sdk`.
pub(crate) async fn run_bridge_loop(mut deps: BridgeDeps) {
    loop {
        if *deps.shutdown.borrow() {
            deps.seam.set_active_handle(None);
            return;
        }

        let cfg = deps.config_store.snapshot().await.sideveil;
        if !cfg.enabled {
            deps.seam.set_active_handle(None);
            if wait_disabled_or_shutdown(&mut deps).await {
                return;
            }
            continue;
        }

        let sdk_cfg = PluginBackendConfig::new(&cfg.plugin_id)
            .with_socket_path(&cfg.hub_sock)
            .with_backoff(MIN_RECONNECT_BACKOFF, MAX_RECONNECT_BACKOFF)
            .with_queue_capacities(RELIABLE_QUEUE_CAPACITY, EPHEMERAL_QUEUE_CAPACITY);

        let run_cancel = CancellationToken::new();
        let (handle, runner) = match PluginBackendRunner::new(sdk_cfg, run_cancel.clone()) {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!("stratad: invalid SideVeil bridge configuration: {err}");
                deps.seam.set_active_handle(None);
                if wait_disabled_or_shutdown(&mut deps).await {
                    return;
                }
                continue;
            }
        };

        deps.seam.set_active_handle(Some(handle.clone()));

        // Subscribe before starting the runner so immediate post-handshake events are never missed.
        let events = deps.event_hub.subscribe();
        let connected_notify = Arc::new(Notify::new());
        let ctx = deps.context();

        let handler = StrataPluginHandler {
            ctx: ctx.clone(),
            hub_sock: cfg.hub_sock.clone(),
            connected_notify: Arc::clone(&connected_notify),
            run_cancel: run_cancel.clone(),
        };
        let mut runner_task = tokio::spawn(async move {
            let _ = runner.run(handler).await;
        });
        let pump_task = tokio::spawn(run_event_and_hud_pump(
            cfg.plugin_id.clone(),
            ctx,
            handle.clone(),
            events,
            connected_notify,
            run_cancel.clone(),
        ));

        let mut shutdown_requested = false;
        let mut runner_finished = false;
        loop {
            if *deps.shutdown.borrow() {
                shutdown_requested = true;
                break;
            }
            tokio::select! {
                _ = wait_shutdown(&mut deps.shutdown) => {
                    shutdown_requested = true;
                    break;
                }
                _ = deps.seam.wait_config_wake() => {
                    let latest = deps.config_store.snapshot().await.sideveil;
                    if !latest.enabled
                        || latest.plugin_id != cfg.plugin_id
                        || latest.hub_sock != cfg.hub_sock
                    {
                        break;
                    }
                }
                _ = &mut runner_task => {
                    runner_finished = true;
                    break;
                }
            }
        }

        if !runner_finished && handle.is_connected() && deps.seam.hud().is_enabled() {
            let _ = handle.emit_event(SIDEVEIL_HUD_DISMISS, None, false).await;
            tokio::time::sleep(HUD_DISMISS_FLUSH_GRACE).await;
        }

        run_cancel.cancel();
        if !runner_finished {
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, runner_task).await;
        }
        let _ = tokio::time::timeout(DRAIN_TIMEOUT, pump_task).await;
        deps.seam.set_active_handle(None);

        if shutdown_requested {
            return;
        }
    }
}

async fn wait_disabled_or_shutdown(deps: &mut BridgeDeps) -> bool {
    tokio::select! {
        _ = wait_shutdown(&mut deps.shutdown) => true,
        _ = deps.seam.wait_config_wake() => false,
        _ = tokio::time::sleep(DISABLED_POLL_INTERVAL) => false,
    }
}

async fn wait_shutdown(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            return;
        }
    }
}
