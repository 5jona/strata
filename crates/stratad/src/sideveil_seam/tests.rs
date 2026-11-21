//! Host-side Tokio integration tests for the SideVeil `hub.sock` plugin backend bridge.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use strata_protocol::rpc::{HudSetParams, methods};
use strata_protocol::{EventEnvelope, RpcRequest, RpcResponse, SideVeilBridgeConfig, StrataConfig};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::watch;

use crate::config_store::ConfigStore;
use crate::dispatcher::Dispatcher;
use crate::manager::ContainerManager;
use crate::sideveil_seam::{EventHub, PluginEventSink, SideVeilBridgeSeam};
use crate::task_runner::TaskRunner;

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
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let bridge_handle = sideveil.spawn_bridge(
        Arc::clone(&config_store),
        Arc::clone(&dispatcher),
        Arc::clone(&manager),
        Arc::clone(&task_runner),
        Arc::clone(&event_hub),
        shutdown_rx,
    );

    let (stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
        .await
        .expect("bridge connects within 3s")
        .expect("accept mock hub connection");

    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    // 1. Verify SideVeil PluginBackend hello handshake.
    let mut hello_line = String::new();
    reader.read_line(&mut hello_line).await.expect("read hello");
    let hello: Value = serde_json::from_str(hello_line.trim()).expect("parse hello");
    assert_eq!(hello["role"], "pluginBackend");
    assert_eq!(hello["pluginId"], "strata.manager");

    write_half
        .write_all(b"{\"ok\":true}\n")
        .await
        .expect("send ack");
    write_half.flush().await.expect("flush ack");

    // 2. Verify multiplexed JSON-RPC request -> response over hub.sock.
    let ping_req = RpcRequest::new("bk_1", methods::DAEMON_PING, None);
    let mut req_line = serde_json::to_string(&ping_req).expect("encode ping");
    req_line.push('\n');
    write_half
        .write_all(req_line.as_bytes())
        .await
        .expect("send ping");
    write_half.flush().await.expect("flush ping");

    let mut resp_line = String::new();
    reader
        .read_line(&mut resp_line)
        .await
        .expect("read ping response");
    let resp: RpcResponse = serde_json::from_str(resp_line.trim()).expect("parse ping resp");
    assert_eq!(resp.id, "bk_1");
    assert_eq!(resp.result.expect("ping result")["pong"], true);
    assert!(sideveil.is_connected());

    // 3. Verify EventHub -> SideVeil HostEventEnvelope forwarding.
    event_hub.emit(EventEnvelope::new(
        "task.output",
        Some(json!({ "taskId": "task-1", "chunk": "hello" })),
    ));
    let mut evt_line = String::new();
    reader.read_line(&mut evt_line).await.expect("read event");
    let evt: EventEnvelope = serde_json::from_str(evt_line.trim()).expect("parse event");
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
    let mut hud_line = serde_json::to_string(&hud_req).expect("encode hud.set");
    hud_line.push('\n');
    write_half
        .write_all(hud_line.as_bytes())
        .await
        .expect("send hud.set");
    write_half.flush().await.expect("flush hud.set");

    let mut saw_hud_resp = false;
    let mut saw_hud_scene = false;
    for _ in 0..4 {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
            .await
            .expect("read hud frame timeout")
            .expect("read hud frame");
        let val: Value = serde_json::from_str(line.trim()).expect("parse frame");
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

    // 5. Verify hub disconnect triggers automatic reconnect.
    drop(reader);
    drop(write_half);
    let (stream2, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
        .await
        .expect("bridge reconnects within 3s")
        .expect("accept second mock hub connection");
    let (r2, mut w2) = stream2.into_split();
    let mut reader2 = BufReader::new(r2);
    let mut hello2 = String::new();
    reader2
        .read_line(&mut hello2)
        .await
        .expect("read second hello");
    w2.write_all(b"{\"ok\":true}\n")
        .await
        .expect("send second ack");
    w2.flush().await.expect("flush second ack");

    // 6. Verify clean shutdown emits `hud.dismiss` and exits the bridge task promptly.
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(2), bridge_handle)
        .await
        .expect("bridge shuts down cleanly");

    let _ = fs::remove_dir_all(&base);
}
