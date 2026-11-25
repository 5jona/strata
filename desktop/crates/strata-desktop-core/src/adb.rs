//! ADB device discovery, local tunnel port allocation, `adb forward` lifecycle,
//! and root Bearer token retrieval.

mod parser;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::net::TcpListener;
use std::process::Command;

pub use parser::{
    AdbDevice, AdbDeviceState, AdbError, AdbTunnel, PortForwardEntry, parse_adb_devices_output,
    parse_adb_forward_list,
};

use crate::{DEFAULT_REMOTE_API_PORT, REMOTE_WEB_TOKEN_PATH};

/// Captured stdout/stderr output from an external command execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status_code: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Abstraction over `adb` process invocation enabling deterministic host unit tests.
pub trait CommandRunner: std::fmt::Debug {
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput, AdbError>;
}

/// Production runner executing the host `adb` binary via `std::process::Command`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput, AdbError> {
        let out = Command::new(program)
            .args(args)
            .output()
            .map_err(|source| AdbError::Spawn {
                program: program.to_owned(),
                source,
            })?;
        Ok(CommandOutput {
            status_code: out.status.code(),
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Allocates an ephemeral free TCP port on `127.0.0.1` for an `adb forward` tunnel.
pub fn allocate_loopback_port() -> Result<u16, AdbError> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(AdbError::PortAllocation)?;
    let port = listener
        .local_addr()
        .map_err(AdbError::PortAllocation)?
        .port();
    drop(listener);
    Ok(port)
}

/// Stateful manager tracking active `adb forward` tunnels per device serial.
#[derive(Debug)]
pub struct AdbManager<R: CommandRunner = SystemCommandRunner> {
    adb_bin: String,
    runner: R,
    tunnels: HashMap<String, AdbTunnel>,
    port_forwards: HashMap<(String, u16), PortForwardEntry>,
}

impl AdbManager<SystemCommandRunner> {
    /// Creates an `AdbManager` using `STRATA_ADB_BIN` or `adb` from `PATH`.
    #[must_use]
    pub fn system() -> Self {
        let adb_bin = std::env::var("STRATA_ADB_BIN").unwrap_or_else(|_| "adb".to_owned());
        Self::with_runner(adb_bin, SystemCommandRunner)
    }
}

impl<R: CommandRunner> AdbManager<R> {
    /// Creates an `AdbManager` with a custom command runner.
    #[must_use]
    pub fn with_runner(adb_bin: impl Into<String>, runner: R) -> Self {
        Self {
            adb_bin: adb_bin.into(),
            runner,
            tunnels: HashMap::new(),
            port_forwards: HashMap::new(),
        }
    }

    /// Discovers attached USB and wireless ADB devices.
    pub fn list_devices(&self) -> Result<Vec<AdbDevice>, AdbError> {
        let out = self.run_ok(&["devices", "-l"])?;
        Ok(parse_adb_devices_output(&out.stdout))
    }

    /// Ensures an `adb forward tcp:<local> tcp:<remote>` tunnel exists and attempts
    /// to auto-read the root `stratad` Bearer token via `su -c`.
    pub fn connect_device(
        &mut self,
        serial: &str,
        remote_port: Option<u16>,
        explicit_local_port: Option<u16>,
    ) -> Result<AdbTunnel, AdbError> {
        parser::validate_serial(serial)?;
        let remote = remote_port.unwrap_or(DEFAULT_REMOTE_API_PORT);
        if let Some(existing) = self.tunnels.get(serial)
            && existing.remote_port == remote
        {
            return Ok(existing.clone());
        }
        let local_port = match explicit_local_port {
            Some(p) if p > 0 => p,
            _ => allocate_loopback_port()?,
        };
        self.run_ok(&[
            "-s",
            serial,
            "forward",
            &format!("tcp:{local_port}"),
            &format!("tcp:{remote}"),
        ])?;
        let token = self.try_fetch_remote_token(serial);
        let tunnel = AdbTunnel {
            serial: serial.to_owned(),
            local_port,
            remote_port: remote,
            base_url: format!("http://127.0.0.1:{local_port}"),
            token,
        };
        self.tunnels.insert(serial.to_owned(), tunnel.clone());
        Ok(tunnel)
    }

    /// Tears down an active `adb forward` tunnel for `serial` if present.
    pub fn disconnect_device(&mut self, serial: &str) -> Result<bool, AdbError> {
        let Some(tunnel) = self.tunnels.remove(serial) else {
            return Ok(false);
        };
        let _ = self.run_ok(&[
            "-s",
            serial,
            "forward",
            "--remove",
            &format!("tcp:{}", tunnel.local_port),
        ]);
        Ok(true)
    }

    /// Returns all currently tracked ADB tunnels.
    #[must_use]
    pub fn active_tunnels(&self) -> Vec<AdbTunnel> {
        self.tunnels.values().cloned().collect()
    }

    /// Forwards an arbitrary container/device TCP port to `127.0.0.1:<local_port>` via `adb forward`.
    /// Reuses an existing tracked forward when `explicit_local_port` is omitted or matches.
    pub fn forward_device_port(
        &mut self,
        serial: &str,
        remote_port: u16,
        explicit_local_port: Option<u16>,
    ) -> Result<PortForwardEntry, AdbError> {
        parser::validate_serial(serial)?;
        if remote_port == 0 {
            return Err(AdbError::InvalidPort(remote_port));
        }
        let key = (serial.to_owned(), remote_port);
        if let Some(existing) = self.port_forwards.get(&key)
            && explicit_local_port.is_none_or(|p| p == 0 || p == existing.local_port)
        {
            return Ok(existing.clone());
        }
        let local_port = match explicit_local_port {
            Some(p) if p > 0 => p,
            _ => allocate_loopback_port()?,
        };
        let local_spec = format!("tcp:{local_port}");
        let remote_spec = format!("tcp:{remote_port}");
        self.run_ok(&["-s", serial, "forward", &local_spec, &remote_spec])?;
        let entry = PortForwardEntry {
            serial: serial.to_owned(),
            local_port,
            remote_port,
            local_spec,
            remote_spec,
        };
        self.port_forwards.insert(key, entry.clone());
        Ok(entry)
    }

    /// Lists active TCP `adb forward` rules, optionally filtered by `serial`.
    pub fn list_port_forwards(
        &self,
        serial: Option<&str>,
    ) -> Result<Vec<PortForwardEntry>, AdbError> {
        if let Some(s) = serial {
            parser::validate_serial(s)?;
        }
        let out = self.run_ok(&["forward", "--list"])?;
        Ok(parse_adb_forward_list(&out.stdout, serial))
    }

    /// Removes an `adb forward` rule bound to `tcp:<local_port>` on `serial`.
    pub fn remove_port_forward(&mut self, serial: &str, local_port: u16) -> Result<bool, AdbError> {
        parser::validate_serial(serial)?;
        if local_port == 0 {
            return Err(AdbError::InvalidPort(local_port));
        }
        self.run_ok(&[
            "-s",
            serial,
            "forward",
            "--remove",
            &format!("tcp:{local_port}"),
        ])?;
        self.port_forwards
            .retain(|(s, _), entry| !(s == serial && entry.local_port == local_port));
        if self
            .tunnels
            .get(serial)
            .is_some_and(|tunnel| tunnel.local_port == local_port)
        {
            self.tunnels.remove(serial);
        }
        Ok(true)
    }

    /// Best-effort root token fetch via `adb -s <serial> shell su -c 'cat /data/adb/strata/run/web.token'`.
    #[must_use]
    pub fn try_fetch_remote_token(&self, serial: &str) -> Option<String> {
        let cmd = format!("cat {REMOTE_WEB_TOKEN_PATH}");
        let args = vec![
            "-s".to_owned(),
            serial.to_owned(),
            "shell".to_owned(),
            "su".to_owned(),
            "-c".to_owned(),
            cmd,
        ];
        let out = self.runner.run(&self.adb_bin, &args).ok()?;
        if !out.success {
            return None;
        }
        let token = out.stdout.trim();
        if token.len() >= 16 && token.chars().all(|c| c.is_ascii_graphic()) {
            Some(token.to_owned())
        } else {
            None
        }
    }

    fn run_ok(&self, args: &[&str]) -> Result<CommandOutput, AdbError> {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let out = self.runner.run(&self.adb_bin, &owned)?;
        if out.success {
            Ok(out)
        } else {
            Err(AdbError::CommandFailed {
                code: out.status_code,
                stderr: out.stderr.trim().to_owned(),
            })
        }
    }
}
