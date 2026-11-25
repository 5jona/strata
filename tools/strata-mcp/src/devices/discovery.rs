//! Multi-source Android/Strata device parsers (`adb devices -l`, `~/.ssh/config`, `devices.json`).

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub(crate) struct AdbEntry {
    pub(crate) serial: String,
    pub(crate) state: String,
    pub(crate) transport: String,
    pub(crate) product: Option<String>,
    pub(crate) model: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SshEntry {
    pub(crate) host: String,
    pub(crate) hostname: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) user: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigDevice {
    pub(crate) adb_serial: Option<String>,
    pub(crate) ssh_host: Option<String>,
    pub(crate) ssh_port: Option<u16>,
    pub(crate) ssh_user: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) prefer_transport: Option<String>,
}

/// Correlated device record merging ADB, SSH config, and optional `devices.json`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceRecord {
    pub(crate) id: String,
    pub(crate) alias: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) product: Option<String>,
    pub(crate) adb_serial: Option<String>,
    pub(crate) adb_state: Option<String>,
    pub(crate) adb_transports: Vec<String>,
    pub(crate) adb_usb_serial: Option<String>,
    pub(crate) adb_tcpip_serial: Option<String>,
    pub(crate) ssh_host: Option<String>,
    pub(crate) ssh_hostname: Option<String>,
    pub(crate) ssh_port: Option<u16>,
    pub(crate) ssh_user: Option<String>,
    pub(crate) prefer_transport: String,
    pub(crate) sources: Vec<String>,
}

impl DeviceRecord {
    pub(crate) fn available(&self) -> bool {
        self.adb_state.as_deref() == Some("device") || self.ssh_host.is_some()
    }

    pub(crate) fn transport_summary(&self) -> String {
        let mut parts = Vec::new();
        if self.adb_usb_serial.is_some() && self.adb_state.as_deref() == Some("device") {
            parts.push("usb");
        }
        if self.adb_tcpip_serial.is_some() && self.adb_state.as_deref() == Some("device") {
            parts.push("adb-tcpip");
        }
        if self.ssh_host.is_some() {
            parts.push("ssh");
        }
        if parts.is_empty() {
            self.adb_state
                .clone()
                .unwrap_or_else(|| "offline".to_owned())
        } else {
            parts.join("+")
        }
    }

    pub(crate) fn to_public_json(&self, selected_id: Option<&str>) -> Value {
        json!({
            "id": self.id,
            "alias": self.alias,
            "model": self.model,
            "serial": self.adb_serial,
            "usb": self.adb_usb_serial.is_some() && self.adb_state.as_deref() == Some("device"),
            "wireless": self.ssh_host.is_some() || (self.adb_tcpip_serial.is_some() && self.adb_state.as_deref() == Some("device")),
            "adbState": self.adb_state,
            "adbTransports": self.adb_transports,
            "sshHost": self.ssh_host,
            "sshPort": self.ssh_port,
            "sshUser": self.ssh_user,
            "preferTransport": self.prefer_transport,
            "selected": selected_id == Some(self.id.as_str()),
            "sources": self.sources,
        })
    }
}

pub(crate) fn is_ip_literal(s: &str) -> bool {
    s.trim_matches(|c| c == '[' || c == ']')
        .parse::<IpAddr>()
        .is_ok()
}

pub(crate) fn tcpip_ip(serial: &str) -> Option<String> {
    let (host, _) = serial.rsplit_once(':')?;
    let clean = host.trim_matches(|c| c == '[' || c == ']');
    is_ip_literal(clean).then(|| clean.to_owned())
}

/// Parses `adb devices -l` text output into structured entries.
pub(crate) fn parse_adb_devices_output(text: &str) -> Vec<AdbEntry> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("List of devices") || line.starts_with('*') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(serial), Some(state)) = (parts.next(), parts.next()) else {
            continue;
        };
        let mut kv = HashMap::new();
        for tok in parts {
            if let Some((k, v)) = tok.split_once(':') {
                kv.insert(k, v);
            }
        }
        let transport = if serial.contains(':') || serial.contains("._adb-tls-") {
            "tcpip"
        } else {
            "usb"
        };
        out.push(AdbEntry {
            serial: serial.to_owned(),
            state: state.to_owned(),
            transport: transport.to_owned(),
            product: kv.get("product").map(|s| (*s).to_owned()),
            model: kv.get("model").map(|s| (*s).to_owned()),
        });
    }
    out
}

#[derive(Default)]
struct SshBlock {
    hosts: Vec<String>,
    hostname: Option<String>,
    port: Option<u16>,
    user: Option<String>,
}

/// Parses `~/.ssh/config` keeping Strata-relevant entries (port 2222, default env host, or configured hosts).
pub(crate) fn parse_ssh_config_text(text: &str, extra_hosts: &HashSet<String>) -> Vec<SshEntry> {
    let env_host = std::env::var("STRATA_SSH_HOST")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase());
    let mut blocks: Vec<SshBlock> = Vec::new();
    let mut cur: Option<SshBlock> = None;

    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() {
            continue;
        }
        if key.eq_ignore_ascii_case("host") {
            if let Some(b) = cur.take() {
                blocks.push(b);
            }
            let hosts = rest
                .into_iter()
                .filter(|h| !h.contains('*') && !h.contains('?') && !h.starts_with('!'))
                .map(str::to_owned)
                .collect();
            cur = Some(SshBlock {
                hosts,
                ..SshBlock::default()
            });
        } else if let Some(b) = cur.as_mut() {
            let val = rest[0];
            if key.eq_ignore_ascii_case("hostname") {
                b.hostname = Some(val.to_owned());
            } else if key.eq_ignore_ascii_case("port") {
                b.port = val.parse::<u16>().ok();
            } else if key.eq_ignore_ascii_case("user") {
                b.user = Some(val.to_owned());
            }
        }
    }
    if let Some(b) = cur {
        blocks.push(b);
    }

    let mut entries = Vec::new();
    for b in blocks {
        for h in b.hosts {
            let low = h.to_ascii_lowercase();
            let keep = b.port == Some(2222)
                || extra_hosts.contains(&low)
                || env_host.as_deref() == Some(low.as_str());
            if keep {
                entries.push(SshEntry {
                    host: h,
                    hostname: b.hostname.clone(),
                    port: b.port,
                    user: b.user.clone(),
                });
            }
        }
    }
    entries
}

pub(crate) fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}
