//! Optional zero-WebView SideVeil `NativeHudScene` builder for Strata runtime summaries.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};
use strata_protocol::ContainerStatusSnapshot;
use strata_protocol::rpc::{HudSetParams, HudStateResult};
use tokio::sync::Notify;

/// Atomic preferences controlling Strata's optional SideVeil Native HUD summary strip.
#[derive(Debug)]
pub(crate) struct HudPreferences {
    enabled: AtomicBool,
    click_through: AtomicBool,
    is_dark: AtomicBool,
    changed: Notify,
}

impl HudPreferences {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            click_through: AtomicBool::new(false),
            is_dark: AtomicBool::new(true),
            changed: Notify::new(),
        }
    }

    #[must_use]
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    #[must_use]
    pub(crate) fn is_click_through(&self) -> bool {
        self.click_through.load(Ordering::Relaxed)
    }

    #[must_use]
    pub(crate) fn is_dark(&self) -> bool {
        self.is_dark.load(Ordering::Relaxed)
    }

    #[must_use]
    pub(crate) fn snapshot(&self, sideveil_connected: bool) -> HudStateResult {
        HudStateResult {
            enabled: self.is_enabled(),
            click_through: self.is_click_through(),
            is_dark: self.is_dark(),
            sideveil_connected,
        }
    }

    pub(crate) fn apply(&self, params: HudSetParams, sideveil_connected: bool) -> HudStateResult {
        if let Some(flag) = params.enabled {
            self.enabled.store(flag, Ordering::Relaxed);
        }
        if let Some(flag) = params.click_through {
            self.click_through.store(flag, Ordering::Relaxed);
        }
        if let Some(flag) = params.is_dark {
            self.is_dark.store(flag, Ordering::Relaxed);
        }
        self.changed.notify_one();
        self.snapshot(sideveil_connected)
    }

    pub(crate) async fn wait_change(&self) {
        self.changed.notified().await;
    }
}

/// Builds a SideVeil-compatible `NativeHudScene` JSON payload summarizing Strata container health.
#[must_use]
pub(crate) fn build_strata_hud_scene(
    plugin_id: &str,
    containers: &[ContainerStatusSnapshot],
    active_tasks: usize,
    click_through: bool,
    is_dark: bool,
) -> Value {
    let total = containers.len();
    let running = containers.iter().filter(|c| c.phase.is_running()).count();
    let primary = containers
        .iter()
        .find(|c| c.is_default)
        .or_else(|| containers.first());

    let (ssh_port, disk_ratio, disk_label) = match primary {
        Some(c) => {
            let ratio = match (c.disk.rootfs_used_bytes, c.disk.rootfs_total_bytes) {
                (Some(used), Some(cap)) if cap > 0 => (used as f32 / cap as f32).clamp(0.0, 1.0),
                _ if c.disk.image_virtual_bytes > 0 => (c.disk.image_allocated_bytes as f32
                    / c.disk.image_virtual_bytes as f32)
                    .clamp(0.0, 1.0),
                _ => 0.0,
            };
            let used_mib = c
                .disk
                .rootfs_used_bytes
                .unwrap_or(c.disk.image_allocated_bytes) as f64
                / (1024.0 * 1024.0);
            let label = if used_mib >= 1024.0 {
                format!("{:.1}G", used_mib / 1024.0)
            } else {
                format!("{used_mib:.0}M")
            };
            (c.ssh_port, ratio, label)
        }
        None => (2222, 0.0, "--".to_owned()),
    };

    let (primary_text, secondary_text, track_argb, bar_start, bar_end, dot_argb) = if is_dark {
        (
            0xFFF1_F5F9_u32,
            0xFFCB_D5E1_u32,
            0x33FF_FFFF_u32,
            0xFF34_D399_u32,
            0xFF38_BDF8_u32,
            if running > 0 {
                0xFF34_D399_u32
            } else {
                0xFF94_A3B8_u32
            },
        )
    } else {
        (
            0xFF0F_172A_u32,
            0xFF47_5569_u32,
            0x280F_172A_u32,
            0xFF05_9669_u32,
            0xFF02_84C7_u32,
            if running > 0 {
                0xFF05_9669_u32
            } else {
                0xFF64_748B_u32
            },
        )
    };

    let title = if running > 0 {
        format!("Strata  {running}/{total} 运行")
    } else {
        format!("Strata  0/{total} 待机")
    };
    let subtitle = format!("SSH :{ssh_port}  任务 {active_tasks}  占用 {disk_label}");

    json!({
        "pluginId": plugin_id,
        "widthDp": 212.0,
        "heightDp": 72.0,
        "cornerRadiusDp": 16.0,
        "backdrop": "native-glass",
        "clickThrough": click_through,
        "anchorXPercent": 4.0,
        "anchorYPercent": 12.0,
        "primitives": [
            {
                "kind": "roundedPill",
                "x": 12.0,
                "y": 14.0,
                "w": 8.0,
                "h": 8.0,
                "radius": 4.0,
                "fillArgb": dot_argb
            },
            {
                "kind": "progressBar",
                "x": 12.0,
                "y": 54.0,
                "w": 188.0,
                "h": 6.0,
                "radius": 3.0,
                "value": disk_ratio,
                "trackArgb": track_argb,
                "fillStartArgb": bar_start,
                "fillEndArgb": bar_end
            }
        ],
        "texts": [
            {
                "x": 26.0,
                "y": 22.0,
                "text": title,
                "sizeSp": 12.0,
                "colorArgb": primary_text,
                "align": "start",
                "bold": true,
                "tabularNums": true,
                "maxWidthDp": 176.0
            },
            {
                "x": 12.0,
                "y": 42.0,
                "text": subtitle,
                "sizeSp": 10.5,
                "colorArgb": secondary_text,
                "align": "start",
                "bold": false,
                "tabularNums": true,
                "maxWidthDp": 188.0
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use strata_protocol::{ContainerDiskStats, ContainerPhase};

    use super::*;

    fn sample_snapshot(phase: ContainerPhase) -> ContainerStatusSnapshot {
        ContainerStatusSnapshot {
            id: "main".to_owned(),
            is_default: true,
            enabled: true,
            auto_start: true,
            phase,
            ns_pid: Some(4242),
            mnt_ns_inode: None,
            cgroup_path: Some("/sys/fs/cgroup/strata/main".to_owned()),
            mounted: true,
            mount_dir: "/data/local/strata/containers/main/mnt".to_owned(),
            image_path: "/data/local/strata/containers/main/rootfs.img".to_owned(),
            loop_devices: vec!["/dev/block/loop9".to_owned()],
            hostname: "strata-main".to_owned(),
            ssh_enabled: true,
            ssh_listen_addr: "0.0.0.0".to_owned(),
            ssh_port: 2222,
            primary_ipv4: "127.0.0.1".to_owned(),
            host_bridge: true,
            host_runtime: false,
            uptime_ms: 120_000,
            restart_count: 0,
            active_leases: 0,
            last_error: None,
            disk: ContainerDiskStats {
                image_exists: true,
                image_allocated_bytes: 900 * 1024 * 1024,
                image_virtual_bytes: 32 * 1024 * 1024 * 1024,
                rootfs_total_bytes: Some(32 * 1024 * 1024 * 1024),
                rootfs_used_bytes: Some(640 * 1024 * 1024),
                rootfs_available_bytes: Some(31 * 1024 * 1024 * 1024),
            },
        }
    }

    #[test]
    fn builds_valid_sideveil_hud_scene_for_running_and_stopped_states() {
        let running_scene = build_strata_hud_scene(
            "strata.manager",
            &[sample_snapshot(ContainerPhase::Running)],
            2,
            true,
            true,
        );
        assert_eq!(running_scene["pluginId"], "strata.manager");
        assert_eq!(running_scene["backdrop"], "native-glass");
        assert_eq!(running_scene["clickThrough"], true);
        assert_eq!(
            running_scene["primitives"].as_array().map(Vec::len),
            Some(2)
        );
        assert_eq!(running_scene["texts"][0]["text"], "Strata  1/1 运行");
        assert_eq!(
            running_scene["texts"][1]["text"],
            "SSH :2222  任务 2  占用 640M"
        );

        let stopped_scene = build_strata_hud_scene(
            "strata.manager",
            &[sample_snapshot(ContainerPhase::Stopped)],
            0,
            false,
            false,
        );
        assert_eq!(stopped_scene["texts"][0]["text"], "Strata  0/1 待机");
    }
}
