//! Container maintenance operations: DNS sync, logs, FITRIM compact, fsck, backup, and SSH keys.

use std::path::PathBuf;
use std::sync::Arc;

use strata_protocol::EventEnvelope;
use strata_protocol::rpc::{
    BackupParams, BackupResult, BackupStartResult, CompactResult, ContainerLogsParams,
    ContainerLogsResult, FsckResult, MaintenanceDonePayload, SshKeyAddParams, SshKeysResult,
    events,
};
use strata_runtime::{storage, sync};
use strata_sys::mount;

use crate::error::{DaemonError, DaemonResult};
use crate::manager::ContainerManager;
use crate::sideveil_seam::PluginEventSink;

impl ContainerManager {
    /// Synchronizes `/etc/resolv.conf`, `/run/strata/host.env`, and bridge scripts for one container.
    pub(crate) async fn sync_dns(&self, requested: Option<&str>) -> DaemonResult<String> {
        let (id, entry, _) = self.resolve_container(requested).await?;
        let mount_dir = PathBuf::from(&entry.mount_dir);
        let mdir = self.module_dir.clone();
        let synced = tokio::task::spawn_blocking(move || {
            if !mount::is_ext4_mounted(&mount_dir).unwrap_or(false) {
                return Ok(false);
            }
            sync::sync_container_runtime_files(&mount_dir, &entry, mdir.as_deref())?;
            Ok::<bool, strata_runtime::RuntimeError>(true)
        })
        .await
        .map_err(|e| DaemonError::join("sync task failed", e))??;
        if !synced {
            return Err(DaemonError::ContainerNotRunning(format!(
                "container '{id}' rootfs is not mounted"
            )));
        }
        Ok(id)
    }

    /// Returns tailed Dropbear/init log lines for one container.
    pub(crate) async fn read_logs(
        &self,
        params: ContainerLogsParams,
    ) -> DaemonResult<ContainerLogsResult> {
        let (id, _, _) = self.resolve_container(params.container.as_deref()).await?;
        let log_path = self.log_file_path(&id);
        let max_lines = params.lines.unwrap_or(50).clamp(1, 500);
        let path_for_read = log_path.clone();
        let lines =
            tokio::task::spawn_blocking(move || sync::read_log_tail(&path_for_read, max_lines))
                .await
                .map_err(|e| DaemonError::join("read logs join", e))?;
        Ok(ContainerLogsResult {
            container: id,
            log_path: log_path.to_string_lossy().into_owned(),
            lines,
        })
    }

    async fn ensure_no_temp_lease(&self, id: &str) -> DaemonResult<()> {
        if self.leases.has_temp_mount_lease(id).await {
            return Err(DaemonError::TempLeaseActive(id.to_owned()));
        }
        Ok(())
    }

    /// Runs online or offline `FITRIM` compaction on a container's sparse ext4 image.
    pub(crate) async fn compact_container(
        &self,
        requested: Option<&str>,
    ) -> DaemonResult<CompactResult> {
        let (id, entry, _) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        self.ensure_no_temp_lease(&id).await?;
        let mdir = self.module_dir.clone();
        let cid = id.clone();
        let res = tokio::task::spawn_blocking(move || {
            storage::compact_image(&cid, &entry, mdir.as_deref())
        })
        .await
        .map_err(|e| DaemonError::join("compact join", e))??;
        Ok(res)
    }

    /// Stops container if running and runs `/system/bin/e2fsck -f -y` under continuous lifecycle lock.
    pub(crate) async fn fsck_container(
        self: &Arc<Self>,
        requested: Option<&str>,
    ) -> DaemonResult<FsckResult> {
        let (id, entry, _) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        self.ensure_no_temp_lease(&id).await?;
        let _ = self.stop_locked(&id, &entry, &slot).await?;
        let cid = id.clone();
        let res = tokio::task::spawn_blocking(move || storage::fsck_image(&cid, &entry))
            .await
            .map_err(|e| DaemonError::join("fsck join", e))??;
        Ok(res)
    }

    /// Stops container cleanly if running, writes atomic gzip backup, and restores prior run state
    /// under a continuous `lifecycle_lock`.
    pub(crate) async fn backup_container(
        self: &Arc<Self>,
        params: BackupParams,
    ) -> DaemonResult<BackupResult> {
        let (id, entry, _) = self.resolve_container(params.container.as_deref()).await?;
        if params.output_path.trim().is_empty() {
            return Err(DaemonError::InvalidParams(
                "outputPath must not be empty".to_owned(),
            ));
        }
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        self.ensure_no_temp_lease(&id).await?;
        let was_running = slot.state.read().await.phase.is_running();
        let _ = self.stop_locked(&id, &entry, &slot).await?;

        let src = PathBuf::from(&entry.image_path);
        let dst = PathBuf::from(&params.output_path);
        let bytes_written =
            tokio::task::spawn_blocking(move || storage::backup_image_gzip(&src, &dst))
                .await
                .map_err(|e| DaemonError::join("backup join", e))??;

        let mut restarted = false;
        if was_running && self.start_locked(&id, &entry, &slot, false).await.is_ok() {
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
    ) -> DaemonResult<BackupStartResult> {
        let (id, _, _) = self.resolve_container(params.container.as_deref()).await?;
        let trimmed = params.output_path.trim().to_owned();
        if trimmed.is_empty() {
            return Err(DaemonError::InvalidParams(
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
                Err(err) => MaintenanceDonePayload {
                    container: cid,
                    op: "backup".to_owned(),
                    ok: false,
                    summary: out_path,
                    error: Some(err.to_string()),
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
    ) -> DaemonResult<SshKeysResult> {
        let (id, entry, _) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        let mdir = self.module_dir.clone();
        let cid = id.clone();

        let res = tokio::task::spawn_blocking(move || {
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
        .map_err(|e| DaemonError::join("ssh-key join", e))??;
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_store::ConfigStore;
    use crate::sideveil_seam::EventHub;
    use strata_protocol::rpc::{SessionMode, error_codes};

    #[tokio::test]
    async fn temp_mount_leases_block_start_compact_fsck_and_backup() {
        let base = std::env::temp_dir().join(format!("strata-lease-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("create temp dir");
        let store = Arc::new(ConfigStore::load_or_seed(base.join("config.json")).expect("seed"));
        let manager = Arc::new(ContainerManager::new(
            store,
            Arc::new(EventHub::new()),
            PathBuf::from("/bin/true"),
            base.join("logs"),
            None,
        ));

        let attached = manager
            .leases
            .insert("main".into(), SessionMode::Attached, 1)
            .await;
        assert!(!manager.leases.has_temp_mount_lease("main").await);
        let oneshot = manager
            .leases
            .insert("main".into(), SessionMode::Oneshot, 2)
            .await;
        assert!(manager.leases.has_temp_mount_lease("main").await);

        for res in [
            manager.start_container(Some("main")).await.map(|_| ()),
            manager.compact_container(Some("main")).await.map(|_| ()),
            manager.fsck_container(Some("main")).await.map(|_| ()),
        ] {
            assert_eq!(
                res.expect_err("must reject active temp lease").code(),
                error_codes::IMAGE_BUSY
            );
        }

        let _ = manager.leases.remove(&oneshot).await;
        let _ = manager.leases.remove(&attached).await;
        assert!(!manager.leases.has_temp_mount_lease("main").await);
        let _ = std::fs::remove_dir_all(&base);
    }
}
