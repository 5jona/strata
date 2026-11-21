//! Multi-container lifecycle manager owning per-container serialization, supervision, and leases.

pub(crate) mod leases;
pub(crate) mod ops;
pub(crate) mod supervisor;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use strata_protocol::rpc::{
    ContainerStateChangedPayload, StorageCleanupReport, error_codes, events,
};
use strata_protocol::{
    ContainerConfig, ContainerDiskStats, ContainerPhase, ContainerStatusSnapshot, EventEnvelope,
};
use strata_runtime::sync;
use strata_sys::{cgroup, loopdev, mount, namespace, net, procfs};
use tokio::sync::{Mutex, RwLock};

use crate::config_store::ConfigStore;
use crate::manager::leases::LeaseRegistry;
use crate::sideveil_seam::{EventHub, PluginEventSink};

/// Per-container live state tracked inside `stratad`.
#[derive(Debug, Default)]
pub(crate) struct ContainerLiveState {
    pub(crate) phase: ContainerPhase,
    pub(crate) ns_pid: Option<u32>,
    pub(crate) mnt_ns_inode: Option<u64>,
    pub(crate) started_at: Option<Instant>,
    pub(crate) restart_count: u32,
    pub(crate) last_error: Option<String>,
    pub(crate) desired_running: bool,
    pub(crate) generation: u64,
}

#[derive(Debug)]
pub(crate) struct ContainerSlot {
    pub(crate) lifecycle_lock: Mutex<()>,
    pub(crate) state: RwLock<ContainerLiveState>,
}

impl ContainerSlot {
    fn new() -> Self {
        Self {
            lifecycle_lock: Mutex::new(()),
            state: RwLock::new(ContainerLiveState::default()),
        }
    }
}

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
    ) -> Result<(String, ContainerConfig, bool), (i32, String)> {
        let cfg = self.config_store.snapshot().await;
        let Some((id, entry)) = cfg.get_container(requested) else {
            let name = cfg.resolve_container_id(requested);
            return Err((
                error_codes::CONTAINER_NOT_FOUND,
                format!("container '{name}' not found"),
            ));
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
            if let Err((_, msg)) = self.start_container(Some(id.as_str())).await {
                eprintln!("stratad: auto-start for '{id}' failed: {msg}");
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
    ) -> Result<ContainerStatusSnapshot, (i32, String)> {
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
    ) -> Result<(String, u32, ContainerConfig), (i32, String)> {
        let (id, entry, _) = self.resolve_container(requested).await?;
        let slot = self.get_or_create_slot(&id).await;
        let live = slot.state.read().await;
        if live.phase.is_running()
            && let Some(pid) = live.ns_pid
            && namespace::pid_exists(pid)
        {
            return Ok((id, pid, entry));
        }
        Err((
            error_codes::CONTAINER_NOT_RUNNING,
            format!("container '{id}' is not running"),
        ))
    }

    pub(crate) fn log_file_path(&self, container_id: &str) -> PathBuf {
        self.logs_dir.join(format!("{container_id}-sshd.log"))
    }

    pub(crate) async fn emit_state_changed(
        &self,
        id: &str,
        previous: ContainerPhase,
        current: ContainerPhase,
    ) {
        if previous == current {
            return;
        }
        if let Ok((cid, entry, is_default)) = self.resolve_container(Some(id)).await {
            let snapshot = self.build_snapshot(&cid, &entry, is_default).await;
            let payload = serde_json::to_value(ContainerStateChangedPayload {
                container: cid,
                previous,
                current,
                snapshot,
            })
            .ok();
            self.event_hub
                .emit(EventEnvelope::new(events::CONTAINER_STATE_CHANGED, payload));
        }
    }

    pub(crate) async fn build_snapshot(
        &self,
        id: &str,
        cfg: &ContainerConfig,
        is_default: bool,
    ) -> ContainerStatusSnapshot {
        let slot = self.get_or_create_slot(id).await;
        let live = slot.state.read().await;
        let mount_dir = Path::new(&cfg.mount_dir);
        let image_path = Path::new(&cfg.image_path);

        let mounted = mount::is_ext4_mounted(mount_dir).unwrap_or(false);
        let loop_devices = loopdev::find_image_loops(image_path)
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        let disk = collect_disk_stats(image_path, mount_dir, mounted);
        let cgroup_path = {
            let p = cgroup::container_cgroup_v2_path(id);
            p.exists().then(|| p.to_string_lossy().into_owned())
        };

        ContainerStatusSnapshot {
            id: id.to_owned(),
            is_default,
            enabled: cfg.enabled,
            auto_start: cfg.auto_start,
            phase: live.phase,
            ns_pid: live.ns_pid,
            mnt_ns_inode: live.mnt_ns_inode,
            cgroup_path,
            mounted,
            mount_dir: cfg.mount_dir.clone(),
            image_path: cfg.image_path.clone(),
            loop_devices,
            hostname: cfg.hostname.clone(),
            ssh_enabled: cfg.ssh.enabled,
            ssh_listen_addr: cfg.ssh.listen_addr.clone(),
            ssh_port: cfg.ssh.port,
            primary_ipv4: net::detect_primary_ipv4(),
            host_bridge: cfg.host_bridge,
            host_runtime: cfg.host_runtime,
            uptime_ms: live
                .started_at
                .map_or(0, |t| t.elapsed().as_millis() as u64),
            restart_count: live.restart_count,
            active_leases: self.leases.active_count_for(id).await,
            last_error: live.last_error.clone(),
            disk,
        }
    }
}

/// Helper alias used by `supervisor` teardown return path.
pub(crate) type StopOutcome = Result<StorageCleanupReport, (i32, String)>;

fn collect_disk_stats(image_path: &Path, mount_dir: &Path, mounted: bool) -> ContainerDiskStats {
    let image_exists = image_path.is_file();
    let (image_allocated_bytes, image_virtual_bytes) = if image_exists {
        procfs::image_disk_usage(image_path).unwrap_or((0, 0))
    } else {
        (0, 0)
    };
    let (rootfs_total_bytes, rootfs_used_bytes, rootfs_available_bytes) = if mounted {
        match procfs::statvfs_bytes(mount_dir) {
            Ok((total, used, avail)) => (Some(total), Some(used), Some(avail)),
            Err(_) => (None, None, None),
        }
    } else {
        (None, None, None)
    };

    ContainerDiskStats {
        image_exists,
        image_allocated_bytes,
        image_virtual_bytes,
        rootfs_total_bytes,
        rootfs_used_bytes,
        rootfs_available_bytes,
    }
}
