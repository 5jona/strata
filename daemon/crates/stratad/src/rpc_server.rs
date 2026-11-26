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
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch};

use crate::dispatcher::{Dispatcher, RpcCaller};
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
        while let Some(mut line) = out_rx.recv().await {
            line.push('\n');
            if write_half.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            let _ = write_half.flush().await;
        }
    });

    let mut reader = BufReader::new(read_half);
    let mut event_forwarder: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        let line = match read_bounded_rpc_line(&mut reader, MAX_FRAME_BYTES).await {
            FrameRead::Line(line) => line,
            FrameRead::TooLarge => {
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
            FrameRead::Eof | FrameRead::IoErr => break,
        };

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
        let resp = dispatcher
            .handle(RpcCaller::LocalCtl { conn_id }, req)
            .await;
        if let Ok(json) = serde_json::to_string(&resp)
            && out_tx.send(json).await.is_err()
        {
            break;
        }

        if is_subscribe && event_forwarder.is_none() {
            let mut sub = event_hub.subscribe();
            let tx = out_tx.clone();
            event_forwarder = Some(tokio::spawn(async move {
                loop {
                    tokio::select! {
                        biased;
                        evt = sub.reliable.recv() => match evt {
                            Ok(env) => {
                                if let Ok(json) = serde_json::to_string(&env)
                                    && tx.send(json).await.is_err()
                                {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        },
                        evt = sub.ephemeral.recv() => match evt {
                            Ok(env) => {
                                if let Ok(json) = serde_json::to_string(&env) {
                                    let _ = tx.try_send(json);
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        },
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

#[derive(Debug, PartialEq, Eq)]
enum FrameRead {
    Line(String),
    TooLarge,
    Eof,
    IoErr,
}

/// Reads a single UTF-8 NDJSON line, enforcing `max_bytes` before allocating beyond the cap.
async fn read_bounded_rpc_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    max_bytes: usize,
) -> FrameRead {
    let mut buf = Vec::with_capacity(256);
    loop {
        let Ok(available) = reader.fill_buf().await else {
            return FrameRead::IoErr;
        };
        if available.is_empty() {
            if buf.is_empty() {
                return FrameRead::Eof;
            }
            return match String::from_utf8(buf) {
                Ok(s) => FrameRead::Line(s),
                Err(_) => FrameRead::IoErr,
            };
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            if buf.len().saturating_add(pos) > max_bytes {
                return FrameRead::TooLarge;
            }
            buf.extend_from_slice(&available[..pos]);
            reader.consume(pos + 1);
            return match String::from_utf8(buf) {
                Ok(s) => FrameRead::Line(s),
                Err(_) => FrameRead::IoErr,
            };
        }
        if buf.len().saturating_add(available.len()) > max_bytes {
            return FrameRead::TooLarge;
        }
        let len = available.len();
        buf.extend_from_slice(available);
        reader.consume(len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_bounded_rpc_line_rejects_oversized_frame_without_newline() {
        let mut reader = BufReader::new(&b"{\"id\":\"1\"}\n"[..]);
        assert_eq!(
            read_bounded_rpc_line(&mut reader, 32).await,
            FrameRead::Line("{\"id\":\"1\"}".to_owned())
        );
        assert_eq!(read_bounded_rpc_line(&mut reader, 32).await, FrameRead::Eof);

        let oversized = [b'a'; 128];
        let mut big_reader = BufReader::new(&oversized[..]);
        assert_eq!(
            read_bounded_rpc_line(&mut big_reader, 64).await,
            FrameRead::TooLarge
        );
    }
}
