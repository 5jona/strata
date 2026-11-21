//! Single-threaded `setns` container re-entry shared by interactive CLI and headless task runner.

use std::convert::Infallible;
use std::os::fd::RawFd;

use serde::{Deserialize, Serialize};
use strata_protocol::capabilities::CapProfile;
use strata_sys::{caps, cgroup, namespace};

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
/// applies capability bounds, and replaces the process image with `spec.command`.
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

    let workdir = spec.workdir.as_deref().unwrap_or("/root");
    if std::env::set_current_dir(workdir).is_err() {
        let _ = std::env::set_current_dir("/");
    }

    caps::apply_cap_profile(spec.cap_profile).map_err(RuntimeError::from_sys)?;
    exec_container_command(&spec.command, spec.term.as_deref())
}
