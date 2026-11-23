//! Active SideVeil `hub.sock` session loop with separated reliable/ephemeral outbound queues.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use strata_protocol::EventEnvelope;
use strata_protocol::sideveil::{
    HUB_EVENT_APPEARANCE_CHANGED, HUB_EVENT_NETWORK_CONTEXT_CHANGED,
    HUB_EVENT_PANEL_VISIBILITY_CHANGED, SIDEVEIL_HUD_DISMISS, SIDEVEIL_HUD_UPDATE_SCENE,
};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::dispatcher::RpcCaller;
use crate::sideveil_hud::build_strata_hud_scene;
use crate::sideveil_seam::bridge::BridgeDeps;
use crate::sideveil_seam::reconnect::wait_shutdown;
use crate::sideveil_seam::wire::{
    InboundHubFrame, encode_backend_event, extract_appearance_is_dark, parse_inbound_frame,
    read_bounded_line,
};

const SIDEVEIL_BRIDGE_CONN_ID: u64 = 0;
const RELIABLE_QUEUE_CAPACITY: usize = 256;
const EPHEMERAL_QUEUE_CAPACITY: usize = 128;
const HUD_REFRESH_INTERVAL: Duration = Duration::from_secs(3);

/// Drives one authenticated SideVeil plugin backend session until disconnect, config change,
/// writer failure, or daemon shutdown. Returns `true` when daemon shutdown was requested.
pub(super) async fn run_connected_session(
    mut reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    mut write_half: tokio::net::unix::OwnedWriteHalf,
    hub_sock: &str,
    plugin_id: &str,
    deps: &mut BridgeDeps,
) -> bool {
    let (reliable_tx, mut reliable_rx) = mpsc::channel::<String>(RELIABLE_QUEUE_CAPACITY);
    let (ephemeral_tx, mut ephemeral_rx) = mpsc::channel::<String>(EPHEMERAL_QUEUE_CAPACITY);

    let mut writer_task = tokio::spawn(async move {
        loop {
            let line = tokio::select! {
                biased;
                maybe_rel = reliable_rx.recv() => match maybe_rel {
                    Some(line) => line,
                    None => break,
                },
                maybe_eph = ephemeral_rx.recv() => match maybe_eph {
                    Some(line) => line,
                    None => break,
                },
            };
            if write_half.write_all(line.as_bytes()).await.is_err()
                || write_half.write_all(b"\n").await.is_err()
                || write_half.flush().await.is_err()
            {
                break;
            }
        }
    });

    let mut events = deps.event_hub.subscribe();
    let mut hud_was_enabled = false;
    let mut hud_ticker = tokio::time::interval(HUD_REFRESH_INTERVAL);
    hud_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    if deps.seam.hud().is_enabled() {
        hud_was_enabled = true;
        push_hud_snapshot(plugin_id, deps, &reliable_tx).await;
    }

    let mut shutdown_requested = false;
    let mut writer_finished = false;
    let mut inbound_buf = Vec::with_capacity(256);

    loop {
        if *deps.shutdown.borrow() {
            shutdown_requested = true;
            if hud_was_enabled {
                send_event(&reliable_tx, SIDEVEIL_HUD_DISMISS, None).await;
            }
            break;
        }

        tokio::select! {
            _ = &mut writer_task => {
                writer_finished = true;
                break;
            }
            _ = wait_shutdown(&mut deps.shutdown) => {
                shutdown_requested = true;
                if hud_was_enabled {
                    send_event(&reliable_tx, SIDEVEIL_HUD_DISMISS, None).await;
                }
                break;
            }
            _ = deps.seam.wait_config_wake() => {
                let latest = deps.config_store.snapshot().await.sideveil;
                if !latest.enabled || latest.plugin_id != plugin_id || latest.hub_sock != hub_sock {
                    if hud_was_enabled {
                        send_event(&reliable_tx, SIDEVEIL_HUD_DISMISS, None).await;
                    }
                    break;
                }
            }
            _ = deps.seam.hud().wait_change() => {
                sync_hud_state(plugin_id, deps, &reliable_tx, &mut hud_was_enabled).await;
            }
            _ = hud_ticker.tick(), if hud_was_enabled => {
                push_hud_snapshot(plugin_id, deps, &reliable_tx).await;
            }
            evt = events.reliable.recv() => {
                match evt {
                    Ok(env) => {
                        let is_state_or_exit =
                            env.event == strata_protocol::rpc::events::CONTAINER_STATE_CHANGED
                                || env.event == strata_protocol::rpc::events::TASK_EXIT;
                        if let Some(json) = encode_backend_event(&env, false)
                            && reliable_tx.send(json).await.is_err()
                        {
                            break;
                        }
                        if is_state_or_exit && hud_was_enabled {
                            push_hud_snapshot(plugin_id, deps, &reliable_tx).await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            evt = events.ephemeral.recv() => {
                match evt {
                    Ok(env) => {
                        if let Some(json) = encode_backend_event(&env, true) {
                            let _ = ephemeral_tx.try_send(json);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            line_res = read_bounded_line(&mut reader, &mut inbound_buf) => {
                match line_res {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match parse_inbound_frame(trimmed) {
                            Some(InboundHubFrame::Rpc(req)) => {
                                let disp = Arc::clone(&deps.dispatcher);
                                let tx = reliable_tx.clone();
                                tokio::spawn(async move {
                                    let resp = disp.handle(RpcCaller::SideVeilBridge, req).await;
                                    if let Ok(json) = serde_json::to_string(&resp) {
                                        let _ = tx.send(json).await;
                                    }
                                });
                            }
                            Some(InboundHubFrame::SystemEvent { event, payload }) => {
                                handle_hub_system_event(&event, payload.as_ref(), deps);
                            }
                            None => {}
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        }
    }

    drop(reliable_tx);
    drop(ephemeral_tx);
    if !writer_finished {
        let _ = tokio::time::timeout(Duration::from_millis(500), writer_task).await;
    }
    deps.manager
        .release_connection_leases(SIDEVEIL_BRIDGE_CONN_ID)
        .await;
    shutdown_requested
}

async fn sync_hud_state(
    plugin_id: &str,
    deps: &BridgeDeps,
    reliable_tx: &mpsc::Sender<String>,
    hud_was_enabled: &mut bool,
) {
    if deps.seam.hud().is_enabled() {
        *hud_was_enabled = true;
        push_hud_snapshot(plugin_id, deps, reliable_tx).await;
    } else if *hud_was_enabled {
        *hud_was_enabled = false;
        send_event(reliable_tx, SIDEVEIL_HUD_DISMISS, None).await;
    }
}

async fn push_hud_snapshot(plugin_id: &str, deps: &BridgeDeps, reliable_tx: &mpsc::Sender<String>) {
    let containers = deps.manager.list_containers().await;
    let active_tasks = deps.task_runner.active_count().await;
    let hud = deps.seam.hud();
    let scene = build_strata_hud_scene(
        plugin_id,
        &containers,
        active_tasks,
        hud.is_click_through(),
        hud.is_dark(),
    );
    send_event(
        reliable_tx,
        SIDEVEIL_HUD_UPDATE_SCENE,
        serde_json::to_value(scene).ok(),
    )
    .await;
}

async fn send_event(reliable_tx: &mpsc::Sender<String>, event: &str, payload: Option<Value>) {
    let env = EventEnvelope::new(event, payload);
    if let Ok(json) = serde_json::to_string(&env) {
        let _ = reliable_tx.send(json).await;
    }
}

fn handle_hub_system_event(event: &str, payload: Option<&Value>, deps: &BridgeDeps) {
    match event {
        HUB_EVENT_APPEARANCE_CHANGED => {
            if let Some(is_dark) = extract_appearance_is_dark(payload) {
                deps.seam.hud().set_dark(is_dark);
            }
        }
        HUB_EVENT_PANEL_VISIBILITY_CHANGED => {
            let visible = payload
                .and_then(|v| v.get("visible"))
                .and_then(Value::as_bool);
            eprintln!("stratad: SideVeil panel visibility changed (visible={visible:?})");
        }
        HUB_EVENT_NETWORK_CONTEXT_CHANGED => {
            eprintln!(
                "stratad: SideVeil reported network context change; refreshing container DNS"
            );
            let manager = Arc::clone(&deps.manager);
            tokio::spawn(async move {
                manager.sync_dns_all().await;
            });
        }
        _ => {}
    }
}
