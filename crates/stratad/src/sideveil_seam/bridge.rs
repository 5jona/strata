//! Optional auto-reconnecting `uds-native` plugin backend loop for SideVeil `hub.sock`.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use strata_protocol::{EventEnvelope, RpcRequest};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};

use crate::config_store::ConfigStore;
use crate::dispatcher::Dispatcher;
use crate::manager::ContainerManager;
use crate::sideveil_hud::build_strata_hud_scene;
use crate::sideveil_seam::{EventHub, SideVeilBridgeSeam};
use crate::task_runner::TaskRunner;

const SIDEVEIL_BRIDGE_CONN_ID: u64 = 0;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_FRAME_BYTES: usize = 256 * 1024;
const OUTBOUND_QUEUE_CAPACITY: usize = 256;
const HUD_REFRESH_INTERVAL: Duration = Duration::from_secs(3);

const SIDEVEIL_HUD_UPDATE_SCENE: &str = "hud.updateScene";
const SIDEVEIL_HUD_DISMISS: &str = "hud.dismiss";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SideVeilPluginHello<'a> {
    role: &'static str,
    plugin_id: &'a str,
    package_name: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SideVeilHandshakeAck {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SideVeilBackendEvent<'a> {
    event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<&'a Value>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    ephemeral: bool,
}

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
pub(crate) async fn run_bridge_loop(mut deps: BridgeDeps) {
    let mut backoff_secs: u64 = 1;
    let mut logged_missing_socket = false;

    loop {
        if *deps.shutdown.borrow() {
            return;
        }
        let cfg = deps.config_store.snapshot().await.sideveil;
        if !cfg.enabled {
            deps.seam.set_connected(false);
            tokio::select! {
                _ = wait_shutdown(&mut deps.shutdown) => return,
                _ = deps.seam.wait_config_wake() => continue,
                _ = tokio::time::sleep(Duration::from_secs(10)) => continue,
            }
        }

        match connect_and_handshake(&cfg.hub_sock, &cfg.plugin_id).await {
            Ok((reader, write_half)) => {
                backoff_secs = 1;
                logged_missing_socket = false;
                deps.seam.set_connected(true);
                eprintln!(
                    "stratad: connected to SideVeil hub '{}' as plugin '{}'",
                    cfg.hub_sock, cfg.plugin_id
                );

                let shutdown_requested =
                    run_connected_session(reader, write_half, &cfg.plugin_id, &mut deps).await;
                deps.seam.set_connected(false);
                if shutdown_requested {
                    return;
                }
                eprintln!("stratad: SideVeil hub session closed; will reconnect automatically");
            }
            Err(err) => {
                deps.seam.set_connected(false);
                let is_absent = matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                );
                if !is_absent {
                    eprintln!("stratad: SideVeil bridge connect failed: {err}");
                } else if !logged_missing_socket {
                    eprintln!(
                        "stratad: SideVeil hub '{}' unavailable (running standalone, auto-retrying)",
                        cfg.hub_sock
                    );
                    logged_missing_socket = true;
                }
            }
        }

        tokio::select! {
            _ = wait_shutdown(&mut deps.shutdown) => return,
            _ = deps.seam.wait_config_wake() => {
                backoff_secs = 1;
            }
            _ = tokio::time::sleep(Duration::from_secs(backoff_secs)) => {
                backoff_secs = (backoff_secs * 2).min(15);
            }
        }
    }
}

async fn connect_and_handshake(
    hub_sock: &str,
    plugin_id: &str,
) -> io::Result<(
    BufReader<tokio::net::unix::OwnedReadHalf>,
    tokio::net::unix::OwnedWriteHalf,
)> {
    let stream = UnixStream::connect(hub_sock).await?;
    let (read_half, mut write_half) = stream.into_split();
    let hello = SideVeilPluginHello {
        role: "pluginBackend",
        plugin_id,
        package_name: None,
    };
    let mut bytes =
        serde_json::to_vec(&hello).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    bytes.push(b'\n');
    write_half.write_all(&bytes).await?;
    write_half.flush().await?;

    let mut reader = BufReader::new(read_half);
    let ack_line = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_bounded_line(&mut reader))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "SideVeil handshake timed out"))??
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SideVeil closed connection before handshake ack",
            )
        })?;

    let ack: SideVeilHandshakeAck = serde_json::from_str(ack_line.trim())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if !ack.ok {
        return Err(io::Error::other(ack.error.unwrap_or_else(|| {
            "SideVeil rejected plugin handshake".to_owned()
        })));
    }
    Ok((reader, write_half))
}

async fn run_connected_session(
    mut reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    mut write_half: tokio::net::unix::OwnedWriteHalf,
    plugin_id: &str,
    deps: &mut BridgeDeps,
) -> bool {
    let (out_tx, mut out_rx) = mpsc::channel::<String>(OUTBOUND_QUEUE_CAPACITY);

    let writer_task = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if write_half.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if write_half.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = write_half.flush().await;
        }
    });

    let mut event_rx = deps.event_hub.subscribe();
    let mut hud_was_enabled = false;
    let mut hud_ticker = tokio::time::interval(HUD_REFRESH_INTERVAL);
    hud_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    if deps.seam.hud().is_enabled() {
        hud_was_enabled = true;
        push_hud_snapshot(plugin_id, deps, &out_tx).await;
    }

    let mut shutdown_requested = false;

    loop {
        if *deps.shutdown.borrow() {
            shutdown_requested = true;
            if hud_was_enabled {
                send_event(&out_tx, SIDEVEIL_HUD_DISMISS, None).await;
            }
            break;
        }

        tokio::select! {
            _ = wait_shutdown(&mut deps.shutdown) => {
                shutdown_requested = true;
                if hud_was_enabled {
                    send_event(&out_tx, SIDEVEIL_HUD_DISMISS, None).await;
                }
                break;
            }
            _ = deps.seam.wait_config_wake() => {
                let latest = deps.config_store.snapshot().await.sideveil;
                if !latest.enabled || latest.plugin_id != plugin_id {
                    if hud_was_enabled {
                        send_event(&out_tx, SIDEVEIL_HUD_DISMISS, None).await;
                    }
                    break;
                }
            }
            _ = deps.seam.hud().wait_change() => {
                sync_hud_state(plugin_id, deps, &out_tx, &mut hud_was_enabled).await;
            }
            _ = hud_ticker.tick(), if hud_was_enabled => {
                push_hud_snapshot(plugin_id, deps, &out_tx).await;
            }
            evt = event_rx.recv() => {
                match evt {
                    Ok(env) => {
                        let is_state_or_exit = env.event == strata_protocol::rpc::events::CONTAINER_STATE_CHANGED
                            || env.event == strata_protocol::rpc::events::TASK_EXIT;
                        let is_ephemeral = env.event == strata_protocol::rpc::events::TASK_OUTPUT;
                        let frame = SideVeilBackendEvent {
                            event: &env.event,
                            payload: env.payload.as_ref(),
                            ephemeral: is_ephemeral,
                        };
                        if let Ok(json) = serde_json::to_string(&frame)
                            && out_tx.try_send(json).is_err()
                        {
                            // Drop output frames only when local outbound queue is saturated.
                        }
                        if is_state_or_exit && hud_was_enabled {
                            push_hud_snapshot(plugin_id, deps, &out_tx).await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            line_res = read_bounded_line(&mut reader) => {
                match line_res {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(req) = serde_json::from_str::<RpcRequest>(trimmed) {
                            let disp = Arc::clone(&deps.dispatcher);
                            let tx = out_tx.clone();
                            tokio::spawn(async move {
                                let resp = disp.handle(SIDEVEIL_BRIDGE_CONN_ID, req).await;
                                if let Ok(json) = serde_json::to_string(&resp) {
                                    let _ = tx.send(json).await;
                                }
                            });
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        }
    }

    drop(out_tx);
    let _ = tokio::time::timeout(Duration::from_millis(500), writer_task).await;
    shutdown_requested
}

async fn sync_hud_state(
    plugin_id: &str,
    deps: &BridgeDeps,
    out_tx: &mpsc::Sender<String>,
    hud_was_enabled: &mut bool,
) {
    if deps.seam.hud().is_enabled() {
        *hud_was_enabled = true;
        push_hud_snapshot(plugin_id, deps, out_tx).await;
    } else if *hud_was_enabled {
        *hud_was_enabled = false;
        send_event(out_tx, SIDEVEIL_HUD_DISMISS, None).await;
    }
}

async fn push_hud_snapshot(plugin_id: &str, deps: &BridgeDeps, out_tx: &mpsc::Sender<String>) {
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
    send_event(out_tx, SIDEVEIL_HUD_UPDATE_SCENE, Some(scene)).await;
}

async fn send_event(out_tx: &mpsc::Sender<String>, event: &str, payload: Option<Value>) {
    let env = EventEnvelope::new(event, payload);
    if let Ok(json) = serde_json::to_string(&env) {
        let _ = out_tx.send(json).await;
    }
}

async fn wait_shutdown(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            return;
        }
    }
}

async fn read_bounded_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> io::Result<Option<String>> {
    let mut buf = Vec::with_capacity(256);
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if buf.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            if buf.len() + pos > MAX_FRAME_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "SideVeil NDJSON frame exceeds 256 KiB",
                ));
            }
            buf.extend_from_slice(&available[..pos]);
            reader.consume(pos + 1);
            break;
        }
        if buf.len() + available.len() > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SideVeil NDJSON frame exceeds 256 KiB",
            ));
        }
        let len = available.len();
        buf.extend_from_slice(available);
        reader.consume(len);
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
