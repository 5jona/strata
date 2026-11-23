//! Container status snapshot assembly and disk/rootfs usage collection.

use std::path::{Path, PathBuf};

use strata_protocol::rpc::{ContainerStateChangedPayload, events};
use strata_protocol::{
    ContainerConfig, ContainerDiskStats, ContainerPhase, ContainerStatusSnapshot, EventEnvelope,
};
use strata_sys::{cgroup, loopdev, mount, net, procfs};

use crate::manager::ContainerManager;
use crate::sideveil_seam::PluginEventSink;

struct HostProbe {
    mounted: bool,
    loop_devices: Vec<String>,
    disk: ContainerDiskStats,
    cgroup_path: Option<String>,
    primary_ipv4: String,
}

impl HostProbe {
    fn collect(id: &str, image_path: &Path, mount_dir: &Path) -> Self {
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
        let primary_ipv4 = net::detect_primary_ipv4();
        Self {
            mounted,
            loop_devices,
            disk,
            cgroup_path,
            primary_ipv4,
        }
    }

    fn fallback() -> Self {
        Self {
            mounted: false,
            loop_devices: Vec::new(),
            disk: ContainerDiskStats {
                image_exists: false,
                image_allocated_bytes: 0,
                image_virtual_bytes: 0,
                rootfs_total_bytes: None,
                rootfs_used_bytes: None,
                rootfs_available_bytes: None,
            },
            cgroup_path: None,
            primary_ipv4: "127.0.0.1".to_owned(),
        }
    }
}

impl ContainerManager {
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
        let (phase, ns_pid, mnt_ns_inode, uptime_ms, restart_count, last_error) = {
            let live = slot.state.read().await;
            (
                live.phase,
                live.ns_pid,
                live.mnt_ns_inode,
                live.started_at
                    .map_or(0, |t| t.elapsed().as_millis() as u64),
                live.restart_count,
                live.last_error.clone(),
            )
        };
        let active_leases = self.leases.active_count_for(id).await;

        let cid = id.to_owned();
        let image_path = PathBuf::from(&cfg.image_path);
        let mount_dir = PathBuf::from(&cfg.mount_dir);
        let probe =
            tokio::task::spawn_blocking(move || HostProbe::collect(&cid, &image_path, &mount_dir))
                .await
                .unwrap_or_else(|_| HostProbe::fallback());

        ContainerStatusSnapshot {
            id: id.to_owned(),
            is_default,
            enabled: cfg.enabled,
            auto_start: cfg.auto_start,
            phase,
            ns_pid,
            mnt_ns_inode,
            cgroup_path: probe.cgroup_path,
            mounted: probe.mounted,
            mount_dir: cfg.mount_dir.clone(),
            image_path: cfg.image_path.clone(),
            loop_devices: probe.loop_devices,
            hostname: cfg.hostname.clone(),
            ssh_enabled: cfg.ssh.enabled,
            ssh_listen_addr: cfg.ssh.listen_addr.clone(),
            ssh_port: cfg.ssh.port,
            primary_ipv4: probe.primary_ipv4,
            host_bridge: cfg.host_bridge,
            host_runtime: cfg.host_runtime,
            uptime_ms,
            restart_count,
            active_leases,
            last_error,
            disk: probe.disk,
        }
    }
}

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
