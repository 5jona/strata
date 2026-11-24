//! Data structures and `adb devices -l` stdout parser for ADB device discovery.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Connection state reported by `adb devices -l`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdbDeviceState {
    Device,
    Offline,
    Unauthorized,
    Recovery,
    Sideload,
    Unknown(String),
}

/// One device row parsed from `adb devices -l`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdbDevice {
    pub serial: String,
    pub state: AdbDeviceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_id: Option<String>,
    pub is_wireless: bool,
}

/// Active `adb forward tcp:<local> tcp:<remote>` tunnel handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdbTunnel {
    pub serial: String,
    pub local_port: u16,
    pub remote_port: u16,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Errors produced by ADB discovery and tunnel operations.
#[derive(Debug, Error)]
pub enum AdbError {
    #[error("failed to execute `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("adb command failed (exit {code:?}): {stderr}")]
    CommandFailed { code: Option<i32>, stderr: String },
    #[error("failed to allocate free loopback port: {0}")]
    PortAllocation(#[source] std::io::Error),
    #[error("invalid ADB serial `{0}`")]
    InvalidSerial(String),
}

/// Parses `adb devices -l` stdout into structured device descriptors.
#[must_use]
pub fn parse_adb_devices_output(stdout: &str) -> Vec<AdbDevice> {
    let mut devices = Vec::new();
    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if line.is_empty()
            || line.starts_with("List of devices")
            || line.starts_with('*')
            || line.starts_with("adb server")
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(serial) = parts.next() else {
            continue;
        };
        let Some(state_str) = parts.next() else {
            continue;
        };
        let state = match state_str {
            "device" => AdbDeviceState::Device,
            "offline" => AdbDeviceState::Offline,
            "unauthorized" => AdbDeviceState::Unauthorized,
            "recovery" => AdbDeviceState::Recovery,
            "sideload" => AdbDeviceState::Sideload,
            other => AdbDeviceState::Unknown(other.to_owned()),
        };

        let mut product = None;
        let mut model = None;
        let mut device = None;
        let mut transport_id = None;
        for token in parts {
            if let Some((k, v)) = token.split_once(':') {
                match k {
                    "product" => product = Some(v.to_owned()),
                    "model" => model = Some(v.to_owned()),
                    "device" => device = Some(v.to_owned()),
                    "transport_id" => transport_id = Some(v.to_owned()),
                    _ => {}
                }
            }
        }
        let is_wireless = serial.contains(':') || serial.contains("_adb-tls-connect");
        devices.push(AdbDevice {
            serial: serial.to_owned(),
            state,
            product,
            model,
            device,
            transport_id,
            is_wireless,
        });
    }
    devices
}

/// Validates an ADB serial identifier before passing it as an argument to `adb -s`.
pub(crate) fn validate_serial(serial: &str) -> Result<(), AdbError> {
    if serial.is_empty()
        || serial.len() > 128
        || !serial
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(AdbError::InvalidSerial(serial.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usb_and_wireless_adb_devices() {
        let sample = "\
List of devices attached
* daemon not running; starting now at tcp:5037
emulator-5554          device usb:1-2 product:demo_tablet model:Demo_Tablet device:generic_arm64_tab transport_id:4
192.0.2.20:5555        device product:demo_phone model:Demo_Phone device:generic_arm64_phone transport_id:7
DEADBEEF               unauthorized transport_id:9
";
        let list = parse_adb_devices_output(sample);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].serial, "emulator-5554");
        assert_eq!(list[0].state, AdbDeviceState::Device);
        assert_eq!(list[0].model.as_deref(), Some("Demo_Tablet"));
        assert!(!list[0].is_wireless);
        assert!(list[1].is_wireless);
        assert_eq!(list[2].state, AdbDeviceState::Unauthorized);
    }
}
