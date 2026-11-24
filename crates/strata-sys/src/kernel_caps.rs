//! Live Linux/Android kernel container capability probing for `daemon.capabilities`.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use flate2::read::GzDecoder;
use strata_protocol::rpc::KernelCapabilitiesResult;

/// Probes the running host kernel for container-relevant namespaces, filesystems, and syscalls.
#[must_use]
pub fn probe_kernel_capabilities() -> KernelCapabilitiesResult {
    let kernel_release = fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "unknown".to_owned());

    let filesystems = fs::read_to_string("/proc/filesystems")
        .map(|s| parse_proc_filesystems(&s))
        .unwrap_or_default();
    let cgroups = fs::read_to_string("/proc/cgroups")
        .map(|s| parse_proc_cgroups(&s))
        .unwrap_or_default();
    let kcfg = read_proc_config_gz().unwrap_or_default();

    let pid_ns =
        kcfg.contains("CONFIG_PID_NS") || Path::new("/proc/self/ns/pid_for_children").exists();
    let ipc_ns = kcfg.contains("CONFIG_IPC_NS") || Path::new("/proc/self/ns/ipc").exists();
    let sysv_ipc = kcfg.contains("CONFIG_SYSVIPC")
        || Path::new("/proc/sysvipc/shm").exists()
        || Path::new("/proc/sys/kernel/shmmax").exists();
    let user_ns = kcfg.contains("CONFIG_USER_NS") || Path::new("/proc/self/ns/user").exists();
    let net_ns = kcfg.contains("CONFIG_NET_NS") || Path::new("/proc/self/ns/net").exists();
    let overlay_fs = kcfg.contains("CONFIG_OVERLAY_FS") || filesystems.contains("overlay");
    let fuse_fs = kcfg.contains("CONFIG_FUSE_FS") || filesystems.contains("fuse");
    let cgroup_v2 =
        filesystems.contains("cgroup2") || Path::new("/sys/fs/cgroup/cgroup.controllers").exists();
    let cgroup_pids = kcfg.contains("CONFIG_CGROUP_PIDS")
        || cgroups.contains("pids")
        || read_cgroup2_controllers().contains("pids");
    let cgroup_bpf = kcfg.contains("CONFIG_CGROUP_BPF") || Path::new("/sys/fs/bpf").exists();
    let io_uring = kcfg.contains("CONFIG_IO_URING") || probe_io_uring_syscall();
    let kvm = kcfg.contains("CONFIG_KVM") || Path::new("/dev/kvm").exists();
    let devtmpfs = kcfg.contains("CONFIG_DEVTMPFS") || filesystems.contains("devtmpfs");

    let (systemd_ready, docker_ready) =
        compute_readiness(pid_ns, ipc_ns, net_ns, overlay_fs, cgroup_v2);

    KernelCapabilitiesResult {
        kernel_release,
        pid_ns,
        ipc_ns,
        sysv_ipc,
        user_ns,
        net_ns,
        overlay_fs,
        fuse_fs,
        cgroup_v2,
        cgroup_pids,
        cgroup_bpf,
        io_uring,
        kvm,
        devtmpfs,
        systemd_ready,
        docker_ready,
    }
}

/// Computes derived high-level container readiness flags (`systemd_ready`, `docker_ready`).
#[must_use]
pub fn compute_readiness(
    pid_ns: bool,
    ipc_ns: bool,
    net_ns: bool,
    overlay_fs: bool,
    cgroup_v2: bool,
) -> (bool, bool) {
    let systemd_ready = pid_ns && cgroup_v2;
    let docker_ready = pid_ns && ipc_ns && net_ns && overlay_fs && cgroup_v2;
    (systemd_ready, docker_ready)
}

/// Parses filesystem names from `/proc/filesystems`.
#[must_use]
pub fn parse_proc_filesystems(raw: &str) -> BTreeSet<String> {
    raw.lines()
        .filter_map(|line| line.split_whitespace().last())
        .map(ToOwned::to_owned)
        .collect()
}

/// Parses enabled cgroup subsystem names from `/proc/cgroups`.
#[must_use]
pub fn parse_proc_cgroups(raw: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 4 && parts[3] == "1" {
            out.insert(parts[0].to_owned());
        }
    }
    out
}

/// Parses enabled `CONFIG_*=y` / `CONFIG_*=m` keys from uncompressed kernel config text.
#[must_use]
pub fn parse_kernel_config_text(raw: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some((key, val)) = trimmed.split_once('=')
            && key.starts_with("CONFIG_")
            && (val == "y" || val == "m")
        {
            out.insert(key.to_owned());
        }
    }
    out
}

fn read_cgroup2_controllers() -> BTreeSet<String> {
    fs::read_to_string("/sys/fs/cgroup/cgroup.controllers")
        .map(|s| s.split_whitespace().map(ToOwned::to_owned).collect())
        .unwrap_or_default()
}

fn read_proc_config_gz() -> Option<BTreeSet<String>> {
    let file = File::open("/proc/config.gz").ok()?;
    let mut decoder = GzDecoder::new(file);
    let mut text = String::new();
    decoder.read_to_string(&mut text).ok()?;
    Some(parse_kernel_config_text(&text))
}

fn probe_io_uring_syscall() -> bool {
    // SAFETY: Passing 0 entries and NULL params never allocates; returns EFAULT/EINVAL if supported,
    // or ENOSYS when CONFIG_IO_URING is disabled.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_io_uring_setup,
            0 as libc::c_uint,
            std::ptr::null_mut::<libc::c_void>(),
        )
    };
    if rc >= 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOSYS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_filesystems_cgroups_and_kernel_config() {
        let fs_sample = "nodev\tsysfs\nnodev\ttmpfs\n\text4\nnodev\toverlay\nnodev\tcgroup2\n";
        let fss = parse_proc_filesystems(fs_sample);
        assert!(fss.contains("ext4"));
        assert!(fss.contains("overlay"));
        assert!(fss.contains("cgroup2"));
        assert!(!fss.contains("devtmpfs"));

        let cg_sample = "#subsys_name\thierarchy\tnum_cgroups\tenabled\ncpu\t0\t12\t1\npids\t0\t5\t0\nmemory\t0\t12\t1\n";
        let cgs = parse_proc_cgroups(cg_sample);
        assert!(cgs.contains("cpu"));
        assert!(cgs.contains("memory"));
        assert!(!cgs.contains("pids"));

        let kcfg_sample =
            "# CONFIG_PID_NS is not set\nCONFIG_NET_NS=y\nCONFIG_OVERLAY_FS=y\nCONFIG_ZRAM=m\n";
        let kcfg = parse_kernel_config_text(kcfg_sample);
        assert!(!kcfg.contains("CONFIG_PID_NS"));
        assert!(kcfg.contains("CONFIG_NET_NS"));
        assert!(kcfg.contains("CONFIG_OVERLAY_FS"));
        assert!(kcfg.contains("CONFIG_ZRAM"));
    }

    #[test]
    fn computes_systemd_and_docker_readiness() {
        // Stock Android 16 GKI 6.12 profile
        assert_eq!(
            compute_readiness(false, false, true, true, true),
            (false, false)
        );
        // DIY GKI with PID_NS only
        assert_eq!(
            compute_readiness(true, false, true, true, true),
            (true, false)
        );
        // DIY GKI with PID_NS + IPC_NS
        assert_eq!(
            compute_readiness(true, true, true, true, true),
            (true, true)
        );
    }

    #[test]
    fn live_probe_succeeds_without_panicking() {
        let caps = probe_kernel_capabilities();
        assert!(!caps.kernel_release.is_empty());
    }
}
