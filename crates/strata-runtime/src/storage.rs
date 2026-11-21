//! Container rootfs storage orchestration: safe mounting and zero-residual teardown.

pub mod maintenance;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

pub use maintenance::{backup_image_gzip, compact_image, fsck_image};

use strata_protocol::ContainerConfig;
use strata_protocol::constants::SELINUX_IMAGE_CONTEXT;
use strata_protocol::rpc::StorageCleanupReport;
use strata_sys::{cgroup, loopdev, mount, namespace, procfs, selinux};

use crate::error::{RuntimeError, RuntimeResult};
use crate::fs_atomic::ensure_dir_mode;
use crate::sync::sync_container_runtime_files;

pub(crate) const E2FSCK_BIN: &str = "/system/bin/e2fsck";

/// Mounts the container's sparse ext4 rootfs image safely with concurrent-mount protection,
/// SELinux labeling, optional `e2fsck -p`, stale `.rurienv` cleanup, and runtime file sync.
pub fn mount_rootfs(
    cfg: &ContainerConfig,
    module_dir: Option<&Path>,
) -> RuntimeResult<Option<PathBuf>> {
    let image_path = Path::new(&cfg.image_path);
    let mount_dir = Path::new(&cfg.mount_dir);

    if !image_path.is_file() {
        return Err(RuntimeError::ImageMissing(image_path.to_path_buf()));
    }

    loopdev::ensure_image_not_mounted_elsewhere(image_path, mount_dir)
        .map_err(RuntimeError::from_sys)?;
    ensure_dir_mode(mount_dir, 0o755)?;

    if mount::is_ext4_mounted(mount_dir).map_err(RuntimeError::from_sys)? {
        clear_legacy_ruri_markers(mount_dir);
        sync_container_runtime_files(mount_dir, cfg, module_dir)?;
        return Ok(None);
    }

    clear_legacy_ruri_markers(mount_dir);
    let _ = selinux::set_file_context(image_path, SELINUX_IMAGE_CONTEXT);
    let _ = loopdev::detach_image_loops(image_path);

    if cfg.fsck_on_mount && Path::new(E2FSCK_BIN).exists() {
        let out = Command::new(E2FSCK_BIN)
            .arg("-p")
            .arg(image_path)
            .output()
            .map_err(|source| RuntimeError::Io {
                op: "e2fsck -p",
                source,
            })?;
        let code = out.status.code().unwrap_or(-1);
        if code != 0 && code != 1 {
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            return Err(RuntimeError::FsckFailed {
                path: image_path.to_path_buf(),
                code,
                detail,
            });
        }
    }

    let attachment = loopdev::attach_loop(image_path).map_err(RuntimeError::from_sys)?;
    let dev_path = attachment.device_path.clone();

    if let Err(err) = mount::mount(
        Some(&dev_path),
        mount_dir,
        Some("ext4"),
        libc::MS_RELATIME,
        None,
    ) {
        let _ = loopdev::detach_loop(&dev_path);
        return Err(RuntimeError::from_sys(err));
    }

    if !mount_dir.join("bin/sh").exists() && !mount_dir.join("usr/bin/sh").exists() {
        let _ = mount::unmount_tree(mount_dir);
        let _ = loopdev::detach_image_loops(image_path);
        return Err(RuntimeError::InvalidRootfs(
            image_path.to_path_buf(),
            "missing /bin/sh inside mounted ext4 image",
        ));
    }

    clear_legacy_ruri_markers(mount_dir);
    sync_container_runtime_files(mount_dir, cfg, module_dir)?;
    drop(attachment);
    Ok(Some(dev_path))
}

/// Drains all container processes via dual signals (cgroup v2 + mount namespace inode),
/// removes the container cgroup, recursively unmounts, detaches all loops, and verifies zero leaks.
pub fn teardown_container(
    container_id: &str,
    cfg: &ContainerConfig,
    ns_pid: Option<u32>,
    mnt_ns_inode: Option<u64>,
) -> RuntimeResult<StorageCleanupReport> {
    if cgroup::current_process_in_strata_cgroup() {
        cgroup::escape_to_root_cgroups(std::process::id());
    }

    let cgroup_dir = cgroup::container_cgroup_v2_path(container_id);
    let resolved_ns =
        mnt_ns_inode.or_else(|| ns_pid.and_then(|pid| namespace::read_ns_inode(pid, "mnt").ok()));

    let drained_pids = drain_container_processes(&cgroup_dir, ns_pid, resolved_ns);
    cgroup::remove_container_cgroup(&cgroup_dir);

    let mount_dir = Path::new(&cfg.mount_dir);
    let image_path = Path::new(&cfg.image_path);

    let unmounted_paths = mount::unmount_tree(mount_dir).unwrap_or(0);
    let detached_loops = loopdev::detach_image_loops(image_path).unwrap_or(0);

    let residual_mounts = mount::mount_points_under(mount_dir)
        .map(|v| v.len())
        .unwrap_or(0);
    let residual_loops = loopdev::find_image_loops(image_path)
        .map(|v| v.len())
        .unwrap_or(0);

    Ok(StorageCleanupReport {
        drained_pids,
        unmounted_paths,
        detached_loops,
        residual_mounts,
        residual_loops,
    })
}

fn drain_container_processes(
    cgroup_dir: &Path,
    ns_pid: Option<u32>,
    target_mnt_ns: Option<u64>,
) -> usize {
    let mut pids = procfs::collect_container_pids(Some(cgroup_dir), target_mnt_ns, &[]);
    if let Some(pid) = ns_pid
        && pid > 1
        && namespace::pid_exists(pid)
        && !pids.contains(&pid)
    {
        pids.push(pid);
    }

    let initial_count = pids.len();
    for &pid in &pids {
        let _ = namespace::send_signal(pid, libc::SIGTERM);
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let remaining = procfs::collect_container_pids(Some(cgroup_dir), target_mnt_ns, &[]);
        if remaining.is_empty() && ns_pid.is_none_or(|p| !namespace::pid_exists(p)) {
            return initial_count;
        }
        thread::sleep(Duration::from_millis(100));
    }

    let _ = cgroup::kill_cgroup(cgroup_dir);
    let survivors = procfs::collect_container_pids(Some(cgroup_dir), target_mnt_ns, &[]);
    for &pid in &survivors {
        let _ = namespace::send_signal(pid, libc::SIGKILL);
    }
    if let Some(pid) = ns_pid
        && pid > 1
        && namespace::pid_exists(pid)
    {
        let _ = namespace::send_signal(pid, libc::SIGKILL);
    }

    let kill_deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < kill_deadline {
        let still = procfs::collect_container_pids(Some(cgroup_dir), target_mnt_ns, &[]);
        if still.is_empty() && ns_pid.is_none_or(|p| !namespace::pid_exists(p)) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    initial_count.max(survivors.len())
}

fn clear_legacy_ruri_markers(mount_dir: &Path) {
    let rurienv = mount_dir.join(".rurienv");
    if rurienv.exists() {
        let _ = mount::clear_immutable_flag(&rurienv);
        let _ = fs::remove_file(&rurienv);
    }
    let umounted = mount_dir.join(".ruri_umounted");
    if umounted.exists() {
        let _ = fs::remove_file(&umounted);
    }
}
