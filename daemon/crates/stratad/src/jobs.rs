//! Asynchronous container creation and streaming import job coordinator.

pub(crate) mod create;
mod raw_image;
mod tracker;

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use futures_util::TryStreamExt;
use strata_protocol::constants::DEFAULT_CONTAINERS_ROOT;
use strata_protocol::rpc::{
    ContainerCreateParams, ContainerJobCancelResult, ContainerJobKind, ContainerJobProgressPayload,
    ContainerJobStartResult,
};
use strata_protocol::{ContainerConfig, validate_container_id};
use strata_runtime::provision::archive::{ArchiveFormat, detect_archive_format};
use strata_runtime::storage;
use tokio_util::io::{StreamReader, SyncIoBridge};

use crate::catalog::{CatalogClient, validate_remote_https_url};
use crate::config_store::ConfigStore;
use crate::error::{DaemonError, DaemonResult};
use crate::manager::{ContainerManager, ProvisionReservation};
use crate::sideveil_seam::EventHub;
use crate::wakelock::WakeLockManager;
use create::{MAX_CONTAINER_SIZE_MB, provision_from_reader};
use tracker::JobTracker;

/// Coordinates async `container.create` and streaming HTTP image imports with progress events.
#[derive(Debug)]
pub(crate) struct ProvisionJobManager {
    config_store: Arc<ConfigStore>,
    manager: Arc<ContainerManager>,
    catalog: Arc<CatalogClient>,
    wakelock: Arc<WakeLockManager>,
    containers_root: PathBuf,
    tracker: JobTracker,
}

impl ProvisionJobManager {
    #[must_use]
    pub(crate) fn new(
        config_store: Arc<ConfigStore>,
        manager: Arc<ContainerManager>,
        catalog: Arc<CatalogClient>,
        event_hub: Arc<EventHub>,
        wakelock: Arc<WakeLockManager>,
    ) -> Self {
        Self {
            config_store,
            manager,
            catalog,
            wakelock,
            containers_root: PathBuf::from(DEFAULT_CONTAINERS_ROOT),
            tracker: JobTracker::new(event_hub),
        }
    }

    pub(crate) async fn active_count(&self) -> usize {
        self.tracker.active_count().await
    }

    pub(crate) async fn job_status(
        &self,
        job_id: &str,
    ) -> DaemonResult<ContainerJobProgressPayload> {
        self.tracker.status(job_id).await
    }

    pub(crate) async fn job_cancel(&self, job_id: &str) -> DaemonResult<ContainerJobCancelResult> {
        self.tracker.cancel(job_id).await
    }

    /// Validates parameters, reserves `(id, ssh_port)`, and launches an async online creation job.
    pub(crate) async fn start_create_job(
        self: &Arc<Self>,
        params: ContainerCreateParams,
    ) -> DaemonResult<ContainerJobStartResult> {
        if let Some(url) = &params.source_url {
            let _ = validate_remote_https_url(url)?;
        }
        let (container_id, cfg, reservation) = self.prepare_container_spec(&params).await?;
        let (job_id, cancel) = self
            .tracker
            .register(&container_id, ContainerJobKind::Create, "resolving")
            .await;

        let this = Arc::clone(self);
        let jid = job_id.clone();
        let cid = container_id.clone();
        tokio::spawn(async move {
            let _reservation = reservation;
            let _wl = this.wakelock.acquire();
            let outcome = this
                .run_online_create(&jid, &cid, cfg, params, Arc::clone(&cancel))
                .await;
            this.tracker.finalize(&jid, outcome, &cancel).await;
        });

        Ok(ContainerJobStartResult {
            job_id,
            container: container_id,
            kind: ContainerJobKind::Create,
        })
    }

    /// Streams an uploaded HTTP body directly into a new container rootfs (zero double disk copy).
    pub(crate) async fn run_streaming_import<S, E>(
        self: &Arc<Self>,
        params: ContainerCreateParams,
        filename: String,
        content_len: Option<u64>,
        stream: S,
    ) -> DaemonResult<ContainerJobProgressPayload>
    where
        S: futures_util::Stream<Item = Result<bytes::Bytes, E>> + Send + Unpin + 'static,
        E: std::fmt::Display + Send + Sync + 'static,
    {
        let format = detect_archive_format(&filename, &[]).ok_or_else(|| {
            DaemonError::InvalidParams(format!(
                "unsupported archive/image filename '{filename}' (expected .tar.gz|.tar.xz|.tar.zst|.tar|.img|.img.gz)"
            ))
        })?;
        let (container_id, cfg, _reservation) = self.prepare_container_spec(&params).await?;
        let (job_id, cancel) = self
            .tracker
            .register(&container_id, ContainerJobKind::Import, "importing")
            .await;
        let _wl = self.wakelock.acquire();

        let io_stream = stream.map_err(|e| std::io::Error::other(e.to_string()));
        let sync_reader = SyncIoBridge::new(StreamReader::new(io_stream));

        let outcome = self
            .execute_reader_provision(
                &job_id,
                &container_id,
                cfg,
                params,
                sync_reader,
                format,
                content_len,
                Arc::clone(&cancel),
            )
            .await;
        self.tracker.finalize(&job_id, outcome, &cancel).await;
        self.job_status(&job_id).await
    }

    async fn prepare_container_spec(
        &self,
        params: &ContainerCreateParams,
    ) -> DaemonResult<(String, ContainerConfig, ProvisionReservation)> {
        validate_container_id(&params.id)?;
        if params.size_mb < 64 || params.size_mb > MAX_CONTAINER_SIZE_MB {
            return Err(DaemonError::InvalidParams(format!(
                "sizeMb must be between 64 and {MAX_CONTAINER_SIZE_MB} MiB"
            )));
        }

        let snapshot = self.config_store.snapshot().await;
        let (ssh_port, reservation) = self.manager.reserve_provision(
            &params.id,
            &snapshot,
            params.ssh_port,
            params.enable_ssh,
        )?;

        let mut cfg = ContainerConfig::default_for_id(&params.id);
        let base_dir = self.containers_root.join(&params.id);
        cfg.image_path = base_dir.join("rootfs.img").to_string_lossy().into_owned();
        cfg.mount_dir = base_dir.join("mnt").to_string_lossy().into_owned();
        if let Some(h) = &params.hostname
            && !h.trim().is_empty()
        {
            cfg.hostname = h.trim().to_owned();
        }
        cfg.ssh.enabled = params.enable_ssh;
        cfg.ssh.port = if params.enable_ssh {
            ssh_port
        } else {
            params.ssh_port.unwrap_or(0)
        };
        cfg.auto_start = params.auto_start;
        cfg.host_bridge = params.host_bridge;
        cfg.sync_android_dns = params.sync_android_dns;
        cfg.noatime = params.noatime;
        cfg.extra_char_devs = match &params.extra_char_devs {
            Some(devs) => devs.clone(),
            None if params.host_bridge => cfg.extra_char_devs,
            None => Vec::new(),
        };
        if params.l5_diagnostic_mounts {
            cfg.extra_mounts = strata_protocol::config::l5_diag_mounts();
        }
        strata_protocol::config::validate_container_config(&params.id, &cfg)?;
        Ok((params.id.clone(), cfg, reservation))
    }

    async fn run_online_create(
        self: &Arc<Self>,
        job_id: &str,
        container_id: &str,
        cfg: ContainerConfig,
        params: ContainerCreateParams,
        cancel: Arc<AtomicBool>,
    ) -> DaemonResult<()> {
        let (download_url, expected_sha, expected_size) = if let Some(url) = &params.source_url {
            let valid = validate_remote_https_url(url)?;
            (valid.to_string(), params.expected_sha256.clone(), None)
        } else {
            let distro_id = params.distro_id.as_deref().ok_or_else(|| {
                DaemonError::InvalidParams("either distroId or sourceUrl is required".to_owned())
            })?;
            let entry = self
                .catalog
                .resolve_distro(distro_id, params.mirror, params.custom_url.clone())
                .await?;
            let valid = validate_remote_https_url(&entry.download_url)?;
            let sha = params
                .expected_sha256
                .clone()
                .or_else(|| (!entry.sha256.is_empty()).then_some(entry.sha256));
            (
                valid.to_string(),
                sha,
                (entry.size_bytes > 0).then_some(entry.size_bytes),
            )
        };

        let format = detect_archive_format(&download_url, &[]).unwrap_or(ArchiveFormat::TarXz);
        self.tracker
            .update(
                job_id,
                "downloading",
                0,
                expected_size,
                Some(download_url.clone()),
            )
            .await;

        let resp = self
            .catalog
            .http()
            .get(&download_url)
            .send()
            .await
            .map_err(|e| DaemonError::Internal(format!("download '{download_url}': {e}")))?;
        if !resp.status().is_success() {
            return Err(DaemonError::Internal(format!(
                "download '{download_url}' failed with HTTP {}",
                resp.status()
            )));
        }
        let total = resp.content_length().or(expected_size);
        let io_stream = resp
            .bytes_stream()
            .map_err(|e| std::io::Error::other(e.to_string()));
        let sync_reader = SyncIoBridge::new(StreamReader::new(io_stream));

        let mut merged_params = params;
        merged_params.expected_sha256 = expected_sha;
        self.execute_reader_provision(
            job_id,
            container_id,
            cfg,
            merged_params,
            sync_reader,
            format,
            total,
            cancel,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_reader_provision<R: Read + Send + 'static>(
        self: &Arc<Self>,
        job_id: &str,
        container_id: &str,
        cfg: ContainerConfig,
        params: ContainerCreateParams,
        reader: R,
        format: ArchiveFormat,
        bytes_total: Option<u64>,
        cancel: Arc<AtomicBool>,
    ) -> DaemonResult<()> {
        let size_bytes = params.size_mb.saturating_mul(1024 * 1024);
        let cfg_for_block = cfg.clone();
        let cid_owned = container_id.to_owned();
        let mdir = self.manager.module_dir.clone();
        let ssh_keys = params.ssh_public_keys.clone();
        let expected_sha = params.expected_sha256.clone();

        let this = Arc::clone(self);
        let jid = job_id.to_owned();
        let rt_handle = tokio::runtime::Handle::current();

        tokio::task::spawn_blocking(move || {
            provision_from_reader(
                reader,
                format,
                expected_sha.as_deref(),
                &cid_owned,
                &cfg_for_block,
                size_bytes,
                &ssh_keys,
                mdir.as_deref(),
                cancel,
                move |stage, bytes_done| {
                    let this = Arc::clone(&this);
                    let jid = jid.clone();
                    let stage = stage.to_owned();
                    rt_handle.spawn(async move {
                        this.tracker
                            .update(&jid, &stage, bytes_done, bytes_total, None)
                            .await;
                    });
                },
            )
        })
        .await
        .map_err(|e| DaemonError::join("provision join", e))??;

        let rollback_cfg = cfg.clone();
        if let Err(err) = self
            .config_store
            .add_container(container_id.to_owned(), cfg, params.set_default)
            .await
        {
            let cid = container_id.to_owned();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = storage::teardown_container(&cid, &rollback_cfg, None, None);
                let _ = fs::remove_file(&rollback_cfg.image_path);
                let _ = fs::remove_dir(&rollback_cfg.mount_dir);
                if let Some(parent) = std::path::Path::new(&rollback_cfg.image_path).parent() {
                    let _ = fs::remove_dir(parent);
                }
            })
            .await;
            return Err(err);
        }

        if params.auto_start {
            let _ = self.manager.start_container(Some(container_id)).await;
        }
        Ok(())
    }
}
