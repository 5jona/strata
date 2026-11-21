//! Single-threaded stage-2 container initializer (`unshare`, `/dev`, `pivot_root`, `capset`, `exec`).

use std::convert::Infallible;
use std::fs;
use std::os::fd::RawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use strata_protocol::capabilities::CapProfile;
use strata_protocol::config::BindMount;
use strata_protocol::constants::DEFAULT_CONTAINER_PATH;
use strata_sys::{caps, cgroup, devfs, mount, namespace};

use crate::error::{RuntimeError, RuntimeResult};
use crate::fs_atomic::ensure_dir_mode;
use crate::spec_pipe;

/// Serialized launch specification passed to a single-threaded stage-2 helper over a pipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage2Spec {
    pub container_id: String,
    pub mount_dir: String,
    pub hostname: String,
    pub oom_score_adj: Option<i16>,
    pub cap_profile: CapProfile,
    pub host_runtime: bool,
    pub bind_data_local_tmp: bool,
    pub extra_char_devs: Vec<String>,
    pub extra_mounts: Vec<BindMount>,
    pub command: Vec<String>,
    pub workdir: Option<String>,
    pub term: Option<String>,
}

/// Reads a `Stage2Spec` from `spec_fd`, marks optional `err_fd` as `FD_CLOEXEC`, and runs stage-2.
pub fn run_stage2_from_fd(spec_fd: RawFd, err_fd: Option<RawFd>) -> RuntimeResult<Infallible> {
    if let Some(fd) = err_fd {
        spec_pipe::mark_cloexec(fd)?;
    }
    let spec: Stage2Spec = spec_pipe::read_spec_fd(spec_fd)?;
    run_stage2(spec)
}

/// Executes full progressive container isolation in a single-threaded process and replaces
/// the process image with `spec.command` via `exec`.
pub fn run_stage2(spec: Stage2Spec) -> RuntimeResult<Infallible> {
    if spec.command.is_empty() {
        return Err(RuntimeError::InvalidArgument(
            "stage2 command must not be empty".to_owned(),
        ));
    }

    let mount_dir = Path::new(&spec.mount_dir);
    if !mount_dir.is_dir() {
        return Err(RuntimeError::InvalidRootfs(
            mount_dir.to_path_buf(),
            "mount directory does not exist",
        ));
    }

    // Move into dedicated container cgroup BEFORE unsharing CLONE_NEWCGROUP so /proc/self/cgroup
    // inside the container roots cleanly at "/".
    let _ = cgroup::attach_pid_to_container_cgroup(&spec.container_id, std::process::id());
    if let Some(score) = spec.oom_score_adj {
        let _ = cgroup::write_oom_score_adj(std::process::id(), score);
    }

    let _time_ns_ok = namespace::unshare_container_namespaces().map_err(RuntimeError::from_sys)?;
    mount::make_rprivate(Path::new("/")).map_err(RuntimeError::from_sys)?;
    namespace::set_hostname(&spec.hostname).map_err(RuntimeError::from_sys)?;

    if spec.host_runtime {
        devfs::setup_host_runtime_virtual_fs(mount_dir).map_err(RuntimeError::from_sys)?;
    } else {
        devfs::setup_isolated_virtual_fs(mount_dir, &spec.extra_char_devs)
            .map_err(RuntimeError::from_sys)?;
    }

    if spec.bind_data_local_tmp && Path::new("/data/local/tmp").is_dir() {
        bind_mount_into(
            Path::new("/data/local/tmp"),
            &mount_dir.join("data/local/tmp"),
            false,
        )?;
    }

    for bm in &spec.extra_mounts {
        let host_path = Path::new(&bm.host_path);
        if !host_path.exists() {
            continue;
        }
        let rel = bm.container_path.trim_start_matches('/');
        bind_mount_into(host_path, &mount_dir.join(rel), bm.read_only)?;
    }

    std::env::set_current_dir(mount_dir).map_err(|source| RuntimeError::Io {
        op: "chdir(mount_dir)",
        source,
    })?;
    mount::pivot_root_in_place().map_err(RuntimeError::from_sys)?;

    let workdir = spec.workdir.as_deref().unwrap_or("/root");
    if std::env::set_current_dir(workdir).is_err() {
        let _ = std::env::set_current_dir("/");
    }

    caps::apply_cap_profile(spec.cap_profile).map_err(RuntimeError::from_sys)?;

    exec_container_command(&spec.command, spec.term.as_deref())
}

pub(crate) fn exec_container_command(
    command: &[String],
    term: Option<&str>,
) -> RuntimeResult<Infallible> {
    let Some((program, args)) = command.split_first() else {
        return Err(RuntimeError::InvalidArgument(
            "command must not be empty".to_owned(),
        ));
    };

    let mut cmd = Command::new(program);
    cmd.args(args)
        .env_clear()
        .env("HOME", "/root")
        .env("USER", "root")
        .env("LOGNAME", "root")
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("PATH", DEFAULT_CONTAINER_PATH)
        .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
        .env("TERM", term.unwrap_or("xterm-256color"));

    let err = cmd.exec();
    Err(RuntimeError::Io {
        op: "execve container command",
        source: err,
    })
}

fn bind_mount_into(host_path: &Path, target_path: &Path, read_only: bool) -> RuntimeResult<()> {
    if host_path.is_dir() {
        ensure_dir_mode(target_path, 0o755)?;
    } else {
        if let Some(parent) = target_path.parent() {
            ensure_dir_mode(parent, 0o755)?;
        }
        if !target_path.exists() {
            let _ = fs::File::create(target_path);
        }
    }

    mount::mount(
        Some(host_path),
        target_path,
        None,
        libc::MS_BIND | libc::MS_REC,
        None,
    )
    .map_err(RuntimeError::from_sys)?;

    if read_only {
        let _ = mount::mount(
            None,
            target_path,
            None,
            libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY,
            None,
        );
    }
    Ok(())
}
