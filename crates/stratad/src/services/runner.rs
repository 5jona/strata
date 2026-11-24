//! Zero-polling `pidfd` + `AsyncFd` supervision loop, bounded log readers, and `-pgid` cleanup.

use std::os::fd::AsRawFd;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use strata_protocol::CapProfile;
use strata_protocol::rpc::{ContainerServiceSpec, ServiceStatus};
use strata_runtime::{EnterSpec, spec_pipe};
use strata_sys::{namespace, pidfd::PidFd, pty};
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, BufReader};
use tokio::process::Command;

use crate::services::ServiceSupervisor;
use crate::services::state::{MAX_SERVICE_LINE_BYTES, ServiceSlot, should_restart};
use crate::task_runner::io::read_bounded_line;

const SERVICE_LOG_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) fn spawn_service_loop(
    sup: Arc<ServiceSupervisor>,
    container_id: String,
    spec: ContainerServiceSpec,
    cap_profile: CapProfile,
    initial_ns_pid: u32,
    slot: Arc<ServiceSlot>,
    generation: u64,
) {
    tokio::spawn(async move {
        let max_restarts = spec.max_restarts.clamp(1, 50);
        let base_backoff_ms = spec.backoff_ms.clamp(250, 60_000);
        let mut ns_pid = initial_ns_pid;

        loop {
            if !is_generation_active(&slot, generation).await {
                return;
            }
            let spawn_res = spawn_once(&sup, &container_id, &spec, cap_profile, ns_pid).await;
            let (mut child, pid, pidfd) = match spawn_res {
                Ok(v) => v,
                Err(err_msg) => {
                    let stop_loop = handle_failure(
                        &sup,
                        &container_id,
                        &spec,
                        &slot,
                        generation,
                        max_restarts,
                        base_backoff_ms,
                        err_msg,
                    )
                    .await;
                    if stop_loop {
                        return;
                    }
                    if let Ok((_, new_ns, _)) =
                        sup.manager.running_target(Some(&container_id)).await
                    {
                        ns_pid = new_ns;
                    } else {
                        return;
                    }
                    continue;
                }
            };

            // Re-verify container is still running with the same ns_pid before registering service.
            if !sup.manager.is_running_with_pid(&container_id, ns_pid).await {
                let _ = tokio::task::spawn_blocking(move || terminate_process_group(pid)).await;
                let _ = child.wait().await;
                return;
            }

            let (prev_status, restarts) = {
                let mut st = slot.state.write().await;
                if !st.desired_running || st.generation != generation {
                    drop(st);
                    let _ = tokio::task::spawn_blocking(move || terminate_process_group(pid)).await;
                    let _ = child.wait().await;
                    return;
                }
                let prev = st.status;
                st.status = ServiceStatus::Running;
                st.pid = Some(pid);
                st.started_at = Some(Instant::now());
                st.last_error = None;
                (prev, st.restart_count)
            };
            sup.emit_state_changed(
                &container_id,
                &spec.id,
                prev_status,
                ServiceStatus::Running,
                Some(pid),
                restarts,
                None,
            );

            let out_task = child
                .stdout
                .take()
                .map(|r| spawn_log_reader(r, Arc::clone(&slot)));
            let err_task = child
                .stderr
                .take()
                .map(|r| spawn_log_reader(r, Arc::clone(&slot)));

            // Zero-polling exit notification via pidfd + AsyncFd.
            // SAFETY: `pidfd` exclusively owns its open kernel pidfd descriptor.
            if let Ok(async_fd) = unsafe { AsyncFd::register(pidfd) } {
                let _ = async_fd.readable().await;
            }
            let exit_code = match child.wait().await {
                Ok(status) => status.code().unwrap_or(1),
                Err(_) => 1,
            };

            // Immediately clear PID so a subsequent stop cannot signal a recycled PID,
            // and kill any orphaned grandchildren still holding stdout/stderr pipes open.
            {
                let mut st = slot.state.write().await;
                if st.pid == Some(pid) {
                    st.pid = None;
                }
            }
            let _ = tokio::task::spawn_blocking(move || reap_orphan_process_group(pid)).await;

            if let Some(mut t) = out_task
                && tokio::time::timeout(SERVICE_LOG_DRAIN_TIMEOUT, &mut t)
                    .await
                    .is_err()
            {
                t.abort();
            }
            if let Some(mut t) = err_task
                && tokio::time::timeout(SERVICE_LOG_DRAIN_TIMEOUT, &mut t)
                    .await
                    .is_err()
            {
                t.abort();
            }

            if !is_generation_active(&slot, generation).await {
                return;
            }

            if !should_restart(spec.restart_policy, exit_code) {
                let (prev, rc) = {
                    let mut st = slot.state.write().await;
                    let prev = st.status;
                    st.desired_running = false;
                    st.pid = None;
                    st.started_at = None;
                    st.status = if exit_code == 0 {
                        ServiceStatus::Stopped
                    } else {
                        ServiceStatus::Crashed
                    };
                    if exit_code != 0 {
                        st.last_error = Some(format!("exited with code {exit_code}"));
                    }
                    (prev, st.restart_count)
                };
                let next = if exit_code == 0 {
                    ServiceStatus::Stopped
                } else {
                    ServiceStatus::Crashed
                };
                sup.emit_state_changed(
                    &container_id,
                    &spec.id,
                    prev,
                    next,
                    None,
                    rc,
                    (exit_code != 0).then(|| format!("exited with code {exit_code}")),
                );
                return;
            }

            let stop_loop = handle_failure(
                &sup,
                &container_id,
                &spec,
                &slot,
                generation,
                max_restarts,
                base_backoff_ms,
                format!("exited with code {exit_code}"),
            )
            .await;
            if stop_loop {
                return;
            }

            if let Ok((_, new_ns, _)) = sup.manager.running_target(Some(&container_id)).await {
                ns_pid = new_ns;
            } else {
                return;
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
async fn handle_failure(
    sup: &ServiceSupervisor,
    container_id: &str,
    spec: &ContainerServiceSpec,
    slot: &ServiceSlot,
    generation: u64,
    max_restarts: u32,
    base_backoff_ms: u64,
    reason: String,
) -> bool {
    let (prev, next_status, restart_count) = {
        let mut st = slot.state.write().await;
        if !st.desired_running || st.generation != generation {
            return true;
        }
        let prev = st.status;
        st.pid = None;
        st.started_at = None;
        st.restart_count = st.restart_count.saturating_add(1);
        st.last_error = Some(reason.clone());
        let next = if st.restart_count >= max_restarts {
            st.desired_running = false;
            ServiceStatus::CircuitOpen
        } else {
            ServiceStatus::Crashed
        };
        st.status = next;
        (prev, next, st.restart_count)
    };
    sup.emit_state_changed(
        container_id,
        &spec.id,
        prev,
        next_status,
        None,
        restart_count,
        Some(reason),
    );
    if next_status == ServiceStatus::CircuitOpen {
        return true;
    }

    let shift = restart_count.saturating_sub(1).min(4);
    let delay_ms = base_backoff_ms.saturating_mul(1u64 << shift).min(60_000);
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    false
}

async fn spawn_once(
    sup: &ServiceSupervisor,
    container_id: &str,
    spec: &ContainerServiceSpec,
    cap_profile: CapProfile,
    ns_pid: u32,
) -> Result<(tokio::process::Child, u32, PidFd), String> {
    let enter = EnterSpec {
        container_id: container_id.to_owned(),
        ns_pid,
        cap_profile,
        command: spec.command.clone(),
        workdir: spec.workdir.clone(),
        term: Some("dumb".to_owned()),
        env: spec.env.clone(),
        pty_ctty: false,
        new_session: true,
    };
    let spec_fd = spec_pipe::create_spec_pipe(&enter).map_err(|e| e.to_string())?;
    let (err_read, err_write) = spec_pipe::create_exec_error_pipe().map_err(|e| e.to_string())?;
    let spec_raw = spec_fd.as_raw_fd();
    let err_raw = err_write.as_raw_fd();

    let mut cmd = Command::new(&sup.self_exe);
    cmd.arg("__exec")
        .arg("--spec-fd")
        .arg(spec_raw.to_string())
        .arg("--err-fd")
        .arg(err_raw.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: Clears `FD_CLOEXEC` only for this child's pipe fds via async-signal-safe `fcntl`.
    unsafe {
        cmd.pre_exec(move || spec_pipe::clear_cloexec_pre_exec(&[spec_raw, err_raw]));
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn service helper: {e}"))?;

    drop(spec_fd);
    drop(err_write);

    let exec_res = tokio::task::spawn_blocking(move || spec_pipe::read_exec_error(err_read)).await;
    match exec_res {
        Ok(Ok(None)) => {}
        Ok(Ok(Some(msg))) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(msg);
        }
        Ok(Err(e)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(e.to_string());
        }
        Err(e) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(format!("service exec check join: {e}"));
        }
    }

    let pid = child.id().unwrap_or(0);
    let pidfd = PidFd::open(pid).map_err(|e| format!("pidfd_open({pid}): {e}"))?;
    Ok((child, pid, pidfd))
}

fn spawn_log_reader<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
    slot: Arc<ServiceSlot>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf_reader = BufReader::new(reader);
        while let Ok(Some(line)) = read_bounded_line(&mut buf_reader, MAX_SERVICE_LINE_BYTES).await
        {
            slot.state.write().await.push_log(line);
        }
    })
}

async fn is_generation_active(slot: &ServiceSlot, generation: u64) -> bool {
    let st = slot.state.read().await;
    st.desired_running && st.generation == generation
}

/// Terminates any remaining orphan grandchildren in `-pgid` after the session leader exits.
fn reap_orphan_process_group(pgid: u32) {
    if pgid <= 1 {
        return;
    }
    if pty::signal_process_group(pgid, libc::SIGTERM).unwrap_or(false) {
        std::thread::sleep(Duration::from_millis(200));
        let _ = pty::signal_process_group(pgid, libc::SIGKILL);
    }
}

/// Terminates an entire service process group (`-pgid` SIGTERM -> 1.5s -> SIGKILL).
pub(super) fn terminate_process_group(pgid: u32) {
    if pgid <= 1 {
        return;
    }
    let _ = pty::signal_process_group(pgid, libc::SIGTERM);
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        if !namespace::pid_exists(pgid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = pty::signal_process_group(pgid, libc::SIGKILL);
}
