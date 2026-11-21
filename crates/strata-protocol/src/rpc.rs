//! JSON-RPC 2.0 wire envelope, method/event constants, and error codes aligned with SideVeil.

pub mod payloads;
pub mod tasks;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use payloads::*;
pub use tasks::*;

/// JSON-RPC method names exposed on `strata.sock` and reusable via SideVeil `plugin.invoke`.
pub mod methods {
    pub const DAEMON_PING: &str = "daemon.ping";
    pub const DAEMON_STATUS: &str = "daemon.status";
    pub const DAEMON_HEALTH: &str = "daemon.health";
    pub const DAEMON_RELOAD_CONFIG: &str = "daemon.reloadConfig";

    pub const CONTAINER_LIST: &str = "container.list";
    pub const CONTAINER_STATUS: &str = "container.status";
    pub const CONTAINER_START: &str = "container.start";
    pub const CONTAINER_STOP: &str = "container.stop";
    pub const CONTAINER_RESTART: &str = "container.restart";
    pub const CONTAINER_LOGS: &str = "container.logs";
    pub const CONTAINER_SYNC_DNS: &str = "container.syncDns";
    pub const CONTAINER_COMPACT: &str = "container.compact";
    pub const CONTAINER_FSCK: &str = "container.fsck";
    pub const CONTAINER_BACKUP: &str = "container.backup";
    pub const CONTAINER_BACKUP_START: &str = "container.backupStart";
    pub const CONTAINER_CONFIG_GET: &str = "container.configGet";
    pub const CONTAINER_CONFIG_PATCH: &str = "container.configPatch";
    pub const CONTAINER_SSH_KEY_LIST: &str = "container.sshKeyList";
    pub const CONTAINER_SSH_KEY_ADD: &str = "container.sshKeyAdd";

    pub const SESSION_ACQUIRE: &str = "session.acquire";
    pub const SESSION_RELEASE: &str = "session.release";

    pub const TASK_RUN: &str = "task.run";
    pub const TASK_CANCEL: &str = "task.cancel";
    pub const TASK_LIST: &str = "task.list";
    pub const TASK_LOGS: &str = "task.logs";

    pub const HUD_STATE: &str = "hud.state";
    pub const HUD_SET: &str = "hud.set";

    pub const EVENTS_SUBSCRIBE: &str = "events.subscribe";
}

/// Asynchronous event names pushed over `strata.sock` and SideVeil `plugin.event`.
pub mod events {
    pub const CONTAINER_STATE_CHANGED: &str = "container.stateChanged";
    pub const CONTAINER_MAINTENANCE_DONE: &str = "container.maintenanceDone";
    pub const TASK_OUTPUT: &str = "task.output";
    pub const TASK_EXIT: &str = "task.exit";
}

/// Standard JSON-RPC 2.0 and Strata domain error codes.
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32000;
    pub const PERMISSION_DENIED: i32 = -32003;
    pub const IMAGE_BUSY: i32 = -32020;
    pub const CONTAINER_NOT_FOUND: i32 = -32021;
    pub const CONTAINER_ALREADY_RUNNING: i32 = -32022;
    pub const CONTAINER_NOT_RUNNING: i32 = -32023;
    pub const INVALID_STATE: i32 = -32024;
    pub const TASK_NOT_FOUND: i32 = -32025;
}

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
    #[must_use]
    pub fn ok(id: impl Into<String>, result: Value) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id: id.into(),
            result: Some(result),
            error: None,
        }
    }

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
