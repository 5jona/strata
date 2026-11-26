//! Human-readable CLI output formatters for container status, lists, compaction, and cleanup.

mod tools;

pub(crate) use tools::{
    print_tools_cache_clean, print_tools_catalog, print_tools_config, print_tools_doctor,
    print_tools_list, print_tools_status,
};

use strata_protocol::rpc::{
    BackupResult, CompactResult, ContainerLogsResult, FsckResult, SshKeysResult,
    StorageCleanupReport, TaskSummary,
};
use strata_protocol::{ContainerPhase, ContainerStatusSnapshot, DaemonSummarySnapshot};

/// Formats a byte count into a compact human-readable IEC unit string.
#[must_use]
pub(crate) fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let val = bytes as f64;
    if val >= GIB {
        format!("{:.2} GiB", val / GIB)
    } else if val >= MIB {
        format!("{:.1} MiB", val / MIB)
    } else if val >= KIB {
        format!("{:.1} KiB", val / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Prints detailed status for a single container.
pub(crate) fn print_status(s: &ContainerStatusSnapshot) {
    let phase = phase_label(s.phase);
    let ns_pid = s
        .ns_pid
        .map_or_else(|| "stopped".to_owned(), |p| p.to_string());
    let loops = if s.loop_devices.is_empty() {
        "none".to_owned()
    } else {
        s.loop_devices.join(" ")
    };
    let cgroup = s.cgroup_path.as_deref().unwrap_or("none");

    println!(
        "container     : {}{}",
        s.id,
        if s.is_default { " (default)" } else { "" }
    );
    println!(
        "state         : {phase} (ns_pid={ns_pid}, uptime={}s, restarts={}, leases={})",
        s.uptime_ms / 1000,
        s.restart_count,
        s.active_leases
    );
    println!("hostname      : {}", s.hostname);
    if s.ssh_enabled {
        println!(
            "ssh           : ssh -p {} root@{} (listen={}:{})",
            s.ssh_port, s.primary_ipv4, s.ssh_listen_addr, s.ssh_port
        );
    }
    println!(
        "rootfs_mounted: {} ({})",
        if s.mounted { "yes" } else { "no" },
        s.mount_dir
    );
    println!("loop_devices  : {loops}");
    println!("cgroup        : {cgroup}");
    if s.disk.image_exists {
        println!(
            "image         : {} (used={} / cap={})",
            s.image_path,
            format_bytes(s.disk.image_allocated_bytes),
            format_bytes(s.disk.image_virtual_bytes)
        );
    } else {
        println!("image         : {} (missing)", s.image_path);
    }
    if let (Some(used), Some(total)) = (s.disk.rootfs_used_bytes, s.disk.rootfs_total_bytes) {
        let pct = if total > 0 {
            (used as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        println!(
            "rootfs_space  : {} used / {} total ({pct:.1}%)",
            format_bytes(used),
            format_bytes(total)
        );
    }
    println!(
        "host_bridge   : {} (host_runtime={})",
        s.host_bridge, s.host_runtime
    );
    if let Some(err) = &s.last_error {
        println!("last_error    : {err}");
    }
}

/// Prints a compact table of all configured containers.
pub(crate) fn print_list(items: &[ContainerStatusSnapshot]) {
    println!(
        "{:<14} {:<10} {:<8} {:<10} {:<8} IMAGE",
        "CONTAINER", "STATE", "NS_PID", "SSH_PORT", "MOUNTED"
    );
    for s in items {
        let name = if s.is_default {
            format!("{}*", s.id)
        } else {
            s.id.clone()
        };
        let pid = s.ns_pid.map_or_else(|| "-".to_owned(), |p| p.to_string());
        println!(
            "{:<14} {:<10} {:<8} {:<10} {:<8} {}",
            name,
            phase_label(s.phase),
            pid,
            s.ssh_port,
            if s.mounted { "yes" } else { "no" },
            s.image_path
        );
    }
}

pub(crate) fn print_cleanup(container: &str, r: &StorageCleanupReport) {
    println!(
        "Container '{container}' stopped (drained_pids={}, unmounted={}, detached_loops={}, residual_mounts={}, residual_loops={}).",
        r.drained_pids, r.unmounted_paths, r.detached_loops, r.residual_mounts, r.residual_loops
    );
}

pub(crate) fn print_compact(r: &CompactResult) {
    println!(
        "Compacted '{}': {} -> {} (FITRIM reported {}).",
        r.container,
        format_bytes(r.before_bytes),
        format_bytes(r.after_bytes),
        format_bytes(r.trimmed_bytes)
    );
}

pub(crate) fn print_fsck(r: &FsckResult) {
    println!("e2fsck '{}' exit_code={}", r.container, r.exit_code);
    if !r.output.is_empty() {
        println!("{}", r.output);
    }
}

pub(crate) fn print_backup(r: &BackupResult) {
    println!(
        "Backup of '{}' written to '{}' ({}, restarted={}).",
        r.container,
        r.output_path,
        format_bytes(r.bytes_written),
        r.restarted
    );
}

pub(crate) fn print_logs(r: &ContainerLogsResult) {
    println!("=== {} ({}) ===", r.container, r.log_path);
    if r.lines.is_empty() {
        println!("(no log output yet)");
    } else {
        for line in &r.lines {
            println!("{line}");
        }
    }
}

pub(crate) fn print_ssh_keys(r: &SshKeysResult) {
    if r.added > 0 {
        println!("Added {} key(s) for '{}'.", r.added, r.container);
    }
    if r.keys.is_empty() {
        println!("(no authorized keys)");
    } else {
        for k in &r.keys {
            println!("{k}");
        }
    }
}

pub(crate) fn print_daemon_status(d: &DaemonSummarySnapshot) {
    println!("pid               : {} (uid={})", d.pid, d.uid);
    println!("uptime            : {}s", d.uptime_ms / 1000);
    println!(
        "default_container : {}",
        d.default_container.as_deref().unwrap_or("(none)")
    );
    println!(
        "containers        : {}/{} running",
        d.running_containers, d.total_containers
    );
    println!("active_tasks      : {}", d.active_tasks);
    println!("sideveil_bridge   : {}", d.sideveil_connected);
}

pub(crate) fn print_tasks(tasks: &[TaskSummary]) {
    if tasks.is_empty() {
        println!("(no recorded tasks)");
        return;
    }
    println!(
        "{:<10} {:<12} {:<8} {:<10} COMMAND",
        "TASK_ID", "CONTAINER", "PID", "STATE"
    );
    for t in tasks {
        let state = if t.running {
            "running".to_owned()
        } else {
            format!("exit({})", t.exit_code.unwrap_or(-1))
        };
        println!(
            "{:<10} {:<12} {:<8} {:<10} {}",
            t.task_id,
            t.container,
            t.pid,
            state,
            t.command.join(" ")
        );
    }
}

fn phase_label(phase: ContainerPhase) -> &'static str {
    match phase {
        ContainerPhase::Stopped => "stopped",
        ContainerPhase::Starting => "starting",
        ContainerPhase::Running => "running",
        ContainerPhase::Stopping => "stopping",
        ContainerPhase::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_human_readable_byte_sizes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.00 GiB");
    }
}
