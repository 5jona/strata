//! Secondary CLI subcommand handlers (`ssh-key`, `config`, `task`, `daemon`, `web`, `rpc`).

mod config;
mod tools;
mod tools_args;
mod web;

use std::fs;
use std::io::{self, Read};

use serde_json::{Value, json};
use strata_protocol::DaemonSummarySnapshot;
use strata_protocol::rpc::{
    ContainerSelectorParams, DaemonLogsParams, SshKeyAddParams, SshKeyRemoveParams, SshKeysResult,
    TaskCancelParams, TaskLogsParams, TaskLogsResult, TaskRunParams, TaskSummary, methods,
};

pub(crate) use config::handle_config;
pub(crate) use tools::handle_tools;
pub(crate) use web::handle_web;

use crate::client::StrataClient;
use crate::format;

pub(crate) fn handle_ssh_key(
    client: &mut StrataClient,
    container: Option<String>,
    rest: &[String],
    json_mode: bool,
) -> Result<i32, String> {
    let action = rest.first().map_or("list", String::as_str);
    match action {
        "list" => {
            let params = serde_json::to_value(ContainerSelectorParams { container }).ok();
            let val = rpc_or_err(client.call(methods::CONTAINER_SSH_KEY_LIST, params))?;
            emit_typed::<SshKeysResult>(&val, json_mode, format::print_ssh_keys)?;
            Ok(0)
        }
        "add" => {
            let Some(src) = rest.get(1) else {
                return Err("usage: strata ssh-key add <pubkey-file-or-string>".to_owned());
            };
            let key_text = if std::path::Path::new(src).is_file() {
                fs::read_to_string(src).map_err(|e| format!("read '{src}': {e}"))?
            } else {
                src.clone()
            };
            let params = serde_json::to_value(SshKeyAddParams {
                container,
                public_key: key_text,
            })
            .ok();
            let val = rpc_or_err(client.call(methods::CONTAINER_SSH_KEY_ADD, params))?;
            emit_typed::<SshKeysResult>(&val, json_mode, format::print_ssh_keys)?;
            Ok(0)
        }
        "remove" | "rm" => {
            let Some(target) = rest.get(1) else {
                return Err("usage: strata ssh-key remove <key-or-fingerprint>".to_owned());
            };
            let params = serde_json::to_value(SshKeyRemoveParams {
                id: container,
                fingerprint_or_key: target.clone(),
            })
            .ok();
            let val = rpc_or_err(client.call(methods::CONTAINER_SSH_KEY_REMOVE, params))?;
            if json_mode {
                print_json(&val);
            } else {
                let removed = val.get("removed").and_then(Value::as_u64).unwrap_or(0);
                println!("Removed {removed} SSH key(s).");
            }
            Ok(0)
        }
        other => Err(format!(
            "unknown ssh-key action '{other}' (expected list|add|remove)"
        )),
    }
}

pub(crate) fn handle_task(
    client: &mut StrataClient,
    container: Option<String>,
    rest: &[String],
    json_mode: bool,
) -> Result<i32, String> {
    let action = rest.first().map_or("list", String::as_str);
    match action {
        "list" => {
            let val = rpc_or_err(client.call(methods::TASK_LIST, None))?;
            emit_typed::<Vec<TaskSummary>>(&val, json_mode, |list| format::print_tasks(list))?;
            Ok(0)
        }
        "run" => {
            if rest.len() < 2 {
                return Err("usage: strata [-c <id>] task run <command> [args...]".to_owned());
            }
            client.run_streaming_task(
                TaskRunParams {
                    container,
                    command: rest[1..].to_vec(),
                    cwd: None,
                    timeout_secs: None,
                },
                json_mode,
            )
        }
        "cancel" => {
            let Some(tid) = rest.get(1) else {
                return Err("usage: strata task cancel <task_id>".to_owned());
            };
            let val = rpc_or_err(client.call(
                methods::TASK_CANCEL,
                Some(json!(TaskCancelParams {
                    task_id: tid.clone()
                })),
            ))?;
            if json_mode {
                print_json(&val);
            } else {
                println!("Cancel signal dispatched for task '{tid}'.");
            }
            Ok(0)
        }
        "logs" => {
            let Some(tid) = rest.get(1) else {
                return Err("usage: strata task logs <task_id> [limit]".to_owned());
            };
            let limit = rest.get(2).and_then(|s| s.parse::<usize>().ok());
            let val = rpc_or_err(client.call(
                methods::TASK_LOGS,
                Some(json!(TaskLogsParams {
                    task_id: tid.clone(),
                    limit,
                })),
            ))?;
            if json_mode {
                print_json(&val);
            } else {
                let parsed: TaskLogsResult =
                    serde_json::from_value(val).map_err(|e| format!("decode task.logs: {e}"))?;
                for entry in parsed.entries {
                    println!("[{}] {}", entry.seq, entry.chunk);
                }
            }
            Ok(0)
        }
        other => Err(format!(
            "unknown task action '{other}' (expected list|run|cancel|logs)"
        )),
    }
}

pub(crate) fn parse_raw_rpc_args(
    rest: &[String],
    stdin_payload: Option<&str>,
) -> Result<(String, Option<Value>), String> {
    let Some(method) = rest.first() else {
        return Err("usage: strata rpc <method> [json-params | -]".to_owned());
    };
    let raw_opt = match rest.get(1).map(String::as_str) {
        Some("-") => stdin_payload.map(str::trim).filter(|s| !s.is_empty()),
        Some(raw) => Some(raw),
        None => stdin_payload.map(str::trim).filter(|s| !s.is_empty()),
    };
    let params = match raw_opt {
        Some(raw) => Some(
            serde_json::from_str::<Value>(raw).map_err(|e| format!("invalid JSON params: {e}"))?,
        ),
        None => None,
    };
    Ok((method.clone(), params))
}

pub(crate) fn raw_rpc_needs_stdin(rest: &[String]) -> bool {
    matches!(rest.get(1).map(String::as_str), Some("-"))
}

pub(crate) fn handle_raw_rpc(client: &mut StrataClient, rest: &[String]) -> Result<i32, String> {
    let stdin_buf = if raw_rpc_needs_stdin(rest) {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("read stdin JSON: {e}"))?;
        Some(buf)
    } else {
        None
    };
    let (method, params) = parse_raw_rpc_args(rest, stdin_buf.as_deref())?;
    let val = rpc_or_err(client.call(&method, params))?;
    print_json(&val);
    Ok(0)
}

pub(crate) fn handle_daemon(
    client: &mut StrataClient,
    rest: &[String],
    json_mode: bool,
) -> Result<i32, String> {
    let action = rest.first().map_or("status", String::as_str);
    match action {
        "ping" => {
            let val = rpc_or_err(client.call(methods::DAEMON_PING, None))?;
            print_json(&val);
            Ok(0)
        }
        "health" => {
            let val = rpc_or_err(client.call(methods::DAEMON_HEALTH, None))?;
            print_json(&val);
            Ok(0)
        }
        "capabilities" | "caps" => {
            let val = rpc_or_err(client.call(methods::DAEMON_CAPABILITIES, None))?;
            print_json(&val);
            Ok(0)
        }
        "logs" => {
            let lines = rest.get(1).and_then(|s| s.parse::<usize>().ok());
            let params = serde_json::to_value(DaemonLogsParams { lines }).ok();
            let val = rpc_or_err(client.call(methods::DAEMON_LOGS, params))?;
            print_json(&val);
            Ok(0)
        }
        "status" => {
            let val = rpc_or_err(client.call(methods::DAEMON_STATUS, None))?;
            emit_typed::<DaemonSummarySnapshot>(&val, json_mode, format::print_daemon_status)?;
            Ok(0)
        }
        other => Err(format!(
            "unknown daemon action '{other}' (expected status|ping|health|capabilities|logs)"
        )),
    }
}

pub(crate) fn rpc_or_err(res: Result<Value, (i32, String)>) -> Result<Value, String> {
    res.map_err(|(code, msg)| format!("error {code}: {msg}"))
}

pub(crate) fn emit_typed<T: serde::de::DeserializeOwned>(
    val: &Value,
    json_mode: bool,
    printer: impl FnOnce(&T),
) -> Result<(), String> {
    if json_mode {
        print_json(val);
        return Ok(());
    }
    let parsed: T =
        serde_json::from_value(val.clone()).map_err(|e| format!("decode result: {e}"))?;
    printer(&parsed);
    Ok(())
}

pub(crate) fn print_json(val: &Value) {
    println!("{}", serde_json::to_string_pretty(val).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raw_rpc_inline_and_stdin_json() {
        let (m, p) = parse_raw_rpc_args(&["daemon.ping".to_owned()], None).expect("no params");
        assert_eq!(m, "daemon.ping");
        assert!(p.is_none());
        assert!(!raw_rpc_needs_stdin(&["daemon.ping".to_owned()]));
        assert!(raw_rpc_needs_stdin(&[
            "web.configure".to_owned(),
            "-".to_owned()
        ]));

        let (m2, p2) = parse_raw_rpc_args(
            &[
                "container.status".to_owned(),
                r#"{"container":"main"}"#.to_owned(),
            ],
            None,
        )
        .expect("inline json");
        assert_eq!(m2, "container.status");
        assert_eq!(p2.expect("params")["container"], "main");

        let (m3, p3) = parse_raw_rpc_args(
            &["web.configure".to_owned(), "-".to_owned()],
            Some(r#"{"lanEnabled":true}"#),
        )
        .expect("stdin dash");
        assert_eq!(m3, "web.configure");
        assert_eq!(p3.expect("params")["lanEnabled"], true);

        assert!(parse_raw_rpc_args(&["x".to_owned(), "{bad".to_owned()], None).is_err());
    }
}
