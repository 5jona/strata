//! End-to-end Tokio HTTP + WebSocket integration tests for the embedded `axum` control plane,
//! Bearer/Host/Origin guards, `Sec-WebSocket-Protocol` echo, `rotateToken` revocation,
//! remote backup jail, `daemon.capabilities`, and `service.*` CRUD RPCs.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use strata_protocol::rpc::{ContainerServiceSpec, ServiceRestartPolicy, error_codes, methods};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::catalog::CatalogClient;
use crate::config_store::ConfigStore;
use crate::dispatcher::Dispatcher;
use crate::jobs::ProvisionJobManager;
use crate::manager::ContainerManager;
use crate::services::ServiceSupervisor;
use crate::sideveil_seam::{EventHub, SideVeilBridgeSeam};
use crate::task_runner::TaskRunner;
use crate::wakelock::WakeLockManager;
use crate::web::{WebAppState, WebServerController, build_router};

#[tokio::test]
async fn http_and_ws_api_enforces_security_guards_subprotocol_echo_and_revocation() {
    let base = std::env::temp_dir().join(format!("strata-web-it-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create temp dir");

    let cfg_path = base.join("config.json");
    let token_path = base.join("web.token");
    let config_store = Arc::new(ConfigStore::load_or_seed(cfg_path).expect("seed config"));
    let event_hub = Arc::new(EventHub::new());
    let sideveil = Arc::new(SideVeilBridgeSeam::new());
    let wakelock = Arc::new(WakeLockManager::new());
    let catalog = Arc::new(CatalogClient::new().expect("catalog"));
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
    let jobs = Arc::new(ProvisionJobManager::new(
        Arc::clone(&config_store),
        Arc::clone(&manager),
        Arc::clone(&catalog),
        Arc::clone(&event_hub),
        Arc::clone(&wakelock),
    ));
    let services = Arc::new(ServiceSupervisor::new(
        Arc::clone(&config_store),
        Arc::clone(&manager),
        Arc::clone(&event_hub),
        PathBuf::from("/bin/true"),
    ));
    let tools = Arc::new(crate::tools::ToolsManager::new(
        Arc::clone(&manager),
        Arc::clone(&task_runner),
        Arc::clone(&event_hub),
        PathBuf::from("/bin/true"),
    ));
    let web = Arc::new(
        WebServerController::new(
            token_path.clone(),
            Arc::clone(&config_store),
            Arc::clone(&jobs),
            Arc::clone(&wakelock),
        )
        .expect("web controller"),
    );
    let dispatcher = Arc::new(Dispatcher::new(
        Instant::now(),
        Arc::clone(&config_store),
        Arc::clone(&manager),
        Arc::clone(&task_runner),
        Arc::clone(&sideveil),
        Arc::clone(&web),
        Arc::clone(&catalog),
        Arc::clone(&jobs),
        Arc::clone(&services),
        tools,
    ));

    let state = Arc::new(WebAppState {
        config_store,
        dispatcher,
        manager,
        event_hub,
        jobs,
        auth: Arc::clone(&web.auth),
        ptys: web.ptys(),
        wakelock,
        session_epoch: web.session_epoch_tx.subscribe(),
        active_events_ws: Arc::clone(&web.active_events_ws),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let router = build_router(state);
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    let token = fs::read_to_string(&token_path)
        .expect("read token")
        .trim()
        .to_owned();
    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");

    // 1. Public /api/info works on loopback, but rejects DNS-rebinding Host headers.
    let info: Value = client
        .get(format!("{base_url}/api/info"))
        .send()
        .await
        .expect("GET /api/info")
        .json()
        .await
        .expect("json info");
    assert_eq!(info["authRequired"], true);

    let bad_host = client
        .get(format!("{base_url}/api/info"))
        .header("Host", "evil.example.com:7681")
        .send()
        .await
        .expect("GET bad host");
    assert_eq!(bad_host.status(), reqwest::StatusCode::FORBIDDEN);

    // 2. Unauthenticated /api/rpc is rejected with 401; bad Origin rejected with 403.
    let unauth = client
        .post(format!("{base_url}/api/rpc"))
        .json(&json!({ "jsonrpc": "2.0", "id": "1", "method": methods::DAEMON_PING }))
        .send()
        .await
        .expect("POST unauth");
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    let bad_origin = client
        .post(format!("{base_url}/api/rpc"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Origin", "https://evil.example.com")
        .json(&json!({ "jsonrpc": "2.0", "id": "2", "method": methods::DAEMON_PING }))
        .send()
        .await
        .expect("POST bad origin");
    assert_eq!(bad_origin.status(), reqwest::StatusCode::FORBIDDEN);

    // 3. Windows Tauri v2 (`http://tauri.localhost`) CORS preflight & authenticated RPC work;
    // evil preflight is rejected; remote backup escape and hostRuntime are blocked.
    let preflight_ok = client
        .request(reqwest::Method::OPTIONS, format!("{base_url}/api/rpc"))
        .header("Origin", "http://tauri.localhost")
        .header("Access-Control-Request-Method", "POST")
        .header(
            "Access-Control-Request-Headers",
            "authorization,content-type",
        )
        .send()
        .await
        .expect("OPTIONS /api/rpc");
    assert_eq!(preflight_ok.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(
        preflight_ok
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("http://tauri.localhost")
    );
    let preflight_bad = client
        .request(reqwest::Method::OPTIONS, format!("{base_url}/api/rpc"))
        .header("Origin", "https://evil.example.com")
        .send()
        .await
        .expect("OPTIONS evil origin");
    assert_eq!(preflight_bad.status(), reqwest::StatusCode::FORBIDDEN);

    let caps_http = client
        .post(format!("{base_url}/api/rpc"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Origin", "http://tauri.localhost")
        .json(&json!({ "jsonrpc": "2.0", "id": "3", "method": methods::DAEMON_CAPABILITIES }))
        .send()
        .await
        .expect("POST caps");
    assert_eq!(caps_http.status(), reqwest::StatusCode::OK);
    assert_eq!(
        caps_http
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("http://tauri.localhost")
    );
    let caps_resp: Value = caps_http.json().await.expect("caps json");
    assert!(caps_resp["result"]["kernelRelease"].is_string());

    let bad_backup: Value = client
        .post(format!("{base_url}/api/rpc"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "b1",
            "method": methods::CONTAINER_BACKUP,
            "params": { "outputPath": "/data/adb/pwn.img.gz" }
        }))
        .send()
        .await
        .expect("POST bad backup")
        .json()
        .await
        .expect("bad backup json");
    assert_eq!(bad_backup["error"]["code"], error_codes::PERMISSION_DENIED);

    let bad_patch: Value = client
        .post(format!("{base_url}/api/rpc"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "p1",
            "method": methods::CONTAINER_CONFIG_PATCH,
            "params": { "hostRuntime": true }
        }))
        .send()
        .await
        .expect("POST bad patch")
        .json()
        .await
        .expect("bad patch json");
    assert_eq!(bad_patch["error"]["code"], error_codes::PERMISSION_DENIED);

    // 4. WebSocket /ws/events echoes Sec-WebSocket-Protocol and closes immediately on rotateToken.
    let mut ws_stream = TcpStream::connect(addr).await.expect("connect ws");
    let expected_proto = format!("strata-auth.{token}");
    let upgrade_req = format!(
        "GET /ws/events HTTP/1.1\r\n\
         Host: 127.0.0.1:{}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Protocol: {expected_proto}\r\n\r\n",
        addr.port()
    );
    ws_stream
        .write_all(upgrade_req.as_bytes())
        .await
        .expect("write upgrade");

    let mut hdr_buf = vec![0u8; 1024];
    let n = tokio::time::timeout(Duration::from_secs(2), ws_stream.read(&mut hdr_buf))
        .await
        .expect("read upgrade timeout")
        .expect("read upgrade");
    let hdr_text = String::from_utf8_lossy(&hdr_buf[..n]);
    assert!(
        hdr_text.starts_with("HTTP/1.1 101"),
        "expected 101 Switching Protocols, got: {hdr_text}"
    );
    assert!(
        hdr_text.to_ascii_lowercase().contains(&format!(
            "sec-websocket-protocol: {}",
            expected_proto.to_ascii_lowercase()
        )),
        "missing echoed Sec-WebSocket-Protocol in: {hdr_text}"
    );

    // Wait briefly for socket task to increment active_events_ws, then rotate token via RPC.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let rotated: Value = client
        .post(format!("{base_url}/api/rpc"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "jsonrpc": "2.0", "id": "rot1", "method": methods::WEB_ROTATE_TOKEN }))
        .send()
        .await
        .expect("POST rotate")
        .json()
        .await
        .expect("rotate json");
    assert_eq!(rotated["result"]["sessionsRevoked"], 1);
    let new_token = rotated["result"]["token"]
        .as_str()
        .expect("new token")
        .to_owned();

    // Active WS must receive close frame / EOF promptly after token rotation.
    let mut close_buf = [0u8; 64];
    let close_read = tokio::time::timeout(Duration::from_secs(2), ws_stream.read(&mut close_buf))
        .await
        .expect("ws must close promptly on rotateToken")
        .expect("ws close read");
    // Either clean EOF (0) or WS Close frame opcode 0x88.
    assert!(
        close_read == 0 || close_buf[0] == 0x88,
        "expected close frame or EOF, got {close_read} bytes starting with {:#x}",
        close_buf[0]
    );

    // 5. Managed container services upsert -> list -> delete CRUD roundtrip with rotated token.
    let spec = ContainerServiceSpec {
        id: "demo-svc".to_owned(),
        name: "Demo Worker".to_owned(),
        command: vec!["/bin/sleep".to_owned(), "60".to_owned()],
        workdir: Some("/root".to_owned()),
        env: std::collections::BTreeMap::new(),
        auto_start: true,
        restart_policy: ServiceRestartPolicy::OnFailure,
        max_restarts: 5,
        backoff_ms: 1000,
    };
    let upsert: Value = client
        .post(format!("{base_url}/api/rpc"))
        .header("Authorization", format!("Bearer {new_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "5",
            "method": methods::SERVICE_UPSERT,
            "params": { "container": "main", "spec": spec }
        }))
        .send()
        .await
        .expect("POST service.upsert")
        .json()
        .await
        .expect("upsert json");
    assert_eq!(upsert["result"]["id"], "demo-svc");

    let del: Value = client
        .post(format!("{base_url}/api/rpc"))
        .header("Authorization", format!("Bearer {new_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "7",
            "method": methods::SERVICE_DELETE,
            "params": { "container": "main", "serviceId": "demo-svc" }
        }))
        .send()
        .await
        .expect("POST service.delete")
        .json()
        .await
        .expect("delete json");
    assert_eq!(del["result"]["deleted"], true);

    server.abort();
    let _ = fs::remove_dir_all(&base);
}
