//! Host-side Tokio integration tests for the SideVeil `hub.sock` plugin backend bridge.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use strata_protocol::rpc::{HudSetParams, error_codes, events, methods};
use strata_protocol::{EventEnvelope, RpcRequest, RpcResponse, SideVeilBridgeConfig, StrataConfig};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::watch;

use crate::config_store::ConfigStore;
use crate::dispatcher::{Dispatcher, RpcCaller};
use crate::manager::ContainerManager;
use crate::sideveil_seam::{EventHub, PluginEventSink, SideVeilBridgeSeam};
use crate::task_runner::TaskRunner;

struct TestStack {
    config_store: Arc<ConfigStore>,
    event_hub: Arc<EventHub>,
    sideveil: Arc<SideVeilBridgeSeam>,
    manager: Arc<ContainerManager>,
    task_runner: Arc<TaskRunner>,
    dispatcher: Arc<Dispatcher>,
}

fn build_test_stack(base: &std::path::Path, cfg_path: PathBuf) -> TestStack {
    let config_store = Arc::new(ConfigStore::load_or_seed(cfg_path).expect("load config store"));
    let event_hub = Arc::new(EventHub::new());
    let sideveil = Arc::new(SideVeilBridgeSeam::new());
    let manager = Arc::new(ContainerManager::new(
        Arc::clone(&config_store),
        Arc::clone(&event_hub),
        PathBuf::from("/bin/true"),
        base.join("logs"),
        None,
    ));
    let task_runner = Arc::new(TaskRunner::new(
        Arc::clone(&manager),
        Arc::clone(&event_hub),
        PathBuf::from("/bin/true"),
    ));
    let dispatcher = Arc::new(Dispatcher::new(
        Instant::now(),
        Arc::clone(&config_store),
        Arc::clone(&manager),
        Arc::clone(&task_runner),
        Arc::clone(&sideveil),
    ));
    TestStack {
        config_store,
        event_hub,
        sideveil,
        manager,
        task_runner,
        dispatcher,
    }
}

async fn send_raw(w: &mut tokio::net::unix::OwnedWriteHalf, data: &str) {
    w.write_all(data.as_bytes()).await.expect("write frame");
    w.flush().await.expect("flush frame");
}

async fn recv_json(r: &mut BufReader<tokio::net::unix::OwnedReadHalf>) -> Value {
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(2), r.read_line(&mut line))
        .await
        .expect("read frame timeout")
        .expect("read frame");
    serde_json::from_str(line.trim()).expect("parse json frame")
}

#[tokio::test]
async fn sideveil_bridge_handshakes_handles_rpc_forwards_events_and_pushes_hud() {
    let base = std::env::temp_dir().join(format!("strata-sv-bridge-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create temp test dir");
    let sock_path = base.join("hub.sock");
    let cfg_path = base.join("config.json");

    let cfg = StrataConfig {
        sideveil: SideVeilBridgeConfig {
            enabled: true,
            hub_sock: sock_path.to_string_lossy().into_owned(),
            plugin_id: "strata.manager".to_owned(),
        },
        ..StrataConfig::default()
    };
    fs::write(
        &cfg_path,
        serde_json::to_string_pretty(&cfg).expect("serialize cfg"),
    )
    .expect("write cfg");

    let listener = UnixListener::bind(&sock_path).expect("bind mock hub.sock");
    let stack = build_test_stack(&base, cfg_path);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let bridge_handle = stack.sideveil.spawn_bridge(
        Arc::clone(&stack.config_store),
        Arc::clone(&stack.dispatcher),
        Arc::clone(&stack.manager),
        Arc::clone(&stack.task_runner),
        Arc::clone(&stack.event_hub),
        shutdown_rx,
    );

    let (stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
        .await
        .expect("bridge connects within 3s")
        .expect("accept mock hub connection");

    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    // 1. Verify SideVeil PluginBackend hello handshake.
    let hello = recv_json(&mut reader).await;
    assert_eq!(hello["role"], "pluginBackend");
    assert_eq!(hello["pluginId"], "strata.manager");
    send_raw(&mut write_half, "{\"ok\":true}\n").await;

    // 2. Verify multiplexed JSON-RPC request -> response over hub.sock.
    let ping_req = RpcRequest::new("bk_1", methods::DAEMON_PING, None);
    let req_line = format!("{}\n", serde_json::to_string(&ping_req).expect("encode"));
    send_raw(&mut write_half, &req_line).await;

    let resp: RpcResponse = serde_json::from_value(recv_json(&mut reader).await).expect("resp");
    assert_eq!(resp.id, "bk_1");
    assert_eq!(resp.result.expect("ping result")["pong"], true);
    assert!(stack.sideveil.is_connected());

    // 3. Verify EventHub -> SideVeil HostEventEnvelope forwarding.
    stack.event_hub.emit(EventEnvelope::new(
        "task.output",
        Some(json!({ "taskId": "task-1", "chunk": "hello" })),
    ));
    let evt: EventEnvelope = serde_json::from_value(recv_json(&mut reader).await).expect("event");
    assert_eq!(evt.event, "task.output");
    assert_eq!(evt.payload.expect("event payload")["chunk"], "hello");

    // 4. Verify enabling Native HUD pushes `hud.updateScene` and disabling pushes `hud.dismiss`.
    let hud_req = RpcRequest::new(
        "bk_2",
        methods::HUD_SET,
        Some(
            serde_json::to_value(HudSetParams {
                enabled: Some(true),
                click_through: Some(true),
                is_dark: Some(true),
            })
            .expect("encode hud params"),
        ),
    );
    let hud_line = format!("{}\n", serde_json::to_string(&hud_req).expect("encode"));
    send_raw(&mut write_half, &hud_line).await;

    let (mut saw_hud_resp, mut saw_hud_scene) = (false, false);
    for _ in 0..4 {
        let val = recv_json(&mut reader).await;
        if val.get("id").and_then(Value::as_str) == Some("bk_2") {
            assert_eq!(val["result"]["enabled"], true);
            saw_hud_resp = true;
        } else if val.get("event").and_then(Value::as_str) == Some("hud.updateScene") {
            assert_eq!(val["payload"]["pluginId"], "strata.manager");
            assert_eq!(val["payload"]["clickThrough"], true);
            saw_hud_scene = true;
        }
        if saw_hud_resp && saw_hud_scene {
            break;
        }
    }
    assert!(saw_hud_resp && saw_hud_scene);

    // 4b. Verify Hub -> Backend system notifications update HUD theme without disrupting RPC.
    let sys_frames = concat!(
        "{\"event\":\"appearance.changed\",\"payload\":{\"isDark\":false}}\n",
        "{\"event\":\"panel.visibilityChanged\",\"payload\":{\"visible\":true}}\n",
        "{\"event\":\"network.contextChanged\",\"payload\":{\"transport\":\"cellular\"}}\n",
        "{\"id\":\"bk_3\",\"method\":\"daemon.ping\"}\n"
    );
    send_raw(&mut write_half, sys_frames).await;

    let (mut saw_light_hud, mut saw_ping_after_events) = (false, false);
    for _ in 0..8 {
        let val = recv_json(&mut reader).await;
        if val.get("id").and_then(Value::as_str) == Some("bk_3") {
            assert_eq!(val["result"]["pong"], true);
            saw_ping_after_events = true;
        } else if val.get("event").and_then(Value::as_str) == Some("hud.updateScene")
            && val["payload"]["texts"][0]["colorArgb"] == 0xFF0F_172A_u32
        {
            saw_light_hud = true;
        }
        if saw_light_hud && saw_ping_after_events {
            break;
        }
    }
    assert!(saw_light_hud && saw_ping_after_events);
    assert!(!stack.sideveil.hud().is_dark());

    // 5. Verify hub disconnect triggers automatic reconnect.
    drop(reader);
    drop(write_half);
    let (stream2, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
        .await
        .expect("bridge reconnects within 3s")
        .expect("accept second mock hub connection");
    let (r2, mut w2) = stream2.into_split();
    let mut reader2 = BufReader::new(r2);
    let _ = recv_json(&mut reader2).await;
    send_raw(&mut w2, "{\"ok\":true}\n").await;

    // 6. Verify clean shutdown emits `hud.dismiss` and exits the bridge task promptly.
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(2), bridge_handle)
        .await
        .expect("bridge shuts down cleanly");

    let _ = fs::remove_dir_all(&base);
}

#[tokio::test]
async fn event_hub_keeps_reliable_events_isolated_from_ephemeral_flood() {
    let hub = EventHub::new();
    let mut sub = hub.subscribe();

    // Emit a reliable event, then flood the ephemeral channel past its 256-item capacity.
    hub.emit(EventEnvelope::new(
        events::TASK_EXIT,
        Some(json!({ "taskId": "task-42", "exitCode": 0 })),
    ));
    for seq in 0..400_u64 {
        hub.emit(EventEnvelope::new(
            events::TASK_OUTPUT,
            Some(json!({ "taskId": "task-42", "seq": seq, "chunk": "noisy" })),
        ));
    }
    hub.emit(EventEnvelope::new(
        events::CONTAINER_STATE_CHANGED,
        Some(json!({ "container": "main", "current": "running" })),
    ));

    // Reliable receiver must deliver both reliable events with zero lag.
    let first = sub.reliable.recv().await.expect("first reliable event");
    assert_eq!(first.event, events::TASK_EXIT);
    let second = sub.reliable.recv().await.expect("second reliable event");
    assert_eq!(second.event, events::CONTAINER_STATE_CHANGED);

    // Ephemeral receiver lags as expected without impacting reliable delivery.
    assert!(matches!(
        sub.ephemeral.recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
    ));
}

#[tokio::test]
async fn dispatcher_rejects_session_lease_methods_from_sideveil_bridge() {
    let base = std::env::temp_dir().join(format!("strata-sv-acl-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create temp test dir");
    let stack = build_test_stack(&base, base.join("config.json"));

    let acq = stack
        .dispatcher
        .handle(
            RpcCaller::SideVeilBridge,
            RpcRequest::new("ac_1", methods::SESSION_ACQUIRE, Some(json!({}))),
        )
        .await;
    assert_eq!(
        acq.error.expect("session.acquire must fail").code,
        error_codes::PERMISSION_DENIED
    );

    let rel = stack
        .dispatcher
        .handle(
            RpcCaller::SideVeilBridge,
            RpcRequest::new(
                "rl_1",
                methods::SESSION_RELEASE,
                Some(json!({ "leaseId": "lease-1" })),
            ),
        )
        .await;
    assert_eq!(
        rel.error.expect("session.release must fail").code,
        error_codes::PERMISSION_DENIED
    );

    let _ = fs::remove_dir_all(&base);
}
