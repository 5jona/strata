//! Single-threaded `setns` container re-entry shared by CLI, task runner, PTY, and services.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::os::fd::RawFd;

use serde::{Deserialize, Serialize};
use strata_protocol::capabilities::CapProfile;
use strata_protocol::constants::DEFAULT_CONTAINER_WORKDIR;
use strata_sys::{caps, cgroup, namespace, pty};

use crate::error::{RuntimeError, RuntimeResult};
use crate::spec_pipe;
use crate::stage2::exec_container_command;

/// Serialized specification for entering an already-running container's namespaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnterSpec {
    pub container_id: String,
    pub ns_pid: u32,
    pub cap_profile: CapProfile,
    pub command: Vec<String>,
    pub workdir: Option<String>,
    pub term: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub pty_ctty: bool,
    #[serde(default)]
    pub new_session: bool,
}

/// Reads an `EnterSpec` from `spec_fd`, marks optional `err_fd` as `FD_CLOEXEC`, and enters.
pub fn run_enter_from_fd(spec_fd: RawFd, err_fd: Option<RawFd>) -> RuntimeResult<Infallible> {
    if let Some(fd) = err_fd {
        spec_pipe::mark_cloexec(fd)?;
    }
    let spec: EnterSpec = spec_pipe::read_spec_fd(spec_fd)?;
    run_enter(spec)
}

/// Joins the target container's cgroup and namespaces (`uts`, `cgroup`, `time`, `mnt`),
/// optionally sets up a new session / controlling TTY, applies capability bounds, and execs.
pub fn run_enter(spec: EnterSpec) -> RuntimeResult<Infallible> {
    if spec.command.is_empty() {
        return Err(RuntimeError::InvalidArgument(
            "enter command must not be empty".to_owned(),
        ));
    }
    if spec.ns_pid <= 1 || !namespace::pid_exists(spec.ns_pid) {
        return Err(RuntimeError::ProcessNotRunning(spec.ns_pid));
    }

    let _ = cgroup::attach_pid_to_container_cgroup(&spec.container_id, std::process::id());
    namespace::enter_container_namespaces(spec.ns_pid).map_err(RuntimeError::from_sys)?;

    if spec.pty_ctty {
        pty::setup_controlling_tty(0).map_err(RuntimeError::from_sys)?;
    } else if spec.new_session {
        pty::create_new_session().map_err(RuntimeError::from_sys)?;
    }

    let workdir = spec.workdir.as_deref().unwrap_or(DEFAULT_CONTAINER_WORKDIR);
    if std::env::set_current_dir(workdir).is_err() && std::env::set_current_dir("/root").is_err() {
        let _ = std::env::set_current_dir("/");
    }

    caps::apply_cap_profile(spec.cap_profile).map_err(RuntimeError::from_sys)?;
    exec_container_command(&spec.command, spec.term.as_deref(), Some(&spec.env))
}
