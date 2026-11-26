//! Async Android host (`host-exec` / `nsenter`) and container command execution helpers.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub(crate) const HOST_EXEC_BIN: &str = "/usr/local/bin/host-exec";

/// Captured stdout/stderr and exit status from an external command.
#[derive(Debug, Clone)]
pub(crate) struct CmdOutput {
    pub(crate) code: i32,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) timed_out: bool,
}

impl CmdOutput {
    pub(crate) fn ok(&self) -> bool {
        !self.timed_out && self.code == 0
    }

    pub(crate) fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub(crate) fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Returns true when the in-container `host-exec` bridge exists.
pub(crate) fn host_exec_available() -> bool {
    Path::new(HOST_EXEC_BIN).exists()
}

const DEVNULL_WRAP: [&str; 4] = ["/system/bin/sh", "-c", "exec \"$@\" </dev/null", "_"];
const NSENTER_ARGS: [&str; 9] = ["-t", "1", "-m", "-u", "-C", "-T", "-r", "-w", "--"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostBackend {
    HostExec,
    DirectHost,
    Nsenter,
}

/// Returns true when running directly on Android host without container `host-exec`.
pub(crate) fn is_direct_host() -> bool {
    detect_host_backend() == HostBackend::DirectHost
}

fn detect_host_backend() -> HostBackend {
    if host_exec_available() {
        HostBackend::HostExec
    } else if Path::new("/system/bin/getprop").exists() {
        HostBackend::DirectHost
    } else {
        HostBackend::Nsenter
    }
}

/// Builds `(program, argv)` for executing a command inside Android PID 1 namespace.
/// When `has_stdin` is false on `host-exec` / `nsenter`, re-opens `/dev/null` inside
/// PID 1 mount namespace so `system_server` Binder services accept stdin under SELinux.
fn build_host_command_spec(
    backend: HostBackend,
    args: &[&str],
    has_stdin: bool,
) -> (String, Vec<String>) {
    let target: &[&str] = if args.is_empty() {
        &["/system/bin/true"]
    } else {
        args
    };
    let to_owned_vec = |prefix: &[&str]| -> Vec<String> {
        let wrap: &[&str] = if has_stdin { &[] } else { &DEVNULL_WRAP };
        prefix
            .iter()
            .chain(wrap.iter())
            .chain(target.iter())
            .map(|s| (*s).to_owned())
            .collect()
    };

    match backend {
        HostBackend::HostExec => (HOST_EXEC_BIN.to_owned(), to_owned_vec(&[])),
        HostBackend::DirectHost => (
            target[0].to_owned(),
            target[1..].iter().map(|s| (*s).to_owned()).collect(),
        ),
        HostBackend::Nsenter => ("nsenter".to_owned(), to_owned_vec(&NSENTER_ARGS)),
    }
}

/// Executes a command inside Android PID 1 namespace via `host-exec` or direct `nsenter`.
pub(crate) async fn run_host(args: &[&str], input: Option<&[u8]>, timeout: Duration) -> CmdOutput {
    let (prog, argv) = build_host_command_spec(detect_host_backend(), args, input.is_some());
    let mut cmd = Command::new(prog);
    cmd.args(argv);
    run_command(&mut cmd, input, timeout).await
}

/// Executes a command in the local container environment.
pub(crate) async fn run_local(
    program: &str,
    args: &[&str],
    input: Option<&[u8]>,
    timeout: Duration,
) -> CmdOutput {
    let mut cmd = Command::new(program);
    cmd.args(args);
    run_command(&mut cmd, input, timeout).await
}

async fn run_command(cmd: &mut Command, input: Option<&[u8]>, timeout: Duration) -> CmdOutput {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    cmd.kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            return CmdOutput {
                code: 127,
                stdout: Vec::new(),
                stderr: format!("spawn failed: {err}").into_bytes(),
                timed_out: false,
            };
        }
    };

    if let Some(bytes) = input
        && let Some(mut stdin) = child.stdin.take()
    {
        let _ = stdin.write_all(bytes).await;
        let _ = stdin.shutdown().await;
    }

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => CmdOutput {
            code: out.status.code().unwrap_or(1),
            stdout: out.stdout,
            stderr: out.stderr,
            timed_out: false,
        },
        Ok(Err(err)) => CmdOutput {
            code: 1,
            stdout: Vec::new(),
            stderr: format!("wait failed: {err}").into_bytes(),
            timed_out: false,
        },
        Err(_) => CmdOutput {
            code: 124,
            stdout: Vec::new(),
            stderr: format!("command timed out after {}ms", timeout.as_millis()).into_bytes(),
            timed_out: true,
        },
    }
}

/// Executes a shell command in either `"host"` or `"container"` scope.
pub(crate) async fn exec_shell(scope: &str, command: &str, timeout_sec: f64) -> Value {
    let dur = Duration::from_secs_f64(timeout_sec.clamp(1.0, 120.0));
    let out = if scope == "container" {
        run_local("/bin/sh", &["-c", command], None, dur).await
    } else {
        run_host(&["/system/bin/sh", "-c", command], None, dur).await
    };
    json!({
        "ok": out.ok(),
        "scope": scope,
        "rc": out.code,
        "timedOut": out.timed_out,
        "stdout": tail_chars(&out.stdout_text(), 16_000),
        "stderr": tail_chars(&out.stderr_text(), 4_000),
    })
}

/// Truncates a string to its trailing `max_chars` characters on valid UTF-8 boundaries.
pub(crate) fn tail_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    text.chars().skip(count - max_chars).collect()
}

/// Atomically writes bytes to `path` via sibling temp file + fsync + rename.
pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let parent = path.parent().unwrap_or_else(|| Path::new("/tmp"));
    std::fs::create_dir_all(parent)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(".strata-tmp-{}-{stamp}", std::process::id()));
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_chars_preserves_utf8_boundaries() {
        let s = "hello世界rust";
        assert_eq!(tail_chars(s, 6), "世界rust");
        assert_eq!(tail_chars(s, 50), s);
    }

    #[test]
    fn atomic_write_replaces_file_safely() {
        let dir = std::env::temp_dir().join(format!("strata-dbg-atomic-{}", std::process::id()));
        let path = dir.join("state.json");
        atomic_write(&path, b"{\"v\":1}").expect("write v1");
        atomic_write(&path, b"{\"v\":2}").expect("write v2");
        let got = std::fs::read_to_string(&path).expect("read");
        assert_eq!(got, "{\"v\":2}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn local_shell_exec_captures_stdout_and_rc() {
        let res = exec_shell("container", "printf 'ok-123'", 5.0).await;
        assert_eq!(res["ok"], json!(true));
        assert_eq!(res["rc"], json!(0));
        assert_eq!(res["stdout"], json!("ok-123"));
    }

    #[test]
    fn host_command_spec_redirects_dev_null_only_without_stdin_pipe() {
        let (prog, argv) =
            build_host_command_spec(HostBackend::HostExec, &["/system/bin/wm", "size"], false);
        assert_eq!(prog, HOST_EXEC_BIN);
        assert_eq!(&argv[..4], DEVNULL_WRAP);
        assert_eq!(&argv[4..], ["/system/bin/wm", "size"]);

        let (_, piped_argv) = build_host_command_spec(
            HostBackend::HostExec,
            &["/system/bin/perfetto", "-c", "-"],
            true,
        );
        assert_eq!(piped_argv, ["/system/bin/perfetto", "-c", "-"]);

        let (ns_prog, ns_argv) = build_host_command_spec(
            HostBackend::Nsenter,
            &["/system/bin/input", "tap", "10", "20"],
            false,
        );
        assert_eq!(ns_prog, "nsenter");
        assert_eq!(&ns_argv[..9], NSENTER_ARGS);
        assert_eq!(&ns_argv[9..13], DEVNULL_WRAP);
        assert_eq!(&ns_argv[13..], ["/system/bin/input", "tap", "10", "20"]);

        let (_, ns_piped) = build_host_command_spec(
            HostBackend::Nsenter,
            &["/system/bin/perfetto", "-c", "-"],
            true,
        );
        assert_eq!(&ns_piped[9..], ["/system/bin/perfetto", "-c", "-"]);
    }
}
