//! CLI subcommand handler for `strata web` (status, token, rotate-token, lan, port, enable/disable).

use std::fs;

use serde_json::json;
use strata_protocol::constants::DEFAULT_WEB_TOKEN_PATH;
use strata_protocol::rpc::{WebConfigureParams, WebRotateTokenResult, WebStatusResult, methods};

use crate::client::StrataClient;
use crate::commands::{emit_typed, print_json, rpc_or_err};

/// Parsed action for `strata web <action>`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WebCliAction {
    Status,
    Token,
    RotateToken,
    Configure(WebConfigureParams),
}

/// Parses `strata web` arguments into a structured action.
pub(crate) fn parse_web_action(rest: &[String]) -> Result<WebCliAction, String> {
    let action = rest.first().map_or("status", String::as_str);
    match action {
        "status" => Ok(WebCliAction::Status),
        "token" => Ok(WebCliAction::Token),
        "rotate-token" => Ok(WebCliAction::RotateToken),
        "lan" => match rest.get(1).map(String::as_str) {
            Some("on" | "true" | "1") => Ok(WebCliAction::Configure(WebConfigureParams {
                lan_enabled: Some(true),
                ..WebConfigureParams::default()
            })),
            Some("off" | "false" | "0") => Ok(WebCliAction::Configure(WebConfigureParams {
                lan_enabled: Some(false),
                ..WebConfigureParams::default()
            })),
            _ => Err("usage: strata web lan <on|off>".to_owned()),
        },
        "enable" => Ok(WebCliAction::Configure(WebConfigureParams {
            enabled: Some(true),
            ..WebConfigureParams::default()
        })),
        "disable" => Ok(WebCliAction::Configure(WebConfigureParams {
            enabled: Some(false),
            ..WebConfigureParams::default()
        })),
        "port" => {
            let port = rest
                .get(1)
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|&p| p > 0)
                .ok_or_else(|| "usage: strata web port <1..65535>".to_owned())?;
            Ok(WebCliAction::Configure(WebConfigureParams {
                port: Some(port),
                ..WebConfigureParams::default()
            }))
        }
        other => Err(format!(
            "unknown web action '{other}' (expected status|token|rotate-token|lan|port|enable|disable)"
        )),
    }
}

/// Executes `strata web ...` against `stratad`.
pub(crate) fn handle_web(
    client: &mut StrataClient,
    rest: &[String],
    json_mode: bool,
) -> Result<i32, String> {
    match parse_web_action(rest)? {
        WebCliAction::Status => {
            let val = rpc_or_err(client.call(methods::WEB_STATUS, None))?;
            emit_typed::<WebStatusResult>(&val, json_mode, print_web_status)?;
            Ok(0)
        }
        WebCliAction::Token => {
            let status_val = rpc_or_err(client.call(methods::WEB_STATUS, None))?;
            let token_path = status_val
                .get("tokenPath")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(DEFAULT_WEB_TOKEN_PATH);
            let token = fs::read_to_string(token_path)
                .map_err(|e| format!("read web token '{token_path}': {e}"))?
                .trim()
                .to_owned();
            if json_mode {
                print_json(&json!({ "token": token, "tokenPath": token_path }));
            } else {
                println!("{token}");
            }
            Ok(0)
        }
        WebCliAction::RotateToken => {
            let val = rpc_or_err(client.call(methods::WEB_ROTATE_TOKEN, None))?;
            emit_typed::<WebRotateTokenResult>(&val, json_mode, |res| {
                println!(
                    "Rotated web console token ({} active session(s) revoked).",
                    res.sessions_revoked
                );
                println!("token_path: {}", res.token_path);
                println!("token     : {}", res.token);
            })?;
            Ok(0)
        }
        WebCliAction::Configure(params) => {
            let val = rpc_or_err(client.call(
                methods::WEB_CONFIGURE,
                Some(serde_json::to_value(&params).map_err(|e| e.to_string())?),
            ))?;
            emit_typed::<WebStatusResult>(&val, json_mode, print_web_status)?;
            Ok(0)
        }
    }
}

fn print_web_status(s: &WebStatusResult) {
    println!(
        "web_console   : {} (running={})",
        if s.enabled { "enabled" } else { "disabled" },
        s.running
    );
    println!(
        "listen        : {}:{} (lan={})",
        s.listen_addr, s.port, s.lan_enabled
    );
    println!("token_path    : {}", s.token_path);
    println!(
        "webroot       : {}",
        if s.webroot_available {
            "ready"
        } else {
            "not-built (API-only)"
        }
    );
    println!(
        "sessions      : {} active PTY, {} active job(s)",
        s.active_pty_sessions, s.active_jobs
    );
    if !s.lan_urls.is_empty() {
        println!("urls          : {}", s.lan_urls.join(", "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_web_cli_subcommands() {
        assert_eq!(parse_web_action(&[]), Ok(WebCliAction::Status));
        assert_eq!(
            parse_web_action(&["token".to_owned()]),
            Ok(WebCliAction::Token)
        );
        assert_eq!(
            parse_web_action(&["rotate-token".to_owned()]),
            Ok(WebCliAction::RotateToken)
        );
        assert_eq!(
            parse_web_action(&["lan".to_owned(), "on".to_owned()]),
            Ok(WebCliAction::Configure(WebConfigureParams {
                lan_enabled: Some(true),
                ..WebConfigureParams::default()
            }))
        );
        assert_eq!(
            parse_web_action(&["lan".to_owned(), "off".to_owned()]),
            Ok(WebCliAction::Configure(WebConfigureParams {
                lan_enabled: Some(false),
                ..WebConfigureParams::default()
            }))
        );
        assert_eq!(
            parse_web_action(&["port".to_owned(), "8088".to_owned()]),
            Ok(WebCliAction::Configure(WebConfigureParams {
                port: Some(8088),
                ..WebConfigureParams::default()
            }))
        );
        assert!(parse_web_action(&["port".to_owned(), "0".to_owned()]).is_err());
        assert!(parse_web_action(&["lan".to_owned(), "maybe".to_owned()]).is_err());
    }
}
