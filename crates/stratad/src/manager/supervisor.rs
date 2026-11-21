//! Zero-polling `pidfd` + `AsyncFd` container start, stop, restart, and crash-backoff supervision.

use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use strata_protocol::constants::DEFAULT_LOG_MAX_BYTES;
use strata_protocol::rpc::{StorageCleanupReport, error_codes};
use strata_protocol::{ContainerConfig, ContainerPhase, ContainerStatusSnapshot, RestartBackoff};
use strata_runtime::{Stage2Spec, fs_atomic, spec_pipe, storage};
use strata_sys::{namespace, pidfd::PidFd};
use tokio::io::unix::AsyncFd;

use crate::manager::{ContainerManager, StopOutcome};

impl ContainerManager {
    /// Starts a container (idempotent when already running) and attaches zero-polling supervision.
    pub(crate) async fn start_container(
        self: &Arc<Self>,
        requested: Option<&str>,
    ) -> Result<ContainerStatusSnapshot, (i32, String)> {
        let (id, cfg, is_default) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;

        {
            let live = slot.state.read().await;
            if live.phase.is_running() && live.ns_pid.is_some_and(namespace::pid_exists) {
                drop(live);
                let _ = self.sync_dns(Some(&id)).await;
                return Ok(self.build_snapshot(&id, &cfg, is_default).await);
            }
        }

        self.start_locked(&id, &cfg, &slot, false).await?;
        Ok(self.build_snapshot(&id, &cfg, is_default).await)
    }

    /// Stops a container cleanly and returns zero-residual mount/loop verification metrics.
    pub(crate) async fn stop_container(self: &Arc<Self>, requested: Option<&str>) -> StopOutcome {
        let (id, cfg, _) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;

        let (prev_phase, ns_pid, mnt_ns) = {
            let mut live = slot.state.write().await;
            let prev = live.phase;
            live.desired_running = false;
            live.generation = live.generation.saturating_add(1);
            live.phase = ContainerPhase::Stopping;
            (prev, live.ns_pid, live.mnt_ns_inode)
        };
        self.emit_state_changed(&id, prev_phase, ContainerPhase::Stopping)
            .await;

        let cid = id.clone();
        let report = tokio::task::spawn_blocking(move || {
            storage::teardown_container(&cid, &cfg, ns_pid, mnt_ns)
        })
        .await
        .map_err(|e| (error_codes::INTERNAL_ERROR, format!("teardown join: {e}")))?
        .map_err(|e| (e.rpc_code(), e.to_string()))?;

        {
            let mut live = slot.state.write().await;
            live.phase = ContainerPhase::Stopped;
            live.ns_pid = None;
            live.mnt_ns_inode = None;
            live.started_at = None;
            live.last_error = None;
        }
        self.emit_state_changed(&id, ContainerPhase::Stopping, ContainerPhase::Stopped)
            .await;
        Ok(report)
    }

    /// Restarts a container cleanly (stop -> zero-residual teardown -> start).
    pub(crate) async fn restart_container(
        self: &Arc<Self>,
        requested: Option<&str>,
    ) -> Result<ContainerStatusSnapshot, (i32, String)> {
        let (id, _, _) = self.resolve_container(requested).await?;
        let _ = self.stop_container(Some(&id)).await?;
        self.start_container(Some(&id)).await
    }

    pub(crate) async fn start_locked(
        self: &Arc<Self>,
        id: &str,
        cfg: &ContainerConfig,
        slot: &Arc<crate::manager::ContainerSlot>,
        is_restart: bool,
    ) -> Result<(), (i32, String)> {
        let prev_phase = {
            let mut live = slot.state.write().await;
            let prev = live.phase;
            live.phase = ContainerPhase::Starting;
            live.desired_running = true;
            live.last_error = None;
            live.generation = live.generation.saturating_add(1);
            prev
        };
        self.emit_state_changed(id, prev_phase, ContainerPhase::Starting)
            .await;

        let mount_cfg = cfg.clone();
        let mdir = self.module_dir.clone();
        let mount_res =
            tokio::task::spawn_blocking(move || storage::mount_rootfs(&mount_cfg, mdir.as_deref()))
                .await
                .map_err(|e| (error_codes::INTERNAL_ERROR, format!("mount join: {e}")))?;

        if let Err(err) = mount_res {
            let msg = err.to_string();
            let code = err.rpc_code();
            self.mark_failed(id, slot, &msg).await;
            return Err((code, msg));
        }

        let init_cmd = cfg.effective_init_command();
        let spec = Stage2Spec {
            container_id: id.to_owned(),
            mount_dir: cfg.mount_dir.clone(),
            hostname: cfg.hostname.clone(),
            oom_score_adj: cfg.oom_score_adj,
            cap_profile: cfg.cap_profile(),
            host_runtime: cfg.host_runtime,
            bind_data_local_tmp: cfg.bind_data_local_tmp,
            extra_char_devs: cfg.extra_char_devs.clone(),
            extra_mounts: cfg.extra_mounts.clone(),
            command: init_cmd,
            workdir: Some("/root".to_owned()),
            term: Some("xterm-256color".to_owned()),
        };

        let self_exe = self.self_exe.clone();
        let log_path = self.log_file_path(id);
        let spawn_res =
            tokio::task::spawn_blocking(move || spawn_stage2_init(&self_exe, &log_path, &spec))
                .await
                .map_err(|e| (error_codes::INTERNAL_ERROR, format!("spawn join: {e}")))?;

        let (child, pid, pidfd, mnt_ns_inode) = match spawn_res {
            Ok(launched) => launched,
            Err(msg) => {
                let cid = id.to_owned();
                let cleanup_cfg = cfg.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    storage::teardown_container(&cid, &cleanup_cfg, None, None)
                })
                .await;
                self.mark_failed(id, slot, &msg).await;
                return Err((error_codes::INTERNAL_ERROR, msg));
            }
        };

        let generation = {
            let mut live = slot.state.write().await;
            live.phase = ContainerPhase::Running;
            live.ns_pid = Some(pid);
            live.mnt_ns_inode = mnt_ns_inode;
            live.started_at = Some(Instant::now());
            live.last_error = None;
            if is_restart {
                live.restart_count = live.restart_count.saturating_add(1);
            }
            live.generation
        };
        self.emit_state_changed(id, ContainerPhase::Starting, ContainerPhase::Running)
            .await;

        self.spawn_exit_monitor(
            id.to_owned(),
            Arc::clone(slot),
            child,
            pidfd,
            generation,
            RestartBackoff::standard(),
        );
        Ok(())
    }

    async fn mark_failed(&self, id: &str, slot: &Arc<crate::manager::ContainerSlot>, msg: &str) {
        let prev = {
            let mut live = slot.state.write().await;
            let prev = live.phase;
            live.phase = ContainerPhase::Failed;
            live.ns_pid = None;
            live.mnt_ns_inode = None;
            live.started_at = None;
            live.last_error = Some(msg.to_owned());
            prev
        };
        self.emit_state_changed(id, prev, ContainerPhase::Failed)
            .await;
    }

    fn spawn_exit_monitor(
        self: &Arc<Self>,
        id: String,
        slot: Arc<crate::manager::ContainerSlot>,
        mut child: Child,
        pidfd: PidFd,
        generation: u64,
        mut backoff: RestartBackoff,
    ) {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let started = Instant::now();
            // SAFETY: `PidFd` exclusively owns its non-blocking `OwnedFd` for its entire lifetime.
            if let Ok(async_fd) = unsafe { AsyncFd::register(pidfd) } {
                let _ = async_fd.readable().await;
            }
            let exit_status = tokio::task::spawn_blocking(move || child.wait())
                .await
                .ok()
                .and_then(Result::ok);

            let should_restart = {
                let _lock = slot.lifecycle_lock.lock().await;
                let mut live = slot.state.write().await;
                if live.generation != generation || !live.desired_running {
                    return;
                }
                let prev = live.phase;
                let ns_pid = live.ns_pid;
                let mnt_ns = live.mnt_ns_inode;
                live.phase = ContainerPhase::Failed;
                live.ns_pid = None;
                live.mnt_ns_inode = None;
                live.started_at = None;
                live.last_error = Some(format!("container init exited ({exit_status:?})"));
                drop(live);

                mgr.emit_state_changed(&id, prev, ContainerPhase::Failed)
                    .await;
                if let Ok((_, cfg, _)) = mgr.resolve_container(Some(&id)).await {
                    let cid = id.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        storage::teardown_container(&cid, &cfg, ns_pid, mnt_ns)
                    })
                    .await;
                }
                true
            };

            if !should_restart {
                return;
            }
            let delay_ms = backoff.record_exit(started.elapsed().as_millis() as u64);
            eprintln!("stratad: container '{id}' exited unexpectedly; restarting in {delay_ms}ms");
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;

            let _lock = slot.lifecycle_lock.lock().await;
            let live = slot.state.read().await;
            if live.generation != generation || !live.desired_running {
                return;
            }
            drop(live);
            if let Ok((_, cfg, _)) = mgr.resolve_container(Some(&id)).await
                && cfg.enabled
            {
                let _ = mgr.start_locked(&id, &cfg, &slot, true).await;
            }
        });
    }
}

fn spawn_stage2_init(
    self_exe: &std::path::Path,
    log_path: &std::path::Path,
    spec: &Stage2Spec,
) -> Result<(Child, u32, PidFd, Option<u64>), String> {
    let _ = fs_atomic::rotate_log_file(log_path, DEFAULT_LOG_MAX_BYTES);
    let log_out = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| format!("open log '{}': {e}", log_path.display()))?;
    let log_err = log_out
        .try_clone()
        .map_err(|e| format!("clone log fd: {e}"))?;

    let spec_fd = spec_pipe::create_spec_pipe(spec).map_err(|e| e.to_string())?;
    let (err_read, err_write) = spec_pipe::create_exec_error_pipe().map_err(|e| e.to_string())?;

    let mut child = Command::new(self_exe)
        .arg("__init")
        .arg("--spec-fd")
        .arg(spec_fd.as_raw_fd().to_string())
        .arg("--err-fd")
        .arg(err_write.as_raw_fd().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_out))
        .stderr(Stdio::from(log_err))
        .spawn()
        .map_err(|e| format!("spawn stage2 helper: {e}"))?;

    drop(spec_fd);
    drop(err_write);

    match spec_pipe::read_exec_error(err_read) {
        Ok(None) => {}
        Ok(Some(err_msg)) => {
            let _ = child.wait();
            return Err(err_msg);
        }
        Err(err) => {
            let _ = child.wait();
            return Err(err.to_string());
        }
    }

    let pid = child.id();
    let pidfd = PidFd::open(pid).map_err(|e| format!("pidfd_open({pid}): {e}"))?;
    let mnt_ns = namespace::read_ns_inode(pid, "mnt").ok();
    Ok((child, pid, pidfd, mnt_ns))
}

#[allow(dead_code)]
fn _assert_cleanup_report(_: StorageCleanupReport) {}
