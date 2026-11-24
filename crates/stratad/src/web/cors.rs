//! CORS preflight and response header middleware for trusted Tauri/localhost/RFC1918 origins.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::web::WebAppState;
use crate::web::auth_policy::{is_allowed_host_header, is_allowed_origin, is_allowed_peer_ip};

const ALLOW_METHODS: HeaderValue = HeaderValue::from_static("GET, POST, DELETE, OPTIONS");
const ALLOW_HEADERS: HeaderValue = HeaderValue::from_static("Authorization, Content-Type");
const EXPOSE_HEADERS: HeaderValue = HeaderValue::from_static("Content-Disposition");
const MAX_AGE: HeaderValue = HeaderValue::from_static("600");
const VARY_ORIGIN: HeaderValue = HeaderValue::from_static("Origin");

/// Handles CORS `OPTIONS` preflight and attaches `Access-Control-Allow-Origin` for allowed origins.
pub(crate) async fn cors_middleware(
    State(state): State<Arc<WebAppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let lan_enabled = state.is_lan_enabled().await;
    let origin_header = req.headers().get(header::ORIGIN).cloned();
    let allowed_origin = origin_header
        .as_ref()
        .and_then(|v| v.to_str().ok())
        .filter(|origin| is_allowed_origin(origin, lan_enabled))
        .and_then(|origin| HeaderValue::from_str(origin).ok());

    if req.method() == Method::OPTIONS {
        if !is_allowed_peer_ip(peer.ip(), lan_enabled) {
            return (StatusCode::FORBIDDEN, "peer IP not allowed").into_response();
        }
        if let Some(host) = req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            && !is_allowed_host_header(host, lan_enabled)
        {
            return (StatusCode::FORBIDDEN, "host header not allowed").into_response();
        }
        let Some(origin_val) = allowed_origin else {
            return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
        };
        let mut resp = StatusCode::NO_CONTENT.into_response();
        let headers = resp.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin_val);
        headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, ALLOW_METHODS);
        headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, ALLOW_HEADERS);
        headers.insert(header::ACCESS_CONTROL_MAX_AGE, MAX_AGE);
        headers.insert(header::VARY, VARY_ORIGIN);
        return resp;
    }

    let mut resp = next.run(req).await;
    if let Some(origin_val) = allowed_origin {
        let headers = resp.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin_val);
        headers.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, EXPOSE_HEADERS);
        headers.insert(header::VARY, VARY_ORIGIN);
    }
    resp
}
