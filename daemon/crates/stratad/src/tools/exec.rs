//! Bounded child-process execution and container tool environment loading for `ToolsManager`.

use std::collections::BTreeMap;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use strata_protocol::ContainerConfig;
use strata_runtime::{EnterSpec, spec_pipe};
use strata_sys::pidfd::PidFd;
use strata_sys::rootfs_io::RootDir;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::error::{DaemonError, DaemonResult};

pub(super) const MISE_MIRRORS_ENV_REL: &str = "etc/strata/mise/mirrors.env";
pub(super) const MISE_SECRETS_ENV_REL: &str = "etc/strata/mise/secrets.env";
const MAX_CAPTURE_BYTES: usize = 4 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub(super) struct ExecOutput {
    pub(super) status: i32,
    pub(super) stdout: String,
    pub(super) stderr: String,
    #[allow(dead_code)]
    pub(super) timed_out: bool,
}

/// Parses `KEY="VALUE"` or `export KEY="VALUE"` lines from Strata env files.
pub(super) fn parse_env_assignments(raw: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let body = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim();
        let Some((key, val)) = body.split_once('=') else {
            continue;
        };
        let k = key.trim();
        if k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        map.insert(k.to_owned(), unquote_shell_value(val.trim()));
    }
    map
}

fn unquote_shell_value(raw: &str) -> String {
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        let inner = &raw[1..raw.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return raw[1..raw.len() - 1].to_owned();
    }
    raw.to_owned()
}

/// Escapes `val` inside double quotes for POSIX `export KEY="..."` files.
pub(super) fn quote_posix_env(val: &str) -> String {
    let mut out = String::with_capacity(val.len() + 2);
    out.push('"');
    for ch in val.chars() {
        if matches!(ch, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Reads a regular UTF-8 file inside `root` safely without following symlinks.
pub(super) fn read_root_utf8(root: &RootDir, rel: &str) -> Option<String> {
    let bytes = root.read_file(Path::new(rel)).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Loads mirror + secret env overrides from the mounted container rootfs without leaking to logs.
pub(super) fn load_container_tool_env_from_root(root: &RootDir) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for rel in [MISE_MIRRORS_ENV_REL, MISE_SECRETS_ENV_REL] {
        if let Some(raw) = read_root_utf8(root, rel) {
            env.extend(parse_env_assignments(&raw));
        }
    }
    env
}

/// Opens `mount_dir` safely and reads `mirrors.env` + `secrets.env`.
pub(super) fn load_container_tool_env_from_mount(mount_dir: &Path) -> BTreeMap<String, String> {
    let Ok(root) = RootDir::open(mount_dir) else {
        return BTreeMap::new();
    };
    load_container_tool_env_from_root(&root)
}

/// Runs a short read-only/utility command inside a running container and captures bounded output.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_container_command_bounded(
    self_exe: &Path,
    container_id: &str,
    ns_pid: u32,
    cfg: &ContainerConfig,
    command: Vec<String>,
    env: BTreeMap<String, String>,
    timeout: Duration,
) -> DaemonResult<ExecOutput> {
    let spec = EnterSpec {
        container_id: container_id.to_owned(),
        ns_pid,
        cap_profile: cfg.cap_profile(),
        command,
        workdir: None,
        term: Some("dumb".to_owned()),
        env,
        pty_ctty: false,
        new_session: false,
    };

    let spec_fd = spec_pipe::create_spec_pipe(&spec)?;
    let (err_read, err_write) = spec_pipe::create_exec_error_pipe()?;
    let spec_raw = spec_fd.as_raw_fd();
    let err_raw = err_write.as_raw_fd();

    let mut cmd = Command::new(self_exe);
    cmd.arg("__exec")
        .arg("--spec-fd")
        .arg(spec_raw.to_string())
        .arg("--err-fd")
        .arg(err_raw.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: Clears `FD_CLOEXEC` only on child helper fds via async-signal-safe `fcntl`.
    unsafe {
        cmd.pre_exec(move || spec_pipe::clear_cloexec_pre_exec(&[spec_raw, err_raw]));
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| DaemonError::Internal(format!("spawn tool exec helper: {e}")))?;

    drop(spec_fd);
    drop(err_write);

    let exec_res = tokio::task::spawn_blocking(move || spec_pipe::read_exec_error(err_read)).await;
    match exec_res {
        Ok(Ok(None)) => {}
        Ok(Ok(Some(msg))) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(DaemonError::Internal(msg));
        }
        Ok(Err(e)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(e.into());
        }
        Err(e) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(DaemonError::join("tool exec check", e));
        }
    }

    let pid = child.id().unwrap_or(0);
    let pidfd = (pid > 0).then(|| PidFd::open(pid).ok()).flatten();
    let stdout_task = child
        .stdout
        .take()
        .map(|r| tokio::spawn(read_bounded_bytes(r, MAX_CAPTURE_BYTES)));
    let stderr_task = child
        .stderr
        .take()
        .map(|r| tokio::spawn(read_bounded_bytes(r, MAX_STDERR_BYTES)));

    let mut timed_out = false;
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(res) => res.ok(),
        Err(_) => {
            timed_out = true;
            if let Some(pfd) = &pidfd {
                let _ = pfd.send_signal(libc::SIGTERM);
                tokio::time::sleep(Duration::from_millis(300)).await;
                let _ = pfd.send_signal(libc::SIGKILL);
            } else {
                let _ = child.kill().await;
            }
            child.wait().await.ok()
        }
    };

    let stdout = collect_reader_task(stdout_task).await;
    let stderr = collect_reader_task(stderr_task).await;

    Ok(ExecOutput {
        status: status.and_then(|s| s.code()).unwrap_or(-1),
        stdout,
        stderr,
        timed_out,
    })
}

async fn collect_reader_task(handle: Option<tokio::task::JoinHandle<String>>) -> String {
    match handle {
        Some(mut h) => match tokio::time::timeout(Duration::from_secs(2), &mut h).await {
            Ok(Ok(text)) => text,
            _ => {
                h.abort();
                String::new()
            }
        },
        None => String::new(),
    }
}

async fn read_bounded_bytes<R: AsyncRead + Unpin>(mut reader: R, cap: usize) -> String {
    let mut out = Vec::new();
    let mut buf = [0_u8; 8192];
    while let Ok(n) = reader.read(&mut buf).await {
        if n == 0 {
            break;
        }
        if out.len() < cap {
            let room = cap - out.len();
            out.extend_from_slice(&buf[..n.min(room)]);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_assignment_round_trip_handles_exports_quotes_and_comments() {
        let raw = "\
# comment
export NPM_CONFIG_REGISTRY=\"https://registry.npmmirror.com\"
UV_DEFAULT_INDEX='https://pypi.tuna.tsinghua.edu.cn/simple'
export MISE_GITHUB_TOKEN=\"ghp_abc\\$123\"
INVALID KEY=foo
";
        let parsed = parse_env_assignments(raw);
        assert_eq!(
            parsed.get("NPM_CONFIG_REGISTRY").map(String::as_str),
            Some("https://registry.npmmirror.com")
        );
        assert_eq!(
            parsed.get("UV_DEFAULT_INDEX").map(String::as_str),
            Some("https://pypi.tuna.tsinghua.edu.cn/simple")
        );
        assert_eq!(
            parsed.get("MISE_GITHUB_TOKEN").map(String::as_str),
            Some("ghp_abc$123")
        );
        assert!(!parsed.contains_key("INVALID KEY"));
        assert_eq!(quote_posix_env("ghp_$a"), "\"ghp_\\$a\"");
    }
}
