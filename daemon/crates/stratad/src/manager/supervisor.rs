//! Zero-polling `pidfd` + `AsyncFd` container start, stop, restart, and crash-backoff supervision.

use std::process::Child;
use std::sync::Arc;
use std::time::{Duration, Instant};

use strata_protocol::{ContainerConfig, ContainerPhase, ContainerStatusSnapshot};
use strata_runtime::storage;
use strata_sys::{namespace, pidfd::PidFd};
use tokio::io::unix::AsyncFd;

use crate::error::{DaemonError, DaemonResult};
use crate::manager::launch::{build_stage2_spec, spawn_stage2_init};
use crate::manager::{ContainerManager, ContainerSlot, StopOutcome};

impl ContainerManager {
    /// Starts a container (idempotent when already running) and attaches zero-polling supervision.
    pub(crate) async fn start_container(
        self: &Arc<Self>,
        requested: Option<&str>,
    ) -> DaemonResult<ContainerStatusSnapshot> {
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

        if self.leases.has_temp_mount_lease(&id).await {
            return Err(DaemonError::TempLeaseActive(id));
        }

        self.start_locked(&id, &cfg, &slot, false).await?;
        Ok(self.build_snapshot(&id, &cfg, is_default).await)
    }

    /// Stops a container cleanly and returns zero-residual mount/loop verification metrics.
    pub(crate) async fn stop_container(self: &Arc<Self>, requested: Option<&str>) -> StopOutcome {
        let (id, cfg, _) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        self.stop_locked(&id, &cfg, &slot).await
    }

    /// Stops a container while the caller already holds `slot.lifecycle_lock`.
    pub(crate) async fn stop_locked(
        self: &Arc<Self>,
        id: &str,
        cfg: &ContainerConfig,
        slot: &Arc<ContainerSlot>,
    ) -> StopOutcome {
        let (prev_phase, ns_pid, mnt_ns) = {
            let mut live = slot.state.write().await;
            let prev = live.phase;
            live.desired_running = false;
            live.generation = live.generation.saturating_add(1);
            live.phase = ContainerPhase::Stopping;
            live.restart_backoff.reset();
            (prev, live.ns_pid, live.mnt_ns_inode)
        };
        self.emit_state_changed(id, prev_phase, ContainerPhase::Stopping)
            .await;

        // Close active PTY sessions and stop supervised background services before rootfs teardown.
        if let Some(ptys) = self.pty_registry.read().await.clone() {
            ptys.close_container_sessions(id).await;
        }
        if let Some(services) = self.service_supervisor.read().await.clone() {
            services.stop_container_services(id).await;
        }

        let cid = id.to_owned();
        let cfg_owned = cfg.clone();
        let report = tokio::task::spawn_blocking(move || {
            storage::teardown_container(&cid, &cfg_owned, ns_pid, mnt_ns)
        })
        .await
        .map_err(|e| DaemonError::join("teardown join", e))??;

        {
            let mut live = slot.state.write().await;
            live.phase = ContainerPhase::Stopped;
            live.ns_pid = None;
            live.mnt_ns_inode = None;
            live.started_at = None;
            live.last_error = None;
        }
        self.emit_state_changed(id, ContainerPhase::Stopping, ContainerPhase::Stopped)
            .await;
        Ok(report)
    }

    /// Restarts a container cleanly (stop -> zero-residual teardown -> start).
    pub(crate) async fn restart_container(
        self: &Arc<Self>,
        requested: Option<&str>,
    ) -> DaemonResult<ContainerStatusSnapshot> {
        let (id, cfg, is_default) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        if self.leases.has_temp_mount_lease(&id).await {
            return Err(DaemonError::TempLeaseActive(id));
        }
        let _ = self.stop_locked(&id, &cfg, &slot).await?;
        self.start_locked(&id, &cfg, &slot, false).await?;
        Ok(self.build_snapshot(&id, &cfg, is_default).await)
    }

    pub(crate) async fn start_locked(
        self: &Arc<Self>,
        id: &str,
        cfg: &ContainerConfig,
        slot: &Arc<ContainerSlot>,
        is_restart: bool,
    ) -> DaemonResult<()> {
        let prev_phase = {
            let mut live = slot.state.write().await;
            live.prepare_start(is_restart)
        };
        self.emit_state_changed(id, prev_phase, ContainerPhase::Starting)
            .await;

        let mount_cfg = cfg.clone();
        let mdir = self.module_dir.clone();
        let mount_res =
            tokio::task::spawn_blocking(move || storage::mount_rootfs(&mount_cfg, mdir.as_deref()))
                .await
                .map_err(|e| DaemonError::join("mount join", e))?;

        if let Err(err) = mount_res {
            let daemon_err = DaemonError::from(err);
            self.mark_failed(id, slot, &daemon_err.to_string()).await;
            return Err(daemon_err);
        }

        if cfg.ssh.enabled && cfg.init_command.is_empty() {
            let dropbear_bin = std::path::Path::new(&cfg.mount_dir).join("usr/sbin/dropbear");
            if !dropbear_bin.is_file() {
                let cid = id.to_owned();
                let cleanup_cfg = cfg.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    storage::teardown_container(&cid, &cleanup_cfg, None, None)
                })
                .await;
                let msg = format!(
                    "Dropbear SSH is enabled for '{id}' but /usr/sbin/dropbear is missing in rootfs. Disable ssh.enabled to boot as a clean container and install dropbear-bin + host keys via terminal, or recreate without enableSsh."
                );
                self.mark_failed(id, slot, &msg).await;
                return Err(DaemonError::InvalidState(msg));
            }
        }

        let spec = build_stage2_spec(id, cfg);
        let self_exe = self.self_exe.clone();
        let log_path = self.log_file_path(id);
        let spawn_res =
            tokio::task::spawn_blocking(move || spawn_stage2_init(&self_exe, &log_path, &spec))
                .await
                .map_err(|e| DaemonError::join("spawn join", e))?;

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
                return Err(DaemonError::Internal(msg));
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

        if let Some(services) = self.service_supervisor.read().await.clone() {
            let cid = id.to_owned();
            tokio::spawn(async move {
                services.start_auto_services(&cid).await;
            });
        }

        self.spawn_exit_monitor(id.to_owned(), Arc::clone(slot), child, pidfd, generation);
        Ok(())
    }

    async fn mark_failed(&self, id: &str, slot: &Arc<ContainerSlot>, msg: &str) {
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
        slot: Arc<ContainerSlot>,
        mut child: Child,
        pidfd: PidFd,
        generation: u64,
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

            let delay_ms = {
                let _lock = slot.lifecycle_lock.lock().await;
                let mut live = slot.state.write().await;
                if live.generation != generation || !live.desired_running {
                    return;
                }
                let ran_for_ms = started.elapsed().as_millis() as u64;
                let (prev, ns_pid, mnt_ns, delay_ms) = live.record_unexpected_exit(
                    ran_for_ms,
                    format!("container init exited ({exit_status:?})"),
                );
                drop(live);

                mgr.emit_state_changed(&id, prev, ContainerPhase::Failed)
                    .await;
                if let Some(ptys) = mgr.pty_registry.read().await.clone() {
                    ptys.close_container_sessions(&id).await;
                }
                if let Some(services) = mgr.service_supervisor.read().await.clone() {
                    services.stop_container_services(&id).await;
                }
                if let Ok((_, cfg, _)) = mgr.resolve_container(Some(&id)).await {
                    let cid = id.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        storage::teardown_container(&cid, &cfg, ns_pid, mnt_ns)
                    })
                    .await;
                }
                delay_ms
            };

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
