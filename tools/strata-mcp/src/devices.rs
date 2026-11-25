//! Cached device registry, session device selection, and deterministic port allocation.

pub(crate) mod discovery;
mod merge;
mod paths;

use std::collections::{HashMap, HashSet};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::sync::Mutex;

pub(crate) use discovery::{DeviceRecord, is_ip_literal, parse_adb_devices_output};
use merge::merge_devices;
pub(crate) use paths::{device_control_path, device_known_hosts_path};
use paths::{fnv1a, load_devices_config, load_ssh_entries};

const FORWARD_PORT_BASE: u16 = 12222;
const FORWARD_PORT_COUNT: u16 = 78;
const DISCOVERY_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
struct RegistryState {
    selected_id: Option<String>,
    allocated_ports: HashMap<String, u16>,
    cached: Option<(Instant, Vec<DeviceRecord>)>,
}

/// Thread-safe multi-device discovery cache and session selector.
#[derive(Debug, Default)]
pub(crate) struct DeviceRegistry {
    state: Mutex<RegistryState>,
}

impl DeviceRegistry {
    /// Discovers devices (using a 5s TTL cache unless `force_refresh` is true).
    pub(crate) async fn discover(&self, force_refresh: bool) -> Vec<DeviceRecord> {
        {
            let guard = self.state.lock().await;
            if !force_refresh
                && let Some((ts, ref list)) = guard.cached
                && ts.elapsed() < DISCOVERY_TTL
            {
                return list.clone();
            }
        }
        let fresh = discover_live().await;
        let mut guard = self.state.lock().await;
        guard.cached = Some((Instant::now(), fresh.clone()));
        fresh
    }

    pub(crate) async fn selected_id(&self) -> Option<String> {
        self.state.lock().await.selected_id.clone()
    }

    /// Selects or clears (`None`/empty) the session default target device.
    pub(crate) async fn select(
        &self,
        device_ref: Option<&str>,
    ) -> Result<Option<DeviceRecord>, String> {
        let Some(raw) = device_ref.map(str::trim).filter(|s| !s.is_empty()) else {
            self.state.lock().await.selected_id = None;
            return Ok(None);
        };
        let all = self.discover(true).await;
        let matches = match_devices(&all, raw);
        match matches.as_slice() {
            [] => Err(format!(
                "device '{raw}' not found. Available: {}",
                format_available(&all)
            )),
            [only] => {
                self.state.lock().await.selected_id = Some(only.id.clone());
                Ok(Some(only.clone()))
            }
            many => Err(format!(
                "ambiguous device '{raw}' matches: {}",
                format_available(many)
            )),
        }
    }

    /// Resolves target device with fail-fast ambiguity rejection when multiple devices are online.
    pub(crate) async fn resolve(
        &self,
        device: Option<&str>,
        host: Option<&str>,
    ) -> Result<Option<DeviceRecord>, String> {
        if host.is_some_and(|h| !h.trim().is_empty()) && device.is_none_or(|d| d.trim().is_empty())
        {
            return Ok(None);
        }
        let all = self.discover(false).await;
        if let Some(d) = device.map(str::trim).filter(|s| !s.is_empty()) {
            let matches = match_devices(&all, d);
            return match matches.as_slice() {
                [] => Err(format!(
                    "device '{d}' not found. Available: {}",
                    format_available(&all)
                )),
                [only] => Ok(Some(only.clone())),
                many => Err(format!(
                    "ambiguous device '{d}' matches: {}",
                    format_available(many)
                )),
            };
        }
        if let Some(sel) = self.selected_id().await {
            let matches = match_devices(&all, &sel);
            return matches.into_iter().next().map(Some).ok_or_else(|| {
                format!(
                    "selected device '{sel}' is offline; pass 'device' or call device_manage(action='select'). Available: {}",
                    format_available(&all)
                )
            });
        }
        let avail: Vec<DeviceRecord> = all.into_iter().filter(DeviceRecord::available).collect();
        if avail.len() > 1 {
            return Err(format!(
                "multiple devices online ({}): {}. Specify 'device' or call device_manage(action='select').",
                avail.len(),
                format_available(&avail)
            ));
        }
        Ok(avail.into_iter().next())
    }

    /// Allocates a deterministic free loopback port (`12222..=12299`) for ADB SSH forwarding.
    pub(crate) async fn allocate_forward_port(&self, device_key: &str) -> Result<u16, String> {
        let mut guard = self.state.lock().await;
        if let Some(&p) = guard.allocated_ports.get(device_key) {
            return Ok(p);
        }
        let used: HashSet<u16> = guard.allocated_ports.values().copied().collect();
        let start = (fnv1a(device_key) % u64::from(FORWARD_PORT_COUNT)) as u16;
        for off in 0..FORWARD_PORT_COUNT {
            let cand = FORWARD_PORT_BASE + ((start + off) % FORWARD_PORT_COUNT);
            if used.contains(&cand) || port_in_use(cand) {
                continue;
            }
            guard.allocated_ports.insert(device_key.to_owned(), cand);
            return Ok(cand);
        }
        Err("no free local ADB forward port in range 12222..12299".to_owned())
    }
}

pub(crate) fn match_devices(records: &[DeviceRecord], needle: &str) -> Vec<DeviceRecord> {
    let exact: Vec<DeviceRecord> = records
        .iter()
        .filter(|r| {
            r.id == needle
                || r.alias.as_deref() == Some(needle)
                || r.adb_serial.as_deref() == Some(needle)
                || r.adb_usb_serial.as_deref() == Some(needle)
                || r.adb_tcpip_serial.as_deref() == Some(needle)
                || r.ssh_host.as_deref() == Some(needle)
        })
        .cloned()
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    let low = needle.to_ascii_lowercase();
    records
        .iter()
        .filter(|r| {
            [
                Some(r.id.as_str()),
                r.alias.as_deref(),
                r.adb_serial.as_deref(),
                r.ssh_host.as_deref(),
                r.model.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|v| v.eq_ignore_ascii_case(&low))
        })
        .cloned()
        .collect()
}

fn format_available(records: &[DeviceRecord]) -> String {
    if records.is_empty() {
        return "none".to_owned();
    }
    records
        .iter()
        .map(|r| {
            format!(
                "{} (model={}, transport={})",
                r.id,
                r.model.as_deref().unwrap_or("unknown"),
                r.transport_summary()
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

async fn discover_live() -> Vec<DeviceRecord> {
    let cfg = load_devices_config();
    let extra_hosts: HashSet<String> = cfg
        .values()
        .filter_map(|v| v.ssh_host.as_deref().map(str::to_ascii_lowercase))
        .collect();
    let ssh_list = load_ssh_entries(&extra_hosts);
    let adb_list = match tokio::time::timeout(
        Duration::from_secs(5),
        Command::new("adb").args(["devices", "-l"]).output(),
    )
    .await
    {
        Ok(Ok(out)) if out.status.success() => {
            parse_adb_devices_output(&String::from_utf8_lossy(&out.stdout))
        }
        _ => Vec::new(),
    };
    merge_devices(&cfg, &adb_list, &ssh_list)
}

fn port_in_use(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(50),
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use paths::safe_device_key;

    #[test]
    fn matches_device_case_insensitively_and_sanitizes_key() {
        let recs = vec![DeviceRecord {
            id: "demo-tab".to_owned(),
            alias: Some("demo-tab".to_owned()),
            model: Some("OPD2513".to_owned()),
            product: None,
            adb_serial: Some("SER123".to_owned()),
            adb_state: Some("device".to_owned()),
            adb_transports: vec!["usb".to_owned()],
            adb_usb_serial: Some("SER123".to_owned()),
            adb_tcpip_serial: None,
            ssh_host: Some("demo-tab".to_owned()),
            ssh_hostname: None,
            ssh_port: Some(2222),
            ssh_user: Some("root".to_owned()),
            prefer_transport: "ssh".to_owned(),
            sources: vec!["adb".to_owned()],
        }];
        assert_eq!(match_devices(&recs, "opd2513").len(), 1);
        let key = safe_device_key("192.0.2.10:2222/foo");
        assert!(!key.contains('/') && !key.contains(':'));
    }
}
