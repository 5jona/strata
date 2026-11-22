//! Zero-polling `pidfd` + `AsyncFd` container start, stop, restart, and crash-backoff supervision.

use std::process::Child;
use std::sync::Arc;
use std::time::{Duration, Instant};

use strata_protocol::rpc::error_codes;
use strata_protocol::{ContainerConfig, ContainerPhase, ContainerStatusSnapshot};
use strata_runtime::storage;
use strata_sys::{namespace, pidfd::PidFd};
use tokio::io::unix::AsyncFd;

use crate::manager::launch::{build_stage2_spec, spawn_stage2_init};
use crate::manager::{ContainerManager, ContainerSlot, StopOutcome};

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
            live.restart_backoff.reset();
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
        slot: &Arc<ContainerSlot>,
        is_restart: bool,
    ) -> Result<(), (i32, String)> {
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
                .map_err(|e| (error_codes::INTERNAL_ERROR, format!("mount join: {e}")))?;

        if let Err(err) = mount_res {
            let msg = err.to_string();
            let code = err.rpc_code();
            self.mark_failed(id, slot, &msg).await;
            return Err((code, msg));
        }

        let spec = build_stage2_spec(id, cfg);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::ContainerLiveState;

    #[test]
    fn persisted_restart_backoff_escalates_across_restarts_and_resets_on_explicit_start() {
        let mut live = ContainerLiveState::default();
        live.prepare_start(false);
        live.phase = ContainerPhase::Running;

        // Consecutive rapid crashes must escalate: 2s -> 4s -> 8s -> 16s -> 30s -> 30s.
        let mut delays = Vec::new();
        for _ in 0..6 {
            let (_, _, _, delay) = live.record_unexpected_exit(250, "crashed".to_owned());
            delays.push(delay);
            live.prepare_start(true);
            live.phase = ContainerPhase::Running;
        }
        assert_eq!(delays, vec![2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);

        // Staying healthy past 60s resets the backoff window to 2s.
        let (_, _, _, post_healthy) = live.record_unexpected_exit(90_000, "late exit".to_owned());
        assert_eq!(post_healthy, 2_000);

        // Explicit user start resets accumulated failures back to 0.
        live.prepare_start(false);
        assert_eq!(live.restart_backoff.failures(), 0);
    }
}
