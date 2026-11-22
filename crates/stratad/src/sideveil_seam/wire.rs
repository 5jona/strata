//! SideVeil `hub.sock` NDJSON wire framing, plugin handshake, and bounded line reader.

use std::io;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use strata_protocol::{EventEnvelope, RpcRequest};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_FRAME_BYTES: usize = 256 * 1024;

pub(super) const SIDEVEIL_HUD_UPDATE_SCENE: &str = "hud.updateScene";
pub(super) const SIDEVEIL_HUD_DISMISS: &str = "hud.dismiss";

pub(super) const HUB_EVENT_APPEARANCE_CHANGED: &str = "appearance.changed";
pub(super) const HUB_EVENT_PANEL_VISIBILITY_CHANGED: &str = "panel.visibilityChanged";
pub(super) const HUB_EVENT_NETWORK_CONTEXT_CHANGED: &str = "network.contextChanged";

/// Inbound NDJSON frame sent from SideVeil Hub to this plugin backend.
#[derive(Debug)]
pub(super) enum InboundHubFrame {
    /// Standard JSON-RPC request forwarded from a WebUI or Hub caller.
    Rpc(RpcRequest),
    /// Unsolicited Hub -> Backend system notification (`{"event": "...", "payload": ...}`).
    SystemEvent {
        event: String,
        payload: Option<Value>,
    },
}

/// Parses an inbound NDJSON line from SideVeil Hub as either an RPC request or a system event.
pub(super) fn parse_inbound_frame(trimmed: &str) -> Option<InboundHubFrame> {
    let value: Value = serde_json::from_str(trimmed).ok()?;
    let obj = value.as_object()?;

    if obj.contains_key("id") && obj.contains_key("method") {
        let req: RpcRequest = serde_json::from_value(value).ok()?;
        return Some(InboundHubFrame::Rpc(req));
    }

    if let Some(event) = obj.get("event").and_then(Value::as_str)
        && !event.is_empty()
    {
        let payload = obj.get("payload").cloned();
        return Some(InboundHubFrame::SystemEvent {
            event: event.to_owned(),
            payload,
        });
    }

    None
}

/// Extracts dark/light theme preference from a SideVeil `appearance.changed` event payload.
pub(super) fn extract_appearance_is_dark(payload: Option<&Value>) -> Option<bool> {
    let val = payload?;
    if let Some(flag) = val
        .get("isDark")
        .or_else(|| val.get("dark"))
        .and_then(Value::as_bool)
    {
        return Some(flag);
    }
    for key in ["mode", "theme", "appearance"] {
        if let Some(raw) = val.get(key).and_then(Value::as_str) {
            if raw.eq_ignore_ascii_case("dark") {
                return Some(true);
            }
            if raw.eq_ignore_ascii_case("light") {
                return Some(false);
            }
        }
    }
    for nested_key in ["appearance", "theme"] {
        if let Some(nested) = val.get(nested_key).filter(|v| v.is_object())
            && let Some(flag) = extract_appearance_is_dark(Some(nested))
        {
            return Some(flag);
        }
    }
    None
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SideVeilPluginHello<'a> {
    role: &'static str,
    plugin_id: &'a str,
    package_name: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SideVeilHandshakeAck {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SideVeilBackendEvent<'a> {
    event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<&'a Value>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    ephemeral: bool,
}

/// Connects to SideVeil's `hub.sock` and completes the `pluginBackend` hello handshake.
pub(super) async fn connect_and_handshake(
    hub_sock: &str,
    plugin_id: &str,
) -> io::Result<(
    BufReader<tokio::net::unix::OwnedReadHalf>,
    tokio::net::unix::OwnedWriteHalf,
)> {
    let stream = UnixStream::connect(hub_sock).await?;
    let (read_half, mut write_half) = stream.into_split();
    let hello = SideVeilPluginHello {
        role: "pluginBackend",
        plugin_id,
        package_name: None,
    };
    let mut bytes =
        serde_json::to_vec(&hello).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    bytes.push(b'\n');
    write_half.write_all(&bytes).await?;
    write_half.flush().await?;

    let mut reader = BufReader::new(read_half);
    let ack_line = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_bounded_line(&mut reader))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "SideVeil handshake timed out"))??
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SideVeil closed connection before handshake ack",
            )
        })?;

    let ack: SideVeilHandshakeAck = serde_json::from_str(ack_line.trim())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if !ack.ok {
        return Err(io::Error::other(ack.error.unwrap_or_else(|| {
            "SideVeil rejected plugin handshake".to_owned()
        })));
    }
    Ok((reader, write_half))
}

/// Serializes an outbound `EventEnvelope` as a SideVeil backend event NDJSON line.
pub(super) fn encode_backend_event(env: &EventEnvelope, ephemeral: bool) -> Option<String> {
    let frame = SideVeilBackendEvent {
        event: &env.event,
        payload: env.payload.as_ref(),
        ephemeral,
    };
    serde_json::to_string(&frame).ok()
}

/// Reads a single UTF-8 NDJSON line bounded to `MAX_FRAME_BYTES`.
pub(super) async fn read_bounded_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> io::Result<Option<String>> {
    let mut buf = Vec::with_capacity(256);
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if buf.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            if buf.len() + pos > MAX_FRAME_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "SideVeil NDJSON frame exceeds 256 KiB",
                ));
            }
            buf.extend_from_slice(&available[..pos]);
            reader.consume(pos + 1);
            break;
        }
        if buf.len() + available.len() > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SideVeil NDJSON frame exceeds 256 KiB",
            ));
        }
        let len = available.len();
        buf.extend_from_slice(available);
        reader.consume(len);
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_hub_system_notification_frames_and_appearance_payloads() {
        let rpc_line = r#"{"id":"rpc_1","method":"daemon.ping"}"#;
        assert!(matches!(
            parse_inbound_frame(rpc_line),
            Some(InboundHubFrame::Rpc(req)) if req.id == "rpc_1" && req.method == "daemon.ping"
        ));

        for (raw, expected_event) in [
            (
                r#"{"event":"appearance.changed","payload":{"isDark":false}}"#,
                HUB_EVENT_APPEARANCE_CHANGED,
            ),
            (
                r#"{"event":"network.contextChanged","payload":{"transport":"wifi"}}"#,
                HUB_EVENT_NETWORK_CONTEXT_CHANGED,
            ),
            (
                r#"{"event":"panel.visibilityChanged","payload":{"visible":true}}"#,
                HUB_EVENT_PANEL_VISIBILITY_CHANGED,
            ),
        ] {
            match parse_inbound_frame(raw) {
                Some(InboundHubFrame::SystemEvent { event, payload }) => {
                    assert_eq!(event, expected_event);
                    if event == HUB_EVENT_APPEARANCE_CHANGED {
                        assert_eq!(extract_appearance_is_dark(payload.as_ref()), Some(false));
                    }
                }
                other => panic!("expected SystemEvent for {raw}, got {other:?}"),
            }
        }

        assert_eq!(
            extract_appearance_is_dark(Some(&json!({ "mode": "dark" }))),
            Some(true)
        );
        assert_eq!(
            extract_appearance_is_dark(Some(&json!({ "appearance": { "isDark": false } }))),
            Some(false)
        );
        assert_eq!(extract_appearance_is_dark(Some(&json!({}))), None);
    }
}
