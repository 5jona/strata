//! NDJSON Unix domain socket control-plane server with kernel `SO_PEERCRED` UID 0 authentication.

use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use strata_protocol::rpc::{error_codes, methods};
use strata_protocol::{RpcRequest, RpcResponse};
use strata_sys::procfs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch};

use crate::dispatcher::Dispatcher;
use crate::manager::ContainerManager;
use crate::sideveil_seam::EventHub;

const MAX_FRAME_BYTES: usize = 1024 * 1024;
static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

/// Runs the `strata.sock` control-plane listener until `shutdown` becomes `true`.
pub(crate) async fn serve(
    socket_path: PathBuf,
    dispatcher: Arc<Dispatcher>,
    manager: Arc<ContainerManager>,
    event_hub: Arc<EventHub>,
    mut shutdown: watch::Receiver<bool>,
) {
    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }

    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(err) => {
            eprintln!(
                "stratad: failed to bind socket '{}': {err}",
                socket_path.display()
            );
            return;
        }
    };
    let _ = fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600));

    loop {
        if *shutdown.borrow() {
            break;
        }
        tokio::select! {
            _ = async {
                while !*shutdown.borrow_and_update() {
                    if shutdown.changed().await.is_err() {
                        break;
                    }
                }
            } => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let disp = Arc::clone(&dispatcher);
                        let mgr = Arc::clone(&manager);
                        let hub = Arc::clone(&event_hub);
                        tokio::spawn(async move {
                            handle_connection(stream, disp, mgr, hub).await;
                        });
                    }
                    Err(err) => {
                        eprintln!("stratad: accept error: {err}");
                    }
                }
            }
        }
    }
}

async fn handle_connection(
    stream: UnixStream,
    dispatcher: Arc<Dispatcher>,
    manager: Arc<ContainerManager>,
    event_hub: Arc<EventHub>,
) {
    match procfs::peer_cred_uid(stream.as_raw_fd()) {
        Ok(0) => {}
        Ok(uid) => {
            eprintln!("stratad: rejected unauthorized strata.sock peer uid={uid}");
            return;
        }
        Err(err) => {
            eprintln!("stratad: failed SO_PEERCRED check: {err}");
            return;
        }
    }

    let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
    let (read_half, mut write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::channel::<String>(128);

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

    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let mut event_forwarder: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(n) if n > MAX_FRAME_BYTES => {
                let err_resp = RpcResponse::err(
                    "0",
                    error_codes::INVALID_REQUEST,
                    "frame exceeds 1 MiB limit",
                );
                if let Ok(json) = serde_json::to_string(&err_resp) {
                    let _ = out_tx.send(json).await;
                }
                break;
            }
            Ok(_) => {}
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req = match serde_json::from_str::<RpcRequest>(trimmed) {
            Ok(r) => r,
            Err(err) => {
                let resp = RpcResponse::err("0", error_codes::PARSE_ERROR, format!("json: {err}"));
                if let Ok(json) = serde_json::to_string(&resp) {
                    let _ = out_tx.send(json).await;
                }
                continue;
            }
        };

        let is_subscribe = req.method == methods::EVENTS_SUBSCRIBE;
        let resp = dispatcher.handle(conn_id, req).await;
        if let Ok(json) = serde_json::to_string(&resp)
            && out_tx.send(json).await.is_err()
        {
            break;
        }

        if is_subscribe && event_forwarder.is_none() {
            let mut rx = event_hub.subscribe();
            let tx = out_tx.clone();
            event_forwarder = Some(tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(evt) => {
                            if let Ok(json) = serde_json::to_string(&evt)
                                && tx.send(json).await.is_err()
                            {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }));
        }
    }

    if let Some(handle) = event_forwarder {
        handle.abort();
    }
    drop(out_tx);
    let _ = writer_task.await;
    manager.release_connection_leases(conn_id).await;
}
