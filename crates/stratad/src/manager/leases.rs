//! Interactive CLI session lease registry with automatic UDS-EOF cleanup for oneshot/debug mounts.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use strata_protocol::capabilities::CapProfile;
use strata_protocol::rpc::{
    AttachLease, SessionAcquireParams, SessionMode, SessionReleaseParams, error_codes,
};
use strata_runtime::storage;
use strata_sys::{cgroup, namespace};
use tokio::sync::Mutex;

use crate::manager::ContainerManager;

#[derive(Debug, Clone)]
struct ActiveLease {
    container_id: String,
    mode: SessionMode,
    owner_conn_id: u64,
}

/// Tracks active interactive CLI leases across containers and client UDS connections.
#[derive(Debug)]
pub(crate) struct LeaseRegistry {
    next_id: AtomicU64,
    active: Mutex<HashMap<String, ActiveLease>>,
}

impl LeaseRegistry {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            active: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn active_count_for(&self, container_id: &str) -> usize {
        self.active
            .lock()
            .await
            .values()
            .filter(|l| l.container_id == container_id)
            .count()
    }

    async fn insert(&self, container_id: String, mode: SessionMode, owner_conn_id: u64) -> String {
        let seq = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("lease-{seq}");
        self.active.lock().await.insert(
            id.clone(),
            ActiveLease {
                container_id,
                mode,
                owner_conn_id,
            },
        );
        id
    }

    async fn remove(&self, lease_id: &str) -> Option<ActiveLease> {
        self.active.lock().await.remove(lease_id)
    }

    async fn remove_all_for_conn(&self, conn_id: u64) -> Vec<ActiveLease> {
        let mut map = self.active.lock().await;
        let ids: Vec<String> = map
            .iter()
            .filter(|(_, v)| v.owner_conn_id == conn_id)
            .map(|(k, _)| k.clone())
            .collect();
        let mut removed = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(entry) = map.remove(&id) {
                removed.push(entry);
            }
        }
        removed
    }

    async fn has_temp_mount_lease(&self, container_id: &str) -> bool {
        self.active.lock().await.values().any(|l| {
            l.container_id == container_id
                && matches!(l.mode, SessionMode::Oneshot | SessionMode::Debug)
        })
    }
}

impl ContainerManager {
    /// Acquires an interactive attach/oneshot/debug lease for a single-threaded CLI session.
    pub(crate) async fn acquire_session(
        &self,
        conn_id: u64,
        params: SessionAcquireParams,
    ) -> Result<AttachLease, (i32, String)> {
        let (id, cfg, _) = self.resolve_container(params.container.as_deref()).await?;
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;

        let live = slot.state.read().await;
        let is_running = live.phase.is_running() && live.ns_pid.is_some_and(namespace::pid_exists);
        let ns_pid = live.ns_pid;
        let mnt_ns_inode = live.mnt_ns_inode;
        drop(live);

        if params.debug {
            if is_running {
                return Err((
                    error_codes::CONTAINER_ALREADY_RUNNING,
                    format!(
                        "stop running container '{id}' before entering full host-runtime debug mode"
                    ),
                ));
            }
            let mount_cfg = cfg.clone();
            let mdir = self.module_dir.clone();
            tokio::task::spawn_blocking(move || storage::mount_rootfs(&mount_cfg, mdir.as_deref()))
                .await
                .map_err(|e| {
                    (
                        error_codes::INTERNAL_ERROR,
                        format!("debug mount join: {e}"),
                    )
                })?
                .map_err(|e| (e.rpc_code(), e.to_string()))?;

            let lease_id = self
                .leases
                .insert(id.clone(), SessionMode::Debug, conn_id)
                .await;
            return Ok(build_lease(
                lease_id,
                &id,
                SessionMode::Debug,
                None,
                None,
                &cfg,
                true,
            ));
        }

        if is_running {
            let _ = self.sync_dns(Some(&id)).await;
            let lease_id = self
                .leases
                .insert(id.clone(), SessionMode::Attached, conn_id)
                .await;
            return Ok(build_lease(
                lease_id,
                &id,
                SessionMode::Attached,
                ns_pid,
                mnt_ns_inode,
                &cfg,
                cfg.host_runtime,
            ));
        }

        let mount_cfg = cfg.clone();
        let mdir = self.module_dir.clone();
        tokio::task::spawn_blocking(move || storage::mount_rootfs(&mount_cfg, mdir.as_deref()))
            .await
            .map_err(|e| {
                (
                    error_codes::INTERNAL_ERROR,
                    format!("oneshot mount join: {e}"),
                )
            })?
            .map_err(|e| (e.rpc_code(), e.to_string()))?;

        let lease_id = self
            .leases
            .insert(id.clone(), SessionMode::Oneshot, conn_id)
            .await;
        Ok(build_lease(
            lease_id,
            &id,
            SessionMode::Oneshot,
            None,
            None,
            &cfg,
            cfg.host_runtime,
        ))
    }

    /// Releases a single lease explicitly and tears down temporary oneshot/debug mounts if unused.
    pub(crate) async fn release_session(
        &self,
        params: SessionReleaseParams,
    ) -> Result<bool, (i32, String)> {
        let Some(lease) = self.leases.remove(&params.lease_id).await else {
            return Ok(false);
        };
        self.cleanup_temp_lease_if_idle(&lease).await;
        Ok(true)
    }

    /// Releases all leases owned by a disconnected UDS client, guaranteeing zero leaked mounts.
    pub(crate) async fn release_connection_leases(&self, conn_id: u64) {
        let removed = self.leases.remove_all_for_conn(conn_id).await;
        for lease in removed {
            self.cleanup_temp_lease_if_idle(&lease).await;
        }
    }

    async fn cleanup_temp_lease_if_idle(&self, lease: &ActiveLease) {
        if !matches!(lease.mode, SessionMode::Oneshot | SessionMode::Debug) {
            return;
        }
        if self.leases.has_temp_mount_lease(&lease.container_id).await {
            return;
        }
        let Ok((id, cfg, _)) = self.resolve_container(Some(&lease.container_id)).await else {
            return;
        };
        let slot = self.get_or_create_slot(&id).await;
        let _guard = slot.lifecycle_lock.lock().await;
        if slot.state.read().await.phase.is_running() {
            return;
        }
        let _ =
            tokio::task::spawn_blocking(move || storage::teardown_container(&id, &cfg, None, None))
                .await;
    }
}

fn build_lease(
    lease_id: String,
    id: &str,
    mode: SessionMode,
    ns_pid: Option<u32>,
    mnt_ns_inode: Option<u64>,
    cfg: &strata_protocol::ContainerConfig,
    debug_override: bool,
) -> AttachLease {
    let cap_profile = if debug_override {
        CapProfile::FullHostRuntime
    } else {
        cfg.cap_profile()
    };
    AttachLease {
        lease_id,
        container: id.to_owned(),
        mode,
        ns_pid,
        mnt_ns_inode,
        cgroup_v2_path: cgroup::container_cgroup_v2_path(id)
            .to_string_lossy()
            .into_owned(),
        mount_dir: cfg.mount_dir.clone(),
        hostname: cfg.hostname.clone(),
        cap_profile,
        host_runtime: debug_override || cfg.host_runtime,
        bind_data_local_tmp: cfg.bind_data_local_tmp,
        extra_char_devs: cfg.extra_char_devs.clone(),
        extra_mounts: cfg.extra_mounts.clone(),
    }
}
