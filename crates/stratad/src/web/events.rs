//! Authenticated WebSocket event stream (`GET /ws/events`) forwarding `EventHub` notifications
//! with `Sec-WebSocket-Protocol` echo and immediate revocation on token rotation / disable.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;

use crate::web::WebAppState;

struct ActiveWsGuard(Arc<AtomicUsize>);

impl Drop for ActiveWsGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Upgrades an authenticated client to the real-time JSON event stream WebSocket.
pub(super) async fn handle_events_ws(
    State(state): State<Arc<WebAppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
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
    let ws = match matched_proto {
        Some(proto) => ws.protocols([proto]),
        None => ws,
    };
    ws.on_upgrade(move |socket| run_events_socket(socket, state))
        .into_response()
}

async fn run_events_socket(socket: WebSocket, state: Arc<WebAppState>) {
    state.active_events_ws.fetch_add(1, Ordering::AcqRel);
    let _guard = ActiveWsGuard(Arc::clone(&state.active_events_ws));
    let mut epoch_rx = state.session_epoch.clone();
    let _ = epoch_rx.borrow_and_update();

    let (mut sink, mut stream) = socket.split();
    let mut sub = state.event_hub.subscribe();

    loop {
        tokio::select! {
            _ = epoch_rx.changed() => {
                let _ = sink.send(Message::Close(None)).await;
                break;
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Ping(payload))) => {
                        if sink.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
            ev = sub.reliable.recv() => {
                match ev {
                    Ok(envelope) => {
                        if let Ok(text) = serde_json::to_string(&envelope)
                            && sink.send(Message::Text(text.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
            ev = sub.ephemeral.recv() => {
                match ev {
                    Ok(envelope) => {
                        if let Ok(text) = serde_json::to_string(&envelope)
                            && sink.send(Message::Text(text.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }
}
