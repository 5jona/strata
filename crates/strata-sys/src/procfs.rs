//! `/proc` process discovery, dual-signal container PID collection, `SO_PEERCRED`, and disk stats.

use std::collections::BTreeSet;
use std::ffi::CString;
use std::fs;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::cgroup;
use crate::error::{SysError, SysResult};
use crate::namespace;

/// Classpath environment keys mirrored from Android `system_server` into `/run/strata/host.env`.
pub const ANDROID_CLASSPATH_KEYS: &[&str] = &[
    "BOOTCLASSPATH",
    "DEX2OATBOOTCLASSPATH",
    "SYSTEMSERVERCLASSPATH",
    "STANDALONE_SYSTEMSERVER_JARS",
];

/// Collects all live PIDs belonging to a container using two independent signals:
/// 1. `/sys/fs/cgroup/strata/<id>/cgroup.procs` (if present).
/// 2. `/proc/[0-9]*/ns/mnt` matching `target_mnt_ns_inode` (only when verified distinct from PID 1).
///
/// Always excludes PID 1, the calling process, and any caller-specified `excluded_pids`.
#[must_use]
pub fn collect_container_pids(
    cgroup_dir: Option<&Path>,
    target_mnt_ns_inode: Option<u64>,
    excluded_pids: &[u32],
) -> Vec<u32> {
    let mut found = BTreeSet::new();

    if let Some(dir) = cgroup_dir {
        for pid in cgroup::read_cgroup_procs(dir) {
            found.insert(pid);
        }
    }

    if let Some(target_ns) = target_mnt_ns_inode {
        let host_ns = namespace::host_mount_ns_inode().ok();
        if host_ns.is_some() && host_ns != Some(target_ns) {
            scan_proc_for_mnt_ns(target_ns, &mut found);
        }
    }

    let self_pid = std::process::id();
    found
        .into_iter()
        .filter(|&pid| pid > 1 && pid != self_pid && !excluded_pids.contains(&pid))
        .filter(|&pid| namespace::pid_exists(pid))
        .collect()
}

/// Verifies the peer UID of an accepted Unix domain socket via kernel `SO_PEERCRED`.
pub fn peer_cred_uid(fd: RawFd) -> SysResult<u32> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: u32::MAX,
        gid: u32::MAX,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `fd` is a connected AF_UNIX stream socket and `cred` has exact `ucred` size.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast::<libc::c_void>(),
            &mut len,
        )
    };
    if rc < 0 {
        return Err(SysError::last_os("getsockopt(SO_PEERCRED)"));
    }
    Ok(cred.uid)
}

/// Captures Android framework classpath variables from the running `system_server` environment
/// (or falls back to the current process environment).
#[must_use]
pub fn read_android_classpath_env() -> Vec<(String, String)> {
    let sys_env = find_pid_by_comm("system_server")
        .and_then(|pid| fs::read(format!("/proc/{pid}/environ")).ok())
        .map(|bytes| parse_proc_environ(&bytes))
        .unwrap_or_default();

    let mut out = Vec::new();
    for &key in ANDROID_CLASSPATH_KEYS {
        if let Ok(val) = std::env::var(key)
            && !val.is_empty()
        {
            out.push((key.to_owned(), val));
            continue;
        }
        if let Some((_, val)) = sys_env.iter().find(|(k, _)| k == key)
            && !val.is_empty()
        {
            out.push((key.to_owned(), val.clone()));
        }
    }
    out
}

/// Returns `(allocated_bytes, virtual_bytes)` for a sparse rootfs image file.
pub fn image_disk_usage(path: &Path) -> SysResult<(u64, u64)> {
    let meta = fs::metadata(path).map_err(|e| SysError::path_io("stat image", path, e))?;
    let allocated = meta.blocks().saturating_mul(512);
    Ok((allocated, meta.size()))
}

/// Returns `(total_bytes, used_bytes, available_bytes)` for a mounted filesystem path.
pub fn statvfs_bytes(path: &Path) -> SysResult<(u64, u64, u64)> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| SysError::InvalidCString(path.to_string_lossy().into_owned()))?;
    let mut vfs: libc::statvfs =
        // SAFETY: `libc::statvfs` is a plain C struct safe to zero-initialize before `statvfs(2)`.
        unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a valid NUL-terminated string and `vfs` is writable `statvfs` storage.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut vfs) };
    if rc < 0 {
        return Err(SysError::path_os("statvfs", path));
    }
    let frsize = if vfs.f_frsize > 0 {
        vfs.f_frsize as u64
    } else {
        vfs.f_bsize as u64
    };
    let total = (vfs.f_blocks as u64).saturating_mul(frsize);
    let free = (vfs.f_bfree as u64).saturating_mul(frsize);
    let avail = (vfs.f_bavail as u64).saturating_mul(frsize);
    let used = total.saturating_sub(free);
    Ok((total, used, avail))
}

/// Parses a NUL-delimited `/proc/<pid>/environ` byte buffer into `(KEY, VALUE)` pairs.
#[must_use]
pub fn parse_proc_environ(raw: &[u8]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for entry in raw.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        let Ok(text) = std::str::from_utf8(entry) else {
            continue;
        };
        if let Some((k, v)) = text.split_once('=')
            && !k.is_empty()
        {
            pairs.push((k.to_owned(), v.to_owned()));
        }
    }
    pairs
}

fn scan_proc_for_mnt_ns(target_ns: u64, out: &mut BTreeSet<u32>) {
    let Ok(entries) = fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let Ok(pid) = name_str.parse::<u32>() else {
            continue;
        };
        if pid <= 1 {
            continue;
        }
        if let Ok(ns) = namespace::read_ns_inode(pid, "mnt")
            && ns == target_ns
        {
            out.insert(pid);
        }
    }
}

fn find_pid_by_comm(target_name: &str) -> Option<u32> {
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let Ok(pid) = name_str.parse::<u32>() else {
            continue;
        };
        let cmdline_path = entry.path().join("cmdline");
        let Ok(raw) = fs::read(cmdline_path) else {
            continue;
        };
        let argv0 = raw.split(|&b| b == 0).next().unwrap_or_default();
        if argv0 == target_name.as_bytes() {
            return Some(pid);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nul_delimited_proc_environ() {
        let raw = b"BOOTCLASSPATH=/apex/a.jar:/apex/b.jar\0EMPTY=\0PATH=/system/bin\0";
        let parsed = parse_proc_environ(raw);
        assert_eq!(
            parsed,
            vec![
                (
                    "BOOTCLASSPATH".to_owned(),
                    "/apex/a.jar:/apex/b.jar".to_owned()
                ),
                ("EMPTY".to_owned(), String::new()),
                ("PATH".to_owned(), "/system/bin".to_owned()),
            ]
        );
    }
}
