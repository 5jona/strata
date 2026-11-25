//! Android-compatible cgroup v2/v1 isolation, teardown killing, and `oom_score_adj` helpers.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use strata_protocol::constants::{CGROUP_V2_NAMESPACE, CGROUP_V2_ROOT};

use crate::error::{SysError, SysResult};

/// Known root cgroup `cgroup.procs` files across unified v2 and legacy Android v1 hierarchies.
const ROOT_CGROUP_PROCS: &[&str] = &[
    "/sys/fs/cgroup/cgroup.procs",
    "/dev/cpuctl/cgroup.procs",
    "/dev/cpuset/cgroup.procs",
    "/dev/freezer/cgroup.procs",
    "/dev/blkio/cgroup.procs",
];

/// Returns the canonical cgroup v2 directory path for a given container ID.
#[must_use]
pub fn container_cgroup_v2_path(container_id: &str) -> PathBuf {
    Path::new(CGROUP_V2_ROOT)
        .join(CGROUP_V2_NAMESPACE)
        .join(container_id)
}

/// Moves `pid` back to the root of unified v2 and Android legacy v1 cgroups so container
/// workloads and teardown helpers are detached from Android App cgroup throttling/freezing.
pub fn escape_to_root_cgroups(pid: u32) {
    let payload = format!("{pid}\n");
    for path in ROOT_CGROUP_PROCS {
        let p = Path::new(path);
        if p.exists() {
            let _ = fs::write(p, &payload);
        }
    }
}

/// Ensures `/sys/fs/cgroup/strata/<container_id>` exists when cgroup v2 is mounted.
pub fn ensure_container_cgroup(container_id: &str) -> SysResult<Option<PathBuf>> {
    let v2_root = Path::new(CGROUP_V2_ROOT);
    if !v2_root.join("cgroup.procs").exists() {
        return Ok(None);
    }
    let parent = v2_root.join(CGROUP_V2_NAMESPACE);
    fs::create_dir_all(&parent).map_err(|e| SysError::path_io("mkdir cgroup ns", &parent, e))?;
    let _ = fs::set_permissions(&parent, fs::Permissions::from_mode(0o700));

    let dir = parent.join(container_id);
    fs::create_dir_all(&dir).map_err(|e| SysError::path_io("mkdir container cgroup", &dir, e))?;
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    Ok(Some(dir))
}

/// Escapes legacy Android App v1 cgroups and moves `pid` into `/sys/fs/cgroup/strata/<id>`.
pub fn attach_pid_to_container_cgroup(container_id: &str, pid: u32) -> SysResult<Option<PathBuf>> {
    escape_to_root_cgroups(pid);
    let Some(dir) = ensure_container_cgroup(container_id)? else {
        return Ok(None);
    };
    let procs = dir.join("cgroup.procs");
    fs::write(&procs, format!("{pid}\n"))
        .map_err(|e| SysError::path_io("write cgroup.procs", &procs, e))?;
    Ok(Some(dir))
}

/// Parses newline-separated PIDs from a `cgroup.procs` buffer.
#[must_use]
pub fn parse_cgroup_procs(content: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Ok(pid) = trimmed.parse::<u32>()
            && pid > 0
        {
            pids.push(pid);
        }
    }
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// Reads all PIDs currently belonging to `cgroup_dir`.
#[must_use]
pub fn read_cgroup_procs(cgroup_dir: &Path) -> Vec<u32> {
    let procs_path = cgroup_dir.join("cgroup.procs");
    match fs::read_to_string(&procs_path) {
        Ok(raw) => parse_cgroup_procs(&raw),
        Err(_) => Vec::new(),
    }
}

/// Triggers kernel cgroup v2 `cgroup.kill` (available on Linux 5.14+ / Android 16 GKI).
/// Returns `true` if the file existed and the write succeeded.
#[must_use]
pub fn kill_cgroup(cgroup_dir: &Path) -> bool {
    let kill_file = cgroup_dir.join("cgroup.kill");
    if !kill_file.exists() {
        return false;
    }
    fs::write(&kill_file, "1\n").is_ok()
}

/// Removes an empty container cgroup v2 directory (ignoring `ENOENT`).
pub fn remove_container_cgroup(cgroup_dir: &Path) {
    let _ = fs::remove_dir(cgroup_dir);
}

/// Returns whether the calling process currently resides inside a Strata container cgroup.
#[must_use]
pub fn current_process_in_strata_cgroup() -> bool {
    match fs::read_to_string("/proc/self/cgroup") {
        Ok(raw) => raw.contains(&format!("/{CGROUP_V2_NAMESPACE}/")),
        Err(_) => false,
    }
}

/// Writes `/proc/<pid>/oom_score_adj` for a container init process.
pub fn write_oom_score_adj(pid: u32, score: i32) -> SysResult<()> {
    if !(-1000..=1000).contains(&score) {
        return Err(SysError::InvalidArgument {
            op: "write_oom_score_adj",
            detail: format!("oom_score_adj {score} out of bounds -1000..=1000"),
        });
    }
    let path = format!("/proc/{pid}/oom_score_adj");
    fs::write(&path, format!("{score}\n"))
        .map_err(|e| SysError::path_io("write oom_score_adj", path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_deduplicates_cgroup_procs() {
        let sample = "420\n102\n\n420\ninvalid\n0\n88\n";
        assert_eq!(parse_cgroup_procs(sample), vec![88, 102, 420]);
    }

    #[test]
    fn builds_canonical_v2_cgroup_path() {
        assert_eq!(
            container_cgroup_v2_path("main"),
            PathBuf::from("/sys/fs/cgroup/strata/main")
        );
    }
}
