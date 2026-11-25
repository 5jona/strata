//! Single-threaded stage-2 helper spawning and `Stage2Spec` construction.

use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use strata_protocol::ContainerConfig;
use strata_protocol::constants::{DEFAULT_CONTAINER_WORKDIR, DEFAULT_LOG_MAX_BYTES};
use strata_runtime::{Stage2Spec, fs_atomic, spec_pipe};
use strata_sys::{namespace, pidfd::PidFd};

/// Builds the serialized `Stage2Spec` for launching a container's init process.
pub(super) fn build_stage2_spec(id: &str, cfg: &ContainerConfig) -> Stage2Spec {
    Stage2Spec {
        container_id: id.to_owned(),
        mount_dir: cfg.mount_dir.clone(),
        hostname: cfg.hostname.clone(),
        oom_score_adj: cfg.oom_score_adj,
        cap_profile: cfg.cap_profile(),
        host_runtime: cfg.host_runtime,
        bind_data_local_tmp: cfg.bind_data_local_tmp,
        extra_char_devs: cfg.extra_char_devs.clone(),
        extra_mounts: cfg.extra_mounts.clone(),
        command: cfg.effective_init_command(),
        workdir: Some(DEFAULT_CONTAINER_WORKDIR.to_owned()),
        term: Some("xterm-256color".to_owned()),
    }
}

/// Spawns `stratad __init` with inherited spec/error pipes and returns `(child, pid, pidfd, mnt_ns)`.
pub(super) fn spawn_stage2_init(
    self_exe: &Path,
    log_path: &Path,
    spec: &Stage2Spec,
) -> Result<(Child, u32, PidFd, Option<u64>), String> {
    let _ = fs_atomic::rotate_log_file(log_path, DEFAULT_LOG_MAX_BYTES);
    let log_out = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| format!("open log '{}': {e}", log_path.display()))?;
    let log_err = log_out
        .try_clone()
        .map_err(|e| format!("clone log fd: {e}"))?;

    let spec_fd = spec_pipe::create_spec_pipe(spec).map_err(|e| e.to_string())?;
    let (err_read, err_write) = spec_pipe::create_exec_error_pipe().map_err(|e| e.to_string())?;
    let spec_raw = spec_fd.as_raw_fd();
    let err_raw = err_write.as_raw_fd();

    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(self_exe);
    cmd.arg("__init")
        .arg("--spec-fd")
        .arg(spec_raw.to_string())
        .arg("--err-fd")
        .arg(err_raw.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_out))
        .stderr(Stdio::from(log_err));
    // SAFETY: Clears `FD_CLOEXEC` exclusively on this child's pipe fds using async-signal-safe `fcntl`.
    unsafe {
        cmd.pre_exec(move || spec_pipe::clear_cloexec_pre_exec(&[spec_raw, err_raw]));
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn stage2 helper: {e}"))?;

    drop(spec_fd);
    drop(err_write);

    match spec_pipe::read_exec_error(err_read) {
        Ok(None) => {}
        Ok(Some(err_msg)) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err_msg);
        }
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err.to_string());
        }
    }

    let pid = child.id();
    let pidfd = match PidFd::open(pid) {
        Ok(pfd) => pfd,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("pidfd_open({pid}): {e}"));
        }
    };
    let mnt_ns = namespace::read_ns_inode(pid, "mnt").ok();
    Ok((child, pid, pidfd, mnt_ns))
}
