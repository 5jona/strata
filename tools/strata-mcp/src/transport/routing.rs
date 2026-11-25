//! Transport route construction, SSH ControlMaster argument building, and ADB forward setup.

use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

use crate::devices::{
    DeviceRecord, DeviceRegistry, device_control_path, device_known_hosts_path, is_ip_literal,
};

/// Single candidate transport path (`ssh` or `adb-ssh`) to reach a device container.
#[derive(Debug, Clone)]
pub(crate) struct TransportRoute {
    pub(crate) kind: &'static str,
    pub(crate) host: String,
    pub(crate) port: Option<u16>,
    pub(crate) user: Option<String>,
    pub(crate) control_path: PathBuf,
    pub(crate) known_hosts_path: PathBuf,
    pub(crate) adb_serial: Option<String>,
    pub(crate) local_port: Option<u16>,
    pub(crate) remote_ssh_port: u16,
}

impl TransportRoute {
    pub(crate) fn target_spec(&self) -> String {
        match self.user.as_deref().filter(|u| !u.is_empty()) {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        }
    }

    pub(crate) fn base_ssh_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(p) = self.port {
            args.push("-p".to_owned());
            args.push(p.to_string());
        }
        args.extend([
            "-o".to_owned(),
            "BatchMode=yes".to_owned(),
            "-o".to_owned(),
            "ConnectTimeout=8".to_owned(),
            "-o".to_owned(),
            "ServerAliveInterval=15".to_owned(),
            "-o".to_owned(),
            "ServerAliveCountMax=2".to_owned(),
            "-o".to_owned(),
            "StrictHostKeyChecking=accept-new".to_owned(),
            "-o".to_owned(),
            "ControlMaster=auto".to_owned(),
            "-o".to_owned(),
            "ControlPersist=300".to_owned(),
            "-o".to_owned(),
            format!("ControlPath={}", self.control_path.display()),
            "-o".to_owned(),
            format!("UserKnownHostsFile={}", self.known_hosts_path.display()),
            self.target_spec(),
        ]);
        args
    }
}

/// Optional per-call target routing overrides passed by MCP tools.
#[derive(Debug, Clone, Default)]
pub(crate) struct TargetOverride {
    pub(crate) device: Option<String>,
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
}

/// Builds ordered failover transport routes for a tool invocation.
pub(crate) async fn build_routes(
    registry: &DeviceRegistry,
    target: &TargetOverride,
) -> Result<Vec<TransportRoute>, String> {
    if let Some(rec) = registry
        .resolve(target.device.as_deref(), target.host.as_deref())
        .await?
    {
        return routes_for_record(registry, &rec, target.port).await;
    }

    let eff_host = target
        .host
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            std::env::var("STRATA_SSH_HOST")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .ok_or_else(|| "no device discovered and STRATA_SSH_HOST is not set".to_owned())?;
    let eff_port = target
        .port
        .or_else(|| {
            std::env::var("STRATA_SSH_PORT")
                .ok()
                .and_then(|s| s.trim().parse::<u16>().ok())
        })
        .or_else(|| is_ip_literal(&eff_host).then_some(2222));
    let eff_user = std::env::var("STRATA_SSH_USER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| is_ip_literal(&eff_host).then(|| "root".to_owned()));
    let key = format!("host-{eff_host}-{}", eff_port.unwrap_or(0));
    Ok(vec![TransportRoute {
        kind: "ssh",
        host: eff_host,
        port: eff_port,
        user: eff_user,
        control_path: device_control_path(&key),
        known_hosts_path: device_known_hosts_path(&key),
        adb_serial: None,
        local_port: None,
        remote_ssh_port: eff_port.unwrap_or(2222),
    }])
}

async fn routes_for_record(
    registry: &DeviceRegistry,
    rec: &DeviceRecord,
    port_override: Option<u16>,
) -> Result<Vec<TransportRoute>, String> {
    let ctl = device_control_path(&rec.id);
    let kh = device_known_hosts_path(&rec.id);
    let mut ssh_route = None;
    let mut adb_route = None;

    if let Some(host) = rec.ssh_host.as_ref() {
        let p = port_override
            .or(rec.ssh_port)
            .or_else(|| is_ip_literal(host).then_some(2222));
        let u = rec
            .ssh_user
            .clone()
            .or_else(|| is_ip_literal(host).then(|| "root".to_owned()));
        ssh_route = Some(TransportRoute {
            kind: "ssh",
            host: host.clone(),
            port: p,
            user: u,
            control_path: ctl.clone(),
            known_hosts_path: kh.clone(),
            adb_serial: None,
            local_port: None,
            remote_ssh_port: p.unwrap_or(2222),
        });
    }

    let serial = rec
        .adb_usb_serial
        .as_deref()
        .or(rec.adb_tcpip_serial.as_deref())
        .or(rec.adb_serial.as_deref());
    if let Some(ser) = serial
        && rec.adb_state.as_deref() == Some("device")
    {
        let local_p = registry.allocate_forward_port(&rec.id).await?;
        let remote_p = port_override.or(rec.ssh_port).unwrap_or(2222);
        adb_route = Some(TransportRoute {
            kind: "adb-ssh",
            host: "127.0.0.1".to_owned(),
            port: Some(local_p),
            user: Some(rec.ssh_user.clone().unwrap_or_else(|| "root".to_owned())),
            control_path: ctl,
            known_hosts_path: kh,
            adb_serial: Some(ser.to_owned()),
            local_port: Some(local_p),
            remote_ssh_port: remote_p,
        });
    }

    let mut routes = Vec::new();
    if rec.prefer_transport == "adb" {
        if let Some(r) = adb_route {
            routes.push(r);
        }
        if let Some(r) = ssh_route {
            routes.push(r);
        }
    } else {
        if let Some(r) = ssh_route {
            routes.push(r);
        }
        if let Some(r) = adb_route {
            routes.push(r);
        }
    }
    if routes.is_empty() {
        return Err(format!(
            "device '{}' has no usable SSH or online ADB transport",
            rec.id
        ));
    }
    Ok(routes)
}

/// Ensures `adb -s <serial> forward tcp:<local> tcp:<remote>` is active.
pub(crate) async fn ensure_adb_forward(
    serial: &str,
    local_port: u16,
    remote_port: u16,
) -> Result<(), String> {
    let out = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("adb")
            .args([
                "-s",
                serial,
                "forward",
                &format!("tcp:{local_port}"),
                &format!("tcp:{remote_port}"),
            ])
            .output(),
    )
    .await
    .map_err(|_| format!("adb forward timed out for {serial}"))?
    .map_err(|e| format!("failed to invoke adb: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "adb forward failed for {serial}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Closes a stale SSH ControlMaster socket cleanly.
pub(crate) async fn reset_control_socket(route: &TransportRoute) {
    let _ = tokio::time::timeout(
        Duration::from_secs(4),
        Command::new("ssh")
            .args([
                "-o",
                &format!("ControlPath={}", route.control_path.display()),
                "-O",
                "exit",
                &route.target_spec(),
            ])
            .output(),
    )
    .await;
    let _ = std::fs::remove_file(&route.control_path);
}

/// POSIX single-quote escaping preventing shell injection when composing remote SSH commands.
pub(crate) fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':' | b'=' | b',')
        })
    {
        return arg.to_owned();
    }
    format!("'{}'", arg.replace('\'', "'\"'\"'"))
}

/// Safely formats an argument vector into a POSIX shell command line.
pub(crate) fn argv_to_shell(argv: &[String]) -> String {
    argv.iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_shell_arguments_against_injection() {
        let cmd = argv_to_shell(&[
            "strata-debug".to_owned(),
            "cdp".to_owned(),
            "eval".to_owned(),
            "document.title; $(rm -rf /); 'x'".to_owned(),
        ]);
        assert!(cmd.contains("'document.title; $(rm -rf /); '\"'\"'x'\"'\"''"));
    }
}
