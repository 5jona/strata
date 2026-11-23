//! Multi-container lifecycle manager owning per-container serialization, supervision, and leases.

mod launch;
pub(crate) mod leases;
pub(crate) mod ops;
mod snapshot;
pub(crate) mod state;
pub(crate) mod supervisor;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use strata_protocol::rpc::StorageCleanupReport;
use strata_protocol::{ContainerConfig, ContainerStatusSnapshot};
use strata_runtime::sync;
use strata_sys::{mount, namespace};
use tokio::sync::RwLock;

use crate::config_store::ConfigStore;
use crate::error::{DaemonError, DaemonResult};
use crate::manager::leases::LeaseRegistry;
pub(crate) use crate::manager::state::ContainerSlot;
use crate::sideveil_seam::EventHub;

/// Central multi-container orchestrator.
#[derive(Debug)]
pub(crate) struct ContainerManager {
    pub(crate) config_store: Arc<ConfigStore>,
    pub(crate) event_hub: Arc<EventHub>,
    pub(crate) self_exe: PathBuf,
    pub(crate) logs_dir: PathBuf,
    pub(crate) module_dir: Option<PathBuf>,
    slots: RwLock<HashMap<String, Arc<ContainerSlot>>>,
    pub(crate) leases: LeaseRegistry,
}

impl ContainerManager {
    #[must_use]
    pub(crate) fn new(
        config_store: Arc<ConfigStore>,
        event_hub: Arc<EventHub>,
        self_exe: PathBuf,
        logs_dir: PathBuf,
        module_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            config_store,
            event_hub,
            self_exe,
            logs_dir,
            module_dir,
            slots: RwLock::new(HashMap::new()),
            leases: LeaseRegistry::new(),
        }
    }

    pub(crate) async fn get_or_create_slot(&self, id: &str) -> Arc<ContainerSlot> {
        if let Some(existing) = self.slots.read().await.get(id) {
            return Arc::clone(existing);
        }
        let mut map = self.slots.write().await;
        Arc::clone(
            map.entry(id.to_owned())
                .or_insert_with(|| Arc::new(ContainerSlot::new())),
        )
    }

    pub(crate) async fn resolve_container(
        &self,
        requested: Option<&str>,
    ) -> DaemonResult<(String, ContainerConfig, bool)> {
        let cfg = self.config_store.snapshot().await;
        let Some((id, entry)) = cfg.get_container(requested) else {
            let name = cfg.resolve_container_id(requested);
            return Err(DaemonError::ContainerNotFound(name.to_owned()));
        };
        Ok((id.to_owned(), entry.clone(), id == cfg.default_container))
    }

    /// Starts all enabled `autoStart` containers whose backing images exist on disk.
    pub(crate) async fn start_auto_containers(self: &Arc<Self>) {
        let cfg = self.config_store.snapshot().await;
        for (id, entry) in &cfg.containers {
            if !entry.enabled || !entry.auto_start {
                continue;
            }
            if !Path::new(&entry.image_path).is_file() {
                eprintln!(
                    "stratad: skipping auto-start for '{id}': image '{}' not found",
                    entry.image_path
                );
                continue;
            }
            if let Err(err) = self.start_container(Some(id.as_str())).await {
                eprintln!("stratad: auto-start for '{id}' failed: {err}");
            }
        }
    }

    /// Stops every tracked container cleanly during daemon shutdown.
    pub(crate) async fn stop_all(self: &Arc<Self>) {
        let ids: Vec<String> = self
            .config_store
            .snapshot()
            .await
            .containers
            .keys()
            .cloned()
            .collect();
        for id in ids {
            let _ = self.stop_container(Some(id.as_str())).await;
        }
    }

    /// Synchronizes DNS, `host.env`, and bridge scripts for all currently mounted containers.
    pub(crate) async fn sync_running_containers(&self) {
        let cfg = self.config_store.snapshot().await;
        for (id, entry) in &cfg.containers {
            let mount_dir = PathBuf::from(&entry.mount_dir);
            if mount::is_ext4_mounted(&mount_dir).unwrap_or(false) {
                let c = entry.clone();
                let mdir = self.module_dir.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    sync::sync_container_runtime_files(&mount_dir, &c, mdir.as_deref())
                })
                .await;
            } else {
                let _ = id;
            }
        }
    }

    /// Re-synchronizes `/etc/resolv.conf` across all running containers with `syncAndroidDns`.
    pub(crate) async fn sync_dns_all(&self) {
        let cfg = self.config_store.snapshot().await;
        for (id, entry) in &cfg.containers {
            if !entry.sync_android_dns {
                continue;
            }
            let is_running = {
                let slots = self.slots.read().await;
                match slots.get(id) {
                    Some(slot) => slot.state.read().await.phase.is_running(),
                    None => false,
                }
            };
            if !is_running {
                continue;
            }
            let mount_dir = PathBuf::from(&entry.mount_dir);
            let c = entry.clone();
            let cid = id.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if mount::is_ext4_mounted(&mount_dir).unwrap_or(false)
                    && let Err(err) = sync::sync_container_dns(&mount_dir, &c)
                {
                    eprintln!("stratad: DNS sync for '{cid}' failed: {err}");
                }
            })
            .await;
        }
    }

    /// Returns status snapshots for all configured containers.
    pub(crate) async fn list_containers(&self) -> Vec<ContainerStatusSnapshot> {
        let cfg = self.config_store.snapshot().await;
        let mut out = Vec::with_capacity(cfg.containers.len());
        for (id, entry) in &cfg.containers {
            let is_default = id == &cfg.default_container;
            out.push(self.build_snapshot(id, entry, is_default).await);
        }
        out
    }

    /// Returns a status snapshot for one container.
    pub(crate) async fn container_status(
        &self,
        requested: Option<&str>,
    ) -> DaemonResult<ContainerStatusSnapshot> {
        let (id, entry, is_default) = self.resolve_container(requested).await?;
        Ok(self.build_snapshot(&id, &entry, is_default).await)
    }

    /// Returns count of currently running containers for `daemon.status` and Native HUD summaries.
    pub(crate) async fn running_count(&self) -> usize {
        let slots: Vec<Arc<ContainerSlot>> = self.slots.read().await.values().cloned().collect();
        let mut count = 0;
        for slot in slots {
            if slot.state.read().await.phase.is_running() {
                count += 1;
            }
        }
        count
    }

    /// Returns `(container_id, ns_pid, cfg)` for a currently running container.
    pub(crate) async fn running_target(
        &self,
        requested: Option<&str>,
    ) -> DaemonResult<(String, u32, ContainerConfig)> {
        let (id, entry, _) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let live = slot.state.read().await;
        if live.phase.is_running()
            && let Some(pid) = live.ns_pid
            && namespace::pid_exists(pid)
        {
            return Ok((id, pid, entry));
        }
        Err(DaemonError::ContainerNotRunning(format!(
            "container '{id}' is not running"
        )))
    }

    pub(crate) fn log_file_path(&self, container_id: &str) -> PathBuf {
        self.logs_dir.join(format!("{container_id}-sshd.log"))
    }
}

/// Helper alias used by `supervisor` teardown return path.
pub(crate) type StopOutcome = DaemonResult<StorageCleanupReport>;
