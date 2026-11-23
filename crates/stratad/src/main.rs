//! Strata UID 0 container supervisor daemon and single-threaded stage-2/enter helper entry point.

mod config_store;
mod dispatcher;
mod error;
mod manager;
mod net_watcher;
mod rpc_server;
mod sideveil_hud;
mod sideveil_seam;
mod task_runner;

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use strata_protocol::constants::{DEFAULT_CONFIG_PATH, DEFAULT_RUN_DIR, DEFAULT_SOCKET_PATH};
use strata_runtime::{fs_atomic, spec_pipe};

use crate::config_store::ConfigStore;
use crate::dispatcher::Dispatcher;
use crate::manager::ContainerManager;
use crate::sideveil_seam::{EventHub, SideVeilBridgeSeam};
use crate::task_runner::TaskRunner;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    // Handle single-threaded helper re-exec modes BEFORE creating any Tokio worker threads.
    if let Some(mode) = args.get(1).map(String::as_str) {
        match mode {
            "__init" => return run_helper_mode(HelperMode::Init, &args[2..]),
            "__exec" => return run_helper_mode(HelperMode::Exec, &args[2..]),
            _ => {}
        }
    }

    let opts = match DaemonArgs::parse(&args[1..]) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("stratad: {msg}");
            return ExitCode::from(2);
        }
    };

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("stratad-worker")
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("stratad: failed to initialize async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };

    match rt.block_on(daemon_main(opts)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("stratad: fatal error: {err}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum HelperMode {
    Init,
    Exec,
}

fn run_helper_mode(mode: HelperMode, args: &[String]) -> ExitCode {
    let mut spec_fd: Option<RawFd> = None;
    let mut err_fd: Option<RawFd> = None;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--spec-fd" if idx + 1 < args.len() => {
                spec_fd = args[idx + 1].parse::<RawFd>().ok();
                idx += 2;
            }
            "--err-fd" if idx + 1 < args.len() => {
                err_fd = args[idx + 1].parse::<RawFd>().ok();
                idx += 2;
            }
            _ => idx += 1,
        }
    }

    let Some(sfd) = spec_fd else {
        spec_pipe::report_exec_error(err_fd, "missing --spec-fd argument");
        return ExitCode::from(2);
    };

    let res = match mode {
        HelperMode::Init => strata_runtime::run_stage2_from_fd(sfd, err_fd),
        HelperMode::Exec => strata_runtime::run_enter_from_fd(sfd, err_fd),
    };
    let Err(err) = res;
    let msg = err.to_string();
    spec_pipe::report_exec_error(err_fd, &msg);
    eprintln!("stratad helper error: {msg}");
    ExitCode::FAILURE
}

#[derive(Debug)]
struct DaemonArgs {
    config_path: PathBuf,
    run_dir: PathBuf,
    socket_path: PathBuf,
    module_dir: Option<PathBuf>,
}

impl DaemonArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut config_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let mut run_dir = PathBuf::from(DEFAULT_RUN_DIR);
        let mut socket_path: Option<PathBuf> = None;
        let mut module_dir: Option<PathBuf> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--config" if i + 1 < args.len() => {
                    config_path = PathBuf::from(&args[i + 1]);
                    i += 2;
                }
                "--run-dir" if i + 1 < args.len() => {
                    run_dir = PathBuf::from(&args[i + 1]);
                    i += 2;
                }
                "--socket" if i + 1 < args.len() => {
                    socket_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                "--module-dir" if i + 1 < args.len() => {
                    module_dir = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                other => return Err(format!("unknown or incomplete argument '{other}'")),
            }
        }

        let socket_path = socket_path.unwrap_or_else(|| {
            if run_dir == Path::new(DEFAULT_RUN_DIR) {
                PathBuf::from(DEFAULT_SOCKET_PATH)
            } else {
                run_dir.join("strata.sock")
            }
        });
        Ok(Self {
            config_path,
            run_dir,
            socket_path,
            module_dir,
        })
    }
}

async fn daemon_main(opts: DaemonArgs) -> Result<(), String> {
    let started_at = Instant::now();
    fs_atomic::ensure_dir_mode(&opts.run_dir, 0o700).map_err(|e| e.to_string())?;
    let logs_dir = opts.run_dir.join("logs");
    fs_atomic::ensure_dir_mode(&logs_dir, 0o700).map_err(|e| e.to_string())?;

    let pid_file = opts.run_dir.join("stratad.pid");
    let _ = fs_atomic::write_atomic(
        &pid_file,
        format!("{}\n", std::process::id()).as_bytes(),
        0o600,
    );

    let self_exe = std::env::current_exe().map_err(|e| format!("resolve current_exe: {e}"))?;
    let config_store = Arc::new(ConfigStore::load_or_seed(opts.config_path)?);
    let event_hub = Arc::new(EventHub::new());
    let sideveil = Arc::new(SideVeilBridgeSeam::new());

    let manager = Arc::new(ContainerManager::new(
        Arc::clone(&config_store),
        Arc::clone(&event_hub),
        self_exe.clone(),
        logs_dir,
        opts.module_dir,
    ));
    let task_runner = Arc::new(TaskRunner::new(
        Arc::clone(&manager),
        Arc::clone(&event_hub),
        self_exe,
    ));

    let dispatcher = Arc::new(Dispatcher::new(
        started_at,
        Arc::clone(&config_store),
        Arc::clone(&manager),
        Arc::clone(&task_runner),
        Arc::clone(&sideveil),
    ));

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let inotify_handle = config_store.spawn_inotify_watcher(
        Arc::clone(&manager),
        Arc::clone(&sideveil),
        shutdown_rx.clone(),
    );
    let net_watcher_handle =
        net_watcher::spawn_net_watcher(Arc::clone(&manager), shutdown_rx.clone());
    let bridge_handle = sideveil.spawn_bridge(
        Arc::clone(&config_store),
        Arc::clone(&dispatcher),
        Arc::clone(&manager),
        Arc::clone(&task_runner),
        Arc::clone(&event_hub),
        shutdown_rx.clone(),
    );

    let server_handle = tokio::spawn(rpc_server::serve(
        opts.socket_path.clone(),
        Arc::clone(&dispatcher),
        Arc::clone(&manager),
        Arc::clone(&event_hub),
        shutdown_rx,
    ));

    manager.start_auto_containers().await;

    wait_for_shutdown_signal().await;
    eprintln!("stratad: received shutdown signal, draining containers cleanly...");
    let _ = shutdown_tx.send(true);
    task_runner.cancel_all().await;
    manager.stop_all().await;

    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server_handle).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), bridge_handle).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), net_watcher_handle).await;
    if let Some(handle) = inotify_handle {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    }
    let _ = std::fs::remove_file(&opts.socket_path);
    let _ = std::fs::remove_file(&pid_file);
    Ok(())
}

async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut sigterm = signal(SignalKind::terminate()).ok();
    let mut sigint = signal(SignalKind::interrupt()).ok();

    tokio::select! {
        _ = async {
            if let Some(s) = &mut sigterm {
                s.recv().await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {}
        _ = async {
            if let Some(s) = &mut sigint {
                s.recv().await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {}
    }
}
