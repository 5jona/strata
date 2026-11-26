//! Extended container storage & lifecycle operations: delete, resize, sparse clone, SSH key removal, and daemon logs.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use strata_protocol::constants::DEFAULT_CONTAINERS_ROOT;
use strata_protocol::rpc::{
    ContainerCloneParams, ContainerCloneResult, ContainerDeleteParams, ContainerDeleteResult,
    ContainerResizeParams, ContainerResizeResult, ContainerSetDefaultParams,
    ContainerSetDefaultResult, DaemonLogsParams, DaemonLogsResult, SshKeyRemoveParams,
    SshKeyRemoveResult,
};
use strata_protocol::{ContainerConfig, validate_container_id};
use strata_runtime::{provision, storage, sync};
use strata_sys::mount;

use crate::error::{DaemonError, DaemonResult};
use crate::jobs::create::MAX_CONTAINER_SIZE_MB;
use crate::manager::ContainerManager;

impl ContainerManager {
    /// Safely deletes a container after zero-residual teardown (`mounts == 0 && loops == 0`).
    pub(crate) async fn delete_container(
        self: &Arc<Self>,
        params: ContainerDeleteParams,
    ) -> DaemonResult<ContainerDeleteResult> {
        let (id, cfg, is_default) = self.resolve_container(Some(&params.id)).await?;
        if is_default && !params.force {
            return Err(DaemonError::InvalidParams(format!(
                "container '{id}' is the defaultContainer; pass force=true to delete"
            )));
        }

        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        self.ensure_no_temp_lease(&id).await?;

        let cleanup = self.stop_locked(&id, &cfg, &slot).await?;
        if cleanup.residual_mounts != 0 || cleanup.residual_loops != 0 {
            return Err(DaemonError::Internal(format!(
                "refusing delete of '{id}': residual_mounts={} residual_loops={}",
                cleanup.residual_mounts, cleanup.residual_loops
            )));
        }

        let cid = id.clone();
        let img = PathBuf::from(&cfg.image_path);
        let mnt = PathBuf::from(&cfg.mount_dir);
        let del_img = params.delete_image;
        let image_deleted = tokio::task::spawn_blocking(move || {
            provision::safe_delete_container_files(
                Path::new(DEFAULT_CONTAINERS_ROOT),
                &cid,
                &img,
                &mnt,
                del_img,
            )
        })
        .await
        .map_err(|e| DaemonError::join("delete files join", e))??;

        let new_default_container = self
            .config_store
            .remove_container(&id, params.force)
            .await?;

        self.evict_slot(&id).await;
        if let Some(services) = self.service_supervisor.read().await.clone() {
            services.evict_container_slots(&id).await;
        }

        Ok(ContainerDeleteResult {
            container: id,
            image_deleted,
            cleanup,
            new_default_container,
        })
    }

    /// Promotes an existing container to `defaultContainer`, persists config, and broadcasts state.
    pub(crate) async fn set_default_container(
        &self,
        params: ContainerSetDefaultParams,
    ) -> DaemonResult<ContainerSetDefaultResult> {
        let id = params.id.trim().to_owned();
        validate_container_id(&id)?;
        let previous = self.config_store.set_default_container(&id).await?;
        let cur_phase = self.get_or_create_slot(&id).await.state.read().await.phase;
        self.emit_state_changed(&id, cur_phase, cur_phase).await;
        if let Some(prev_id) = previous.as_deref()
            && prev_id != id
        {
            let prev_phase = self
                .get_or_create_slot(prev_id)
                .await
                .state
                .read()
                .await
                .phase;
            self.emit_state_changed(prev_id, prev_phase, prev_phase)
                .await;
        }
        Ok(ContainerSetDefaultResult {
            default_container: id,
            previous_default_container: previous,
        })
    }

    /// Safely resizes a container's ext4 rootfs image (stopping if needed, then restoring state).
    pub(crate) async fn resize_container(
        self: &Arc<Self>,
        params: ContainerResizeParams,
    ) -> DaemonResult<ContainerResizeResult> {
        if params.size_mb < 64 || params.size_mb > MAX_CONTAINER_SIZE_MB {
            return Err(DaemonError::InvalidParams(format!(
                "sizeMb must be between 64 and {MAX_CONTAINER_SIZE_MB} MiB"
            )));
        }
        let (id, cfg, _) = self.resolve_container(params.id.as_deref()).await?;
        let new_size_bytes = params.size_mb.saturating_mul(1024 * 1024);

        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        self.ensure_no_temp_lease(&id).await?;

        let was_running = slot.state.read().await.phase.is_running();
        if was_running {
            let _ = self.stop_locked(&id, &cfg, &slot).await?;
        }

        let img = PathBuf::from(&cfg.image_path);
        let (previous_bytes, new_bytes) = tokio::task::spawn_blocking(move || {
            provision::resize::resize_ext4_image(&img, new_size_bytes)
        })
        .await
        .map_err(|e| DaemonError::join("resize join", e))??;

        let mut restarted = false;
        if was_running {
            self.start_locked(&id, &cfg, &slot, false).await?;
            restarted = true;
        }

        Ok(ContainerResizeResult {
            container: id,
            previous_bytes,
            new_bytes,
            restarted,
        })
    }

    /// Sparse-clones an offline container into a brand-new container entry.
    pub(crate) async fn clone_container(
        self: &Arc<Self>,
        params: ContainerCloneParams,
    ) -> DaemonResult<ContainerCloneResult> {
        validate_container_id(&params.target_id)?;
        let (src_id, src_cfg, _) = self.resolve_container(Some(&params.source_id)).await?;
        let snapshot = self.config_store.snapshot().await;
        let enable_ssh = src_cfg.ssh.enabled || params.ssh_port.is_some();
        let (ssh_port, _reservation) =
            self.reserve_provision(&params.target_id, &snapshot, params.ssh_port, enable_ssh)?;

        let src_slot = self.get_or_create_slot(&src_id).await;
        let _guard = src_slot.lifecycle_lock.lock().await;
        if src_slot.state.read().await.phase.is_running() {
            return Err(DaemonError::ContainerAlreadyRunning(format!(
                "stop running container '{src_id}' before cloning"
            )));
        }
        self.ensure_no_temp_lease(&src_id).await?;

        let mut target_cfg = ContainerConfig::default_for_id(&params.target_id);
        target_cfg.hostname = params
            .hostname
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map_or_else(|| format!("strata-{}", params.target_id), ToOwned::to_owned);
        target_cfg.ssh.enabled = enable_ssh;
        target_cfg.ssh.port = ssh_port;
        target_cfg.host_bridge = src_cfg.host_bridge;
        target_cfg.host_runtime = src_cfg.host_runtime;
        target_cfg.extra_char_devs = src_cfg.extra_char_devs.clone();
        target_cfg.sync_android_dns = src_cfg.sync_android_dns;
        target_cfg.dns_servers = src_cfg.dns_servers.clone();
        target_cfg.noatime = src_cfg.noatime;
        target_cfg.auto_start = false;
        strata_protocol::config::validate_container_config(&params.target_id, &target_cfg)?;

        let src_img = PathBuf::from(&src_cfg.image_path);
        let dst_img = PathBuf::from(&target_cfg.image_path);
        let dst_img_rollback = dst_img.clone();
        let (copied_bytes, virtual_bytes) = tokio::task::spawn_blocking(move || {
            provision::clone::sparse_clone_image(&src_img, &dst_img, None)
        })
        .await
        .map_err(|e| DaemonError::join("clone join", e))??;

        let image_path = target_cfg.image_path.clone();
        if let Err(err) = self
            .config_store
            .add_container(params.target_id.clone(), target_cfg, false)
            .await
        {
            let _ = tokio::task::spawn_blocking(move || {
                let _ = fs::remove_file(&dst_img_rollback);
                if let Some(parent) = dst_img_rollback.parent() {
                    let _ = fs::remove_dir(parent);
                }
            })
            .await;
            return Err(err);
        }

        Ok(ContainerCloneResult {
            source_id: src_id,
            target_id: params.target_id,
            image_path,
            copied_bytes,
            virtual_bytes,
        })
    }

    /// Removes matching SSH keys from `<mount_dir>/root/.ssh/authorized_keys`.
    pub(crate) async fn remove_ssh_key(
        &self,
        params: SshKeyRemoveParams,
    ) -> DaemonResult<SshKeyRemoveResult> {
        let (id, entry, _) = self.resolve_container(params.id.as_deref()).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        self.ensure_no_temp_lease(&id).await?;
        let mdir = self.module_dir.clone();
        let cid = id.clone();
        let needle = params.fingerprint_or_key;

        let (keys, removed) = tokio::task::spawn_blocking(move || {
            let mount_dir = PathBuf::from(&entry.mount_dir);
            let was_mounted = mount::is_ext4_mounted(&mount_dir).unwrap_or(false);
            if !was_mounted {
                storage::mount_rootfs(&entry, mdir.as_deref())?;
            }
            let res = sync::remove_ssh_keys(&mount_dir, &needle);
            if !was_mounted {
                let _ = storage::teardown_container(&cid, &entry, None, None);
            }
            res
        })
        .await
        .map_err(|e| DaemonError::join("ssh-key remove join", e))??;

        Ok(SshKeyRemoveResult {
            container: id,
            removed,
            keys,
        })
    }

    /// Reads recent lines from the daemon log file (`<logs_dir>/stratad.log`).
    pub(crate) async fn read_daemon_logs(
        &self,
        params: DaemonLogsParams,
    ) -> DaemonResult<DaemonLogsResult> {
        let log_path = self.logs_dir.join("stratad.log");
        let max_lines = params.lines.unwrap_or(100).clamp(1, 500);
        let path_for_read = log_path.clone();
        let lines =
            tokio::task::spawn_blocking(move || sync::read_log_tail(&path_for_read, max_lines))
                .await
                .map_err(|e| DaemonError::join("read daemon logs join", e))?;
        Ok(DaemonLogsResult {
            log_path: log_path.to_string_lossy().into_owned(),
            lines,
        })
    }
}
