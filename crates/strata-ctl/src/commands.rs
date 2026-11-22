//! Secondary CLI subcommand handlers (`ssh-key`, `config`, `task`, `daemon`).

mod config;

use std::fs;

use serde_json::{Value, json};
use strata_protocol::DaemonSummarySnapshot;
use strata_protocol::rpc::{
    ContainerSelectorParams, SshKeyAddParams, SshKeysResult, TaskCancelParams, TaskLogsParams,
    TaskLogsResult, TaskRunParams, TaskSummary, methods,
};

pub(crate) use config::handle_config;

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
        other => Err(format!(
            "unknown ssh-key action '{other}' (expected list|add)"
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

pub(crate) fn handle_raw_rpc(client: &mut StrataClient, rest: &[String]) -> Result<i32, String> {
    let Some(method) = rest.first() else {
        return Err("usage: strata rpc <method> [json-params]".to_owned());
    };
    let params = match rest.get(1) {
        Some(raw) => Some(
            serde_json::from_str::<Value>(raw).map_err(|e| format!("invalid JSON params: {e}"))?,
        ),
        None => None,
    };
    let val = rpc_or_err(client.call(method, params))?;
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
        "status" => {
            let val = rpc_or_err(client.call(methods::DAEMON_STATUS, None))?;
            emit_typed::<DaemonSummarySnapshot>(&val, json_mode, format::print_daemon_status)?;
            Ok(0)
        }
        other => Err(format!(
            "unknown daemon action '{other}' (expected status|ping|health)"
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
