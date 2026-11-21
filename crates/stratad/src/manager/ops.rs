//! Container maintenance operations: DNS sync, logs, FITRIM compact, fsck, backup, and SSH keys.

use std::path::PathBuf;
use std::sync::Arc;

use strata_protocol::EventEnvelope;
use strata_protocol::rpc::{
    BackupParams, BackupResult, BackupStartResult, CompactResult, ContainerLogsParams,
    ContainerLogsResult, FsckResult, MaintenanceDonePayload, SshKeyAddParams, SshKeysResult,
    error_codes, events,
};
use strata_runtime::{storage, sync};
use strata_sys::mount;

use crate::manager::ContainerManager;
use crate::sideveil_seam::PluginEventSink;

impl ContainerManager {
    /// Synchronizes `/etc/resolv.conf`, `/run/strata/host.env`, and bridge scripts for one container.
    pub(crate) async fn sync_dns(&self, requested: Option<&str>) -> Result<String, (i32, String)> {
        let (id, entry, _) = self.resolve_container(requested).await?;
        let mount_dir = PathBuf::from(&entry.mount_dir);
        let mounted = mount::is_ext4_mounted(&mount_dir).unwrap_or(false);
        if !mounted {
            return Err((
                error_codes::CONTAINER_NOT_RUNNING,
                format!("container '{id}' rootfs is not mounted"),
            ));
        }
        let mdir = self.module_dir.clone();
        tokio::task::spawn_blocking(move || {
            sync::sync_container_runtime_files(&mount_dir, &entry, mdir.as_deref())
        })
        .await
        .map_err(|e| {
            (
                error_codes::INTERNAL_ERROR,
                format!("sync task failed: {e}"),
            )
        })?
        .map_err(|e| (e.rpc_code(), e.to_string()))?;
        Ok(id)
    }

    /// Returns tailed Dropbear/init log lines for one container.
    pub(crate) async fn read_logs(
        &self,
        params: ContainerLogsParams,
    ) -> Result<ContainerLogsResult, (i32, String)> {
        let (id, _, _) = self.resolve_container(params.container.as_deref()).await?;
        let log_path = self.log_file_path(&id);
        let max_lines = params.lines.unwrap_or(50).clamp(1, 500);
        let lines = sync::read_log_tail(&log_path, max_lines);
        Ok(ContainerLogsResult {
            container: id,
            log_path: log_path.to_string_lossy().into_owned(),
            lines,
        })
    }

    /// Runs online or offline `FITRIM` compaction on a container's sparse ext4 image.
    pub(crate) async fn compact_container(
        &self,
        requested: Option<&str>,
    ) -> Result<CompactResult, (i32, String)> {
        let (id, entry, _) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        let mdir = self.module_dir.clone();
        let cid = id.clone();
        tokio::task::spawn_blocking(move || storage::compact_image(&cid, &entry, mdir.as_deref()))
            .await
            .map_err(|e| (error_codes::INTERNAL_ERROR, format!("compact join: {e}")))?
            .map_err(|e| (e.rpc_code(), e.to_string()))
    }

    /// Stops container if running and runs `/system/bin/e2fsck -f -y`.
    pub(crate) async fn fsck_container(
        self: &Arc<Self>,
        requested: Option<&str>,
    ) -> Result<FsckResult, (i32, String)> {
        let (id, entry, _) = self.resolve_container(requested).await?;
        let _ = self.stop_container(Some(&id)).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        let cid = id.clone();
        tokio::task::spawn_blocking(move || storage::fsck_image(&cid, &entry))
            .await
            .map_err(|e| (error_codes::INTERNAL_ERROR, format!("fsck join: {e}")))?
            .map_err(|e| (e.rpc_code(), e.to_string()))
    }

    /// Stops container cleanly if running, writes atomic gzip backup, and restores prior run state.
    pub(crate) async fn backup_container(
        self: &Arc<Self>,
        params: BackupParams,
    ) -> Result<BackupResult, (i32, String)> {
        let (id, entry, _) = self.resolve_container(params.container.as_deref()).await?;
        if params.output_path.trim().is_empty() {
            return Err((
                error_codes::INVALID_PARAMS,
                "outputPath must not be empty".to_owned(),
            ));
        }
        let was_running = self
            .get_or_create_slot(&id)
            .await
            .state
            .read()
            .await
            .phase
            .is_running();
        let _ = self.stop_container(Some(&id)).await?;

        let src = PathBuf::from(&entry.image_path);
        let dst = PathBuf::from(&params.output_path);
        let bytes_written = {
            let slot = self.get_or_create_slot(&id).await;
            let _guard = slot.lifecycle_lock.lock().await;
            tokio::task::spawn_blocking(move || storage::backup_image_gzip(&src, &dst))
                .await
                .map_err(|e| (error_codes::INTERNAL_ERROR, format!("backup join: {e}")))?
                .map_err(|e| (e.rpc_code(), e.to_string()))?
        };

        let mut restarted = false;
        if was_running && self.start_container(Some(&id)).await.is_ok() {
            restarted = true;
        }
        Ok(BackupResult {
            container: id,
            output_path: params.output_path,
            bytes_written,
            restarted,
        })
    }

    /// Launches a background gzip backup and emits `container.maintenanceDone` upon completion.
    /// Avoids blocking SideVeil `plugin.invoke` beyond its 8-second RPC timeout.
    pub(crate) async fn backup_container_async(
        self: &Arc<Self>,
        params: BackupParams,
    ) -> Result<BackupStartResult, (i32, String)> {
        let (id, _, _) = self.resolve_container(params.container.as_deref()).await?;
        let trimmed = params.output_path.trim().to_owned();
        if trimmed.is_empty() {
            return Err((
                error_codes::INVALID_PARAMS,
                "outputPath must not be empty".to_owned(),
            ));
        }
        let mgr = Arc::clone(self);
        let cid = id.clone();
        let out_path = trimmed.clone();
        tokio::spawn(async move {
            let outcome = mgr
                .backup_container(BackupParams {
                    container: Some(cid.clone()),
                    output_path: out_path.clone(),
                })
                .await;
            let payload = match outcome {
                Ok(res) => MaintenanceDonePayload {
                    container: cid,
                    op: "backup".to_owned(),
                    ok: true,
                    summary: format!(
                        "{} ({:.1} MiB)",
                        res.output_path,
                        res.bytes_written as f64 / (1024.0 * 1024.0)
                    ),
                    error: None,
                },
                Err((_, err_msg)) => MaintenanceDonePayload {
                    container: cid,
                    op: "backup".to_owned(),
                    ok: false,
                    summary: out_path,
                    error: Some(err_msg),
                },
            };
            mgr.event_hub.emit(EventEnvelope::new(
                events::CONTAINER_MAINTENANCE_DONE,
                serde_json::to_value(payload).ok(),
            ));
        });

        Ok(BackupStartResult {
            started: true,
            container: id,
            output_path: trimmed,
        })
    }

    /// Lists or appends authorized SSH public keys (temporarily mounting stopped rootfs if needed).
    pub(crate) async fn ssh_keys(
        &self,
        requested: Option<&str>,
        add_params: Option<SshKeyAddParams>,
    ) -> Result<SshKeysResult, (i32, String)> {
        let (id, entry, _) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        let mdir = self.module_dir.clone();
        let cid = id.clone();

        tokio::task::spawn_blocking(move || {
            let mount_dir = PathBuf::from(&entry.mount_dir);
            let was_mounted = mount::is_ext4_mounted(&mount_dir).unwrap_or(false);
            if !was_mounted {
                storage::mount_rootfs(&entry, mdir.as_deref())?;
            }
            let op_res = match add_params {
                Some(p) => sync::add_ssh_keys(&mount_dir, &p.public_key),
                None => sync::list_ssh_keys(&mount_dir).map(|keys| (keys, 0)),
            };
            if !was_mounted {
                let _ = storage::teardown_container(&cid, &entry, None, None);
            }
            op_res.map(|(keys, added)| SshKeysResult {
                container: cid,
                keys,
                added,
            })
        })
        .await
        .map_err(|e| (error_codes::INTERNAL_ERROR, format!("ssh-key join: {e}")))?
        .map_err(|e| (e.rpc_code(), e.to_string()))
    }
}
