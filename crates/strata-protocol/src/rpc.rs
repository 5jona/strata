//! JSON-RPC 2.0 wire envelope, method/event constants, and error codes aligned with SideVeil.

pub mod payloads;
pub mod tasks;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::generated::constants::{error_codes, events, methods};
pub use payloads::*;
pub use tasks::*;

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcRequest {
    #[serde(default = "default_jsonrpc")]
    pub jsonrpc: String,
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

impl RpcRequest {
    /// Creates a new JSON-RPC 2.0 request envelope.
    #[must_use]
    pub fn new(id: impl Into<String>, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id: id.into(),
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 response envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcResponse {
    #[serde(default = "default_jsonrpc")]
    pub jsonrpc: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    /// Creates a successful JSON-RPC 2.0 response carrying `result`.
    #[must_use]
    pub fn ok(id: impl Into<String>, result: Value) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id: id.into(),
            result: Some(result),
            error: None,
        }
    }

    /// Creates a JSON-RPC 2.0 error response with numeric `code` and diagnostic `message`.
    #[must_use]
    pub fn err(id: impl Into<String>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id: id.into(),
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// Server-pushed NDJSON event envelope compatible with SideVeil `HostEventEnvelope`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event: String,
    #[serde(default)]
    pub payload: Option<Value>,
}

impl EventEnvelope {
    /// Creates a new server-pushed event envelope.
    #[must_use]
    pub fn new(event: impl Into<String>, payload: Option<Value>) -> Self {
        Self {
            event: event.into(),
            payload,
        }
    }
}

fn default_jsonrpc() -> String {
    "2.0".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_rpc_request_response_and_event() {
        let req = RpcRequest::new("req-1", methods::DAEMON_PING, None);
        let json = serde_json::to_string(&req).expect("serialize request");
        let parsed: RpcRequest = serde_json::from_str(&json).expect("deserialize request");
        assert_eq!(parsed, req);

        let resp = RpcResponse::err("req-1", error_codes::IMAGE_BUSY, "image mounted elsewhere");
        let resp_json = serde_json::to_string(&resp).expect("serialize response");
        let parsed_resp: RpcResponse =
            serde_json::from_str(&resp_json).expect("deserialize response");
        assert_eq!(
            parsed_resp.error.expect("error").code,
            error_codes::IMAGE_BUSY
        );
    }
}
