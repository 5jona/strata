//! Cross-platform native terminal SSH launcher, VS Code Remote-SSH URI builder,
//! and `~/.ssh/config` snippet formatter.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Supported native desktop terminal emulators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeTerminalKind {
    WindowsTerminal,
    Ghostty,
    WezTerm,
    Alacritty,
    ITerm2,
    XTerminalEmulator,
}

/// Target SSH coordinates for a Strata container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTarget {
    pub user: String,
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// Ready-to-spawn native command specification (zero shell interpolation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Validation errors for external launch arguments.
#[derive(Debug, Error)]
pub enum LauncherError {
    #[error("invalid SSH user `{0}`")]
    InvalidUser(String),
    #[error("invalid SSH host `{0}`")]
    InvalidHost(String),
    #[error("invalid SSH port 0")]
    InvalidPort,
    #[error("invalid remote path `{0}`")]
    InvalidRemotePath(String),
}

/// Validates SSH target fields to prevent argument/URI injection.
pub fn validate_ssh_target(target: &SshTarget) -> Result<(), LauncherError> {
    if target.user.is_empty()
        || target.user.len() > 64
        || !target
            .user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(LauncherError::InvalidUser(target.user.clone()));
    }
    if target.host.is_empty()
        || target.host.len() > 253
        || target.host.starts_with('-')
        || !target
            .host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
    {
        return Err(LauncherError::InvalidHost(target.host.clone()));
    }
    if target.port == 0 {
        return Err(LauncherError::InvalidPort);
    }
    if let Some(path) = &target.remote_path
        && (!path.starts_with('/') || path.chars().any(|c| c.is_control() || c == '?' || c == '#'))
    {
        return Err(LauncherError::InvalidRemotePath(path.clone()));
    }
    Ok(())
}

/// Builds a `vscode://vscode-remote/ssh-remote+<user>@<host>:<port><path>` deep link.
pub fn build_vscode_remote_uri(target: &SshTarget) -> Result<String, LauncherError> {
    validate_ssh_target(target)?;
    let path = target.remote_path.as_deref().unwrap_or("/root");
    Ok(format!(
        "vscode://vscode-remote/ssh-remote+{}@{}:{}{}",
        target.user, target.host, target.port, path
    ))
}

/// Builds a copyable `~/.ssh/config` host block with `ControlMaster` multiplexing.
pub fn build_ssh_config_snippet(target: &SshTarget) -> Result<String, LauncherError> {
    validate_ssh_target(target)?;
    let alias = target
        .alias
        .as_deref()
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
        .unwrap_or("strata-main");
    Ok(format!(
        "Host {alias}\n    HostName {host}\n    Port {port}\n    User {user}\n    ServerAliveInterval 30\n    ControlMaster auto\n    ControlPath ~/.ssh/cm-%r@%h:%p\n    ControlPersist 600\n",
        host = target.host,
        port = target.port,
        user = target.user,
    ))
}

/// Builds an argv-safe native terminal launch command for `ssh` into a container.
pub fn build_terminal_launch_command(
    kind: NativeTerminalKind,
    target: &SshTarget,
) -> Result<LaunchCommand, LauncherError> {
    validate_ssh_target(target)?;
    let dest = format!("{}@{}", target.user, target.host);
    let port = target.port.to_string();

    let (program, args) = match kind {
        NativeTerminalKind::WindowsTerminal => (
            "wt.exe".to_owned(),
            vec![
                "new-tab".to_owned(),
                "--title".to_owned(),
                format!("Strata ({dest})"),
                "ssh".to_owned(),
                "-p".to_owned(),
                port,
                dest,
            ],
        ),
        NativeTerminalKind::Ghostty => (
            "ghostty".to_owned(),
            vec![
                "-e".to_owned(),
                "ssh".to_owned(),
                "-p".to_owned(),
                port,
                dest,
            ],
        ),
        NativeTerminalKind::WezTerm => (
            "wezterm".to_owned(),
            vec![
                "start".to_owned(),
                "--".to_owned(),
                "ssh".to_owned(),
                "-p".to_owned(),
                port,
                dest,
            ],
        ),
        NativeTerminalKind::Alacritty => (
            "alacritty".to_owned(),
            vec![
                "-e".to_owned(),
                "ssh".to_owned(),
                "-p".to_owned(),
                port,
                dest,
            ],
        ),
        NativeTerminalKind::ITerm2 => (
            "open".to_owned(),
            vec![
                "-a".to_owned(),
                "iTerm".to_owned(),
                format!("ssh://{dest}:{port}"),
            ],
        ),
        NativeTerminalKind::XTerminalEmulator => (
            "x-terminal-emulator".to_owned(),
            vec![
                "-e".to_owned(),
                "ssh".to_owned(),
                "-p".to_owned(),
                port,
                dest,
            ],
        ),
    };
    Ok(LaunchCommand { program, args })
}

/// Returns platform-appropriate terminal launcher candidates in preference order.
#[must_use]
pub fn default_terminal_candidates() -> Vec<NativeTerminalKind> {
    if cfg!(target_os = "windows") {
        vec![
            NativeTerminalKind::WindowsTerminal,
            NativeTerminalKind::WezTerm,
            NativeTerminalKind::Alacritty,
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            NativeTerminalKind::Ghostty,
            NativeTerminalKind::WezTerm,
            NativeTerminalKind::ITerm2,
            NativeTerminalKind::Alacritty,
        ]
    } else {
        vec![
            NativeTerminalKind::Ghostty,
            NativeTerminalKind::WezTerm,
            NativeTerminalKind::Alacritty,
            NativeTerminalKind::XTerminalEmulator,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_target() -> SshTarget {
        SshTarget {
            user: "root".to_owned(),
            host: "192.0.2.10".to_owned(),
            port: 2222,
            remote_path: Some("/root/workspace".to_owned()),
            alias: Some("strata-tablet".to_owned()),
        }
    }

    #[test]
    fn builds_vscode_uri_and_ssh_config() {
        let uri = build_vscode_remote_uri(&sample_target()).unwrap();
        assert_eq!(
            uri,
            "vscode://vscode-remote/ssh-remote+root@192.0.2.10:2222/root/workspace"
        );

        let cfg = build_ssh_config_snippet(&sample_target()).unwrap();
        assert!(cfg.contains("Host strata-tablet"));
        assert!(cfg.contains("Port 2222"));
        assert!(cfg.contains("ControlMaster auto"));
    }

    #[test]
    fn builds_terminal_commands_without_shell_injection() {
        let cmd =
            build_terminal_launch_command(NativeTerminalKind::WindowsTerminal, &sample_target())
                .unwrap();
        assert_eq!(cmd.program, "wt.exe");
        assert!(cmd.args.contains(&"root@192.0.2.10".to_owned()));

        let ghostty =
            build_terminal_launch_command(NativeTerminalKind::Ghostty, &sample_target()).unwrap();
        assert_eq!(ghostty.program, "ghostty");
        assert_eq!(
            ghostty.args,
            vec!["-e", "ssh", "-p", "2222", "root@192.0.2.10"]
        );
    }

    #[test]
    fn rejects_unsafe_host_and_path() {
        let mut bad = sample_target();
        bad.host = "-oProxyCommand=calc".to_owned();
        assert!(build_vscode_remote_uri(&bad).is_err());

        let mut bad_path = sample_target();
        bad_path.remote_path = Some("relative/path".to_owned());
        assert!(build_vscode_remote_uri(&bad_path).is_err());
    }
}
