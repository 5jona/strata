//! Interactive WebSocket PTY handler (`GET /ws/pty/:id`) and explicit kill (`DELETE /api/pty/:session_id`).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::web::WebAppState;
use crate::web::pty_session::{
    ClientPtyFrame, PtySession, WatermarkFlow, decode_client_frame, encode_data_frame,
    encode_exit_frame,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PtyConnectQuery {
    pub(super) session: Option<String>,
    #[serde(default = "default_cols")]
    pub(super) cols: u16,
    #[serde(default = "default_rows")]
    pub(super) rows: u16,
}

fn default_cols() -> u16 {
    120
}

fn default_rows() -> u16 {
    36
}

/// Upgrades an authenticated client to an interactive binary PTY WebSocket for `:container_id`.
pub(super) async fn handle_pty_ws(
    State(state): State<Arc<WebAppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(container_id): Path<String>,
    Query(q): Query<PtyConnectQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let lan_enabled = state.is_lan_enabled().await;
    let matched_proto = match state
        .auth
        .verify_request(peer.ip(), lan_enabled, &headers)
        .await
    {
        Ok(proto) => proto,
        Err((status, msg)) => return (status, msg).into_response(),
    };

    let session = if let Some(sid) = &q.session
        && let Some(existing) = state.ptys.get(sid).await
        && existing.container_id == container_id
    {
        existing.resize(q.cols, q.rows);
        existing
    } else {
        let (cid, ns_pid, cfg) = match state.manager.running_target(Some(&container_id)).await {
            Ok(v) => v,
            Err(err) => {
                return (
                    StatusCode::CONFLICT,
                    Json(json!({ "error": err.to_string(), "code": err.code() })),
                )
                    .into_response();
            }
        };
        let spawned = match state
            .ptys
            .spawn_session(
                &state.manager.self_exe,
                cid.clone(),
                ns_pid,
                cfg.cap_profile(),
                q.cols,
                q.rows,
                None,
            )
            .await
        {
            Ok(sess) => sess,
            Err(err) => {
                let status = if err.contains("maximum concurrent PTY") {
                    StatusCode::TOO_MANY_REQUESTS
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                return (status, Json(json!({ "error": err }))).into_response();
            }
        };
        if !state.manager.is_running_with_pid(&cid, ns_pid).await {
            let _ = state.ptys.kill_session(&spawned.session_id).await;
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": format!("container '{cid}' stopped during PTY launch") })),
            )
                .into_response();
        }
        spawned
    };

    let session_id = session.session_id.clone();
    let ws = match matched_proto {
        Some(proto) => ws.protocols([proto]),
        None => ws,
    };
    let mut resp = ws
        .on_upgrade(move |socket| run_pty_socket(socket, session, state))
        .into_response();
    if let Ok(val) = session_id.parse() {
        resp.headers_mut().insert("X-Strata-Pty-Session", val);
    }
    resp
}

/// Explicitly terminates a live or detached PTY session (`DELETE /api/pty/:session_id`).
pub(super) async fn handle_pty_delete(
    State(state): State<Arc<WebAppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Response {
    let lan_enabled = state.is_lan_enabled().await;
    if let Err((status, msg)) = state
        .auth
        .verify_request(peer.ip(), lan_enabled, &headers)
        .await
    {
        return (status, Json(json!({ "error": msg }))).into_response();
    }
    let killed = state.ptys.kill_session(&session_id).await;
    Json(json!({ "sessionId": session_id, "terminated": killed })).into_response()
}

async fn run_pty_socket(socket: WebSocket, session: Arc<PtySession>, state: Arc<WebAppState>) {
    let my_gen = session.mark_attached().await;
    let mut epoch_rx = state.session_epoch.clone();
    let _ = epoch_rx.borrow_and_update();

    let (mut sink, mut stream) = socket.split();

    let meta = json!({
        "sessionId": session.session_id,
        "container": session.container_id,
        "pid": session.pid,
    });
    if sink
        .send(Message::Text(meta.to_string().into()))
        .await
        .is_err()
    {
        session.mark_detached(my_gen).await;
        return;
    }

    let mut out_rx = session.out_tx.subscribe();
    let backlog = session.ring.lock().await.snapshot();
    let mut flow = WatermarkFlow::default();
    if !backlog.is_empty() {
        flow.on_sent(backlog.len());
        session.set_read_paused(flow.is_paused());
        if sink
            .send(Message::Binary(encode_data_frame(&backlog).into()))
            .await
            .is_err()
        {
            session.mark_detached(my_gen).await;
            return;
        }
    }

    loop {
        if !session.is_current_generation(my_gen) {
            let _ = sink.send(Message::Close(None)).await;
            break;
        }
        if let Some(exit) = session.exit_state.read().await.clone() {
            let _ = sink
                .send(Message::Binary(encode_exit_frame(&exit).into()))
                .await;
            break;
        }

        tokio::select! {
            _ = epoch_rx.changed() => {
                let _ = sink.send(Message::Close(None)).await;
                break;
            }
            _ = session.attach_notify.notified() => {
                if !session.is_current_generation(my_gen) {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Binary(bytes))) => {
                        if let Ok(frame) = decode_client_frame(&bytes) {
                            match frame {
                                ClientPtyFrame::Data(data) => {
                                    let _ = session.stdin_tx.send(data).await;
                                }
                                ClientPtyFrame::Resize(r) => {
                                    session.resize(r.cols, r.rows);
                                }
                                ClientPtyFrame::Ack(n) => {
                                    let _ = flow.on_ack(n);
                                    session.set_read_paused(flow.is_paused());
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if sink.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
            chunk = out_rx.recv(), if !flow.is_paused() => {
                match chunk {
                    Ok(data) => {
                        flow.on_sent(data.len());
                        session.set_read_paused(flow.is_paused());
                        if sink.send(Message::Binary(encode_data_frame(&data).into())).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
            _ = session.exit_notify.notified() => {
                if let Some(exit) = session.exit_state.read().await.clone() {
                    let _ = sink.send(Message::Binary(encode_exit_frame(&exit).into())).await;
                }
                break;
            }
        }
    }

    session.mark_detached(my_gen).await;
}
