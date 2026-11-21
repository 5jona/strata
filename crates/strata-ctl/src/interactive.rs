//! Local single-threaded `setns` and foreground `stage2` interactive TTY execution.

use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};

use serde_json::json;
use strata_protocol::rpc::{
    AttachLease, SessionAcquireParams, SessionMode, SessionReleaseParams, methods,
};
use strata_runtime::{EnterSpec, Stage2Spec, spec_pipe};

use crate::client::StrataClient;

/// Executes an interactive command inside a container with native TTY fidelity.
/// - `Attached`: drops the control socket and enters the running container directly via `setns`.
/// - `Oneshot` / `Debug`: keeps the `strata.sock` lease open while a foreground single-threaded
///   `strata __stage2` child runs, then releases the lease for zero-residual teardown.
pub(crate) fn run_interactive(
    mut client: StrataClient,
    container: Option<String>,
    debug: bool,
    command: Vec<String>,
) -> Result<i32, String> {
    if command.is_empty() {
        return Err("command must not be empty".to_owned());
    }

    let req = SessionAcquireParams { container, debug };
    let lease: AttachLease = client
        .call_typed(
            methods::SESSION_ACQUIRE,
            Some(serde_json::to_value(&req).map_err(|e| e.to_string())?),
        )
        .map_err(|(code, msg)| format!("error {code}: {msg}"))?;

    let caller_term = std::env::var("TERM").ok().filter(|s| !s.is_empty());

    match lease.mode {
        SessionMode::Attached => {
            let Some(ns_pid) = lease.ns_pid else {
                return Err("stratad returned Attached lease without ns_pid".to_owned());
            };
            let _ = client.call(
                methods::SESSION_RELEASE,
                Some(json!(SessionReleaseParams {
                    lease_id: lease.lease_id.clone()
                })),
            );
            drop(client);

            let spec = EnterSpec {
                container_id: lease.container,
                ns_pid,
                cap_profile: lease.cap_profile,
                command,
                workdir: Some("/root".to_owned()),
                term: caller_term,
            };
            match strata_runtime::run_enter(spec) {
                Ok(never) => match never {},
                Err(err) => Err(format!("failed to enter container: {err}")),
            }
        }
        SessionMode::Oneshot | SessionMode::Debug => {
            let spec = Stage2Spec {
                container_id: lease.container.clone(),
                mount_dir: lease.mount_dir.clone(),
                hostname: lease.hostname.clone(),
                oom_score_adj: None,
                cap_profile: lease.cap_profile,
                host_runtime: lease.host_runtime,
                bind_data_local_tmp: lease.bind_data_local_tmp,
                extra_char_devs: lease.extra_char_devs.clone(),
                extra_mounts: lease.extra_mounts.clone(),
                command,
                workdir: Some("/root".to_owned()),
                term: caller_term,
            };

            let exit_code = run_foreground_stage2(&spec);

            // Explicitly release lease before closing socket so teardown finishes synchronously.
            let _ = client.call(
                methods::SESSION_RELEASE,
                Some(json!(SessionReleaseParams {
                    lease_id: lease.lease_id
                })),
            );
            exit_code
        }
    }
}

fn run_foreground_stage2(spec: &Stage2Spec) -> Result<i32, String> {
    let self_exe = std::env::current_exe().map_err(|e| format!("resolve current_exe: {e}"))?;
    let spec_fd = spec_pipe::create_spec_pipe(spec).map_err(|e| e.to_string())?;

    let mut child = Command::new(self_exe)
        .arg("__stage2")
        .arg("--spec-fd")
        .arg(spec_fd.as_raw_fd().to_string())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("spawn foreground stage2: {e}"))?;

    drop(spec_fd);

    let status = child
        .wait()
        .map_err(|e| format!("wait foreground stage2: {e}"))?;
    Ok(status.code().unwrap_or(1))
}
