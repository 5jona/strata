//! Strata multi-container CLI (`strata`, symlinked to `/data/adb/ksu/bin/strata`).

mod client;
mod commands;
mod format;
mod interactive;

use std::os::fd::RawFd;
use std::path::PathBuf;
use std::process::ExitCode;

use strata_protocol::ContainerStatusSnapshot;
use strata_protocol::constants::DEFAULT_SOCKET_PATH;
use strata_protocol::rpc::{
    BackupParams, BackupResult, CompactResult, ContainerLogsParams, ContainerLogsResult,
    ContainerSelectorParams, FsckResult, StorageCleanupReport, methods,
};

use crate::client::StrataClient;
use crate::commands::{emit_typed, print_json, rpc_or_err};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    // Handle internal single-threaded foreground stage-2 mode used for Oneshot / Debug leases.
    if args.get(1).map(String::as_str) == Some("__stage2") {
        return run_stage2_helper(&args[2..]);
    }

    match run_cli(&args[1..]) {
        Ok(code) => ExitCode::from(code as u8),
        Err(msg) => {
            eprintln!("strata: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run_stage2_helper(args: &[String]) -> ExitCode {
    let mut spec_fd: Option<RawFd> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--spec-fd" && i + 1 < args.len() {
            spec_fd = args[i + 1].parse::<RawFd>().ok();
            i += 2;
        } else {
            i += 1;
        }
    }
    let Some(fd) = spec_fd else {
        eprintln!("strata __stage2: missing --spec-fd");
        return ExitCode::from(2);
    };
    let Err(err) = strata_runtime::run_stage2_from_fd(fd, None);
    eprintln!("strata __stage2: {err}");
    ExitCode::FAILURE
}

fn run_cli(args: &[String]) -> Result<i32, String> {
    let mut container: Option<String> = None;
    let mut socket_path = PathBuf::from(DEFAULT_SOCKET_PATH);
    let mut json_mode = false;
    let mut idx = 0;

    while idx < args.len() {
        match args[idx].as_str() {
            "-c" | "--container" if idx + 1 < args.len() => {
                container = Some(args[idx + 1].clone());
                idx += 2;
            }
            "--socket" if idx + 1 < args.len() => {
                socket_path = PathBuf::from(&args[idx + 1]);
                idx += 2;
            }
            "--json" => {
                json_mode = true;
                idx += 1;
            }
            "-h" | "--help" | "help" => {
                print_usage();
                return Ok(0);
            }
            _ => break,
        }
    }

    let Some(subcmd) = args.get(idx).map(String::as_str) else {
        print_usage();
        return Ok(0);
    };
    let rest = &args[idx + 1..];
    let mut client = StrataClient::connect(&socket_path)?;
    let selector = || {
        serde_json::to_value(ContainerSelectorParams {
            container: container.clone(),
        })
        .ok()
    };

    match subcmd {
        "start" => {
            let val = rpc_or_err(client.call(methods::CONTAINER_START, selector()))?;
            emit_typed::<ContainerStatusSnapshot>(&val, json_mode, format::print_status)?;
            Ok(0)
        }
        "stop" => {
            let val = rpc_or_err(client.call(methods::CONTAINER_STOP, selector()))?;
            let target = container.as_deref().unwrap_or("main");
            emit_typed::<StorageCleanupReport>(&val, json_mode, |r| {
                format::print_cleanup(target, r);
            })?;
            Ok(0)
        }
        "restart" => {
            let val = rpc_or_err(client.call(methods::CONTAINER_RESTART, selector()))?;
            emit_typed::<ContainerStatusSnapshot>(&val, json_mode, format::print_status)?;
            Ok(0)
        }
        "status" => {
            let val = rpc_or_err(client.call(methods::CONTAINER_STATUS, selector()))?;
            emit_typed::<ContainerStatusSnapshot>(&val, json_mode, format::print_status)?;
            Ok(0)
        }
        "list" => {
            let val = rpc_or_err(client.call(methods::CONTAINER_LIST, None))?;
            emit_typed::<Vec<ContainerStatusSnapshot>>(&val, json_mode, |list| {
                format::print_list(list);
            })?;
            Ok(0)
        }
        "enter" => interactive::run_interactive(
            client,
            container,
            false,
            vec!["/bin/bash".to_owned(), "-l".to_owned()],
        ),
        "exec" => {
            if rest.is_empty() {
                return Err("usage: strata [-c <id>] exec <command> [args...]".to_owned());
            }
            interactive::run_interactive(client, container, false, rest.to_vec())
        }
        "py" => {
            let mut cmd = vec!["/usr/bin/python3".to_owned()];
            cmd.extend(rest.iter().cloned());
            interactive::run_interactive(client, container, false, cmd)
        }
        "debug" => {
            let cmd = if rest.is_empty() {
                vec!["/bin/bash".to_owned(), "-l".to_owned()]
            } else {
                rest.to_vec()
            };
            interactive::run_interactive(client, container, true, cmd)
        }
        "dns" => {
            let val = rpc_or_err(client.call(methods::CONTAINER_SYNC_DNS, selector()))?;
            if json_mode {
                print_json(&val);
            } else {
                println!(
                    "Synchronized /etc/resolv.conf, /run/strata/host.env, and bridge scripts."
                );
            }
            Ok(0)
        }
        "logs" => {
            let lines = rest.first().and_then(|s| s.parse::<usize>().ok());
            let params = serde_json::to_value(ContainerLogsParams { container, lines }).ok();
            let val = rpc_or_err(client.call(methods::CONTAINER_LOGS, params))?;
            emit_typed::<ContainerLogsResult>(&val, json_mode, format::print_logs)?;
            Ok(0)
        }
        "ssh-key" => commands::handle_ssh_key(&mut client, container, rest, json_mode),
        "compact" => {
            let val = rpc_or_err(client.call(methods::CONTAINER_COMPACT, selector()))?;
            emit_typed::<CompactResult>(&val, json_mode, format::print_compact)?;
            Ok(0)
        }
        "fsck" => {
            let val = rpc_or_err(client.call(methods::CONTAINER_FSCK, selector()))?;
            emit_typed::<FsckResult>(&val, json_mode, format::print_fsck)?;
            Ok(0)
        }
        "backup" => {
            let Some(out_path) = rest.first() else {
                return Err("usage: strata [-c <id>] backup <output.img.gz>".to_owned());
            };
            let params = serde_json::to_value(BackupParams {
                container,
                output_path: out_path.clone(),
            })
            .ok();
            let val = rpc_or_err(client.call(methods::CONTAINER_BACKUP, params))?;
            emit_typed::<BackupResult>(&val, json_mode, format::print_backup)?;
            Ok(0)
        }
        "config" => commands::handle_config(&mut client, container, rest, json_mode),
        "task" => commands::handle_task(&mut client, container, rest, json_mode),
        "daemon" => commands::handle_daemon(&mut client, rest, json_mode),
        "rpc" => commands::handle_raw_rpc(&mut client, rest),
        other => Err(format!(
            "unknown subcommand '{other}' (run 'strata --help')"
        )),
    }
}

fn print_usage() {
    println!(
        "\
Usage: strata [-c <container>] [--json] [--socket <path>] <command> [args...]

Commands:
  start                       Start supervised container service (Dropbear SSH)
  stop                        Stop container with zero-residual mount/loop teardown
  restart                     Restart container cleanly
  status                      Show container runtime, SSH, mount, loop, and disk status
  list                        List all configured containers
  enter                       Open an interactive bash login shell (local setns)
  exec <cmd...>               Execute a command inside the container
  py [script] [args...]       Run Python 3 inside the container
  debug [cmd...]              Enter standalone full host-runtime mode (/dev,/sys,/proc)
  dns                         Sync /etc/resolv.conf, /run/strata/host.env, and bridges
  logs [lines]                Show recent Dropbear/init logs
  ssh-key <list|add <key>>    Inspect or append authorized SSH public keys
  compact                     Run native FITRIM ioctl to reclaim sparse image blocks
  fsck                        Stop container and run e2fsck -f -y on rootfs image
  backup <out.img.gz>         Stop container and write atomic gzip-compressed backup
  config <show|reload|set-*>  Inspect or update container configuration
  task <run|list|cancel>      Run or inspect daemon-hosted streaming tasks
  daemon <status|ping|health> Inspect stratad daemon health and summary"
    );
}
