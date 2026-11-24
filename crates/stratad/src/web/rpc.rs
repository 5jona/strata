//! HTTP JSON-RPC (`POST /api/rpc`) and public metadata (`GET /api/info`) endpoints.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use strata_protocol::RpcRequest;

use crate::dispatcher::RpcCaller;
use crate::web::WebAppState;
use crate::web::auth::{is_allowed_host_header, is_allowed_peer_ip};

/// Public minimal probe endpoint (`GET /api/info`) exposing version and auth mode (no secrets).
pub(super) async fn handle_info(
    State(state): State<Arc<WebAppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let lan_enabled = state.is_lan_enabled().await;
    if !is_allowed_peer_ip(peer.ip(), lan_enabled) {
        return (StatusCode::FORBIDDEN, "peer IP not allowed").into_response();
    }
    if let Some(host_val) = headers.get(header::HOST) {
        let host = host_val.to_str().unwrap_or("");
        if !is_allowed_host_header(host, lan_enabled) {
            return (StatusCode::FORBIDDEN, "host header not allowed").into_response();
        }
    }
    Json(json!({
        "name": "strata",
        "version": env!("CARGO_PKG_VERSION"),
        "apiVersion": 1,
        "authRequired": true,
        "lanEnabled": lan_enabled,
    }))
    .into_response()
}

/// Authenticated JSON-RPC 2.0 endpoint (`POST /api/rpc`) routed directly to `Dispatcher`.
pub(super) async fn handle_rpc(
    State(state): State<Arc<WebAppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<RpcRequest>,
) -> Response {
    let lan_enabled = state.is_lan_enabled().await;
    if let Err((status, msg)) = state
        .auth
        .verify_request(peer.ip(), lan_enabled, &headers)
        .await
    {
        return (status, Json(json!({ "error": msg }))).into_response();
    }
    let resp = state.dispatcher.handle(RpcCaller::WebConsole, req).await;
    Json(resp).into_response()
}
