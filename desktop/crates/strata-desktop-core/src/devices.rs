//! Multi-device profile persistence for ADB, LAN, and SSH-tunneled Android hosts.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::DEFAULT_REMOTE_API_PORT;

/// Transport mechanism used to reach `stratad` on an Android device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceKind {
    /// USB or Wireless ADB device tunneled via `adb forward`.
    Adb,
    /// Direct RFC1918 LAN HTTP/WS endpoint (`lanEnabled = true`).
    Lan,
    /// SSH local port-forwarded endpoint (`ssh -L`).
    Ssh,
}

/// Persisted desktop profile describing one managed Android device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceProfile {
    pub id: String,
    pub label: String,
    pub kind: DeviceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    pub host: String,
    #[serde(default = "default_remote_port")]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

const fn default_remote_port() -> u16 {
    DEFAULT_REMOTE_API_PORT
}

/// On-disk JSON wrapper for saved desktop devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStoreFile {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_device_id: Option<String>,
    #[serde(default)]
    pub devices: Vec<DeviceProfile>,
}

impl Default for DeviceStoreFile {
    fn default() -> Self {
        Self {
            version: 1,
            active_device_id: None,
            devices: Vec::new(),
        }
    }
}

/// Errors produced when validating or persisting device profiles.
#[derive(Debug, Error)]
pub enum DeviceStoreError {
    #[error("invalid device profile: {0}")]
    Validation(String),
    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("corrupt device store JSON at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// Validates a device profile before persistence or connection.
pub fn validate_device_profile(profile: &DeviceProfile) -> Result<(), DeviceStoreError> {
    let id = profile.id.trim();
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(DeviceStoreError::Validation(
            "id must be 1..=64 ASCII alphanumeric/[-_.] chars".to_owned(),
        ));
    }
    if profile.label.trim().is_empty() {
        return Err(DeviceStoreError::Validation(
            "label must not be empty".to_owned(),
        ));
    }
    if profile.port == 0 {
        return Err(DeviceStoreError::Validation(
            "port must be between 1 and 65535".to_owned(),
        ));
    }
    match profile.kind {
        DeviceKind::Adb => {
            if profile
                .serial
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return Err(DeviceStoreError::Validation(
                    "ADB device requires a non-empty serial".to_owned(),
                ));
            }
        }
        DeviceKind::Lan | DeviceKind::Ssh => {
            if profile.host.trim().is_empty() {
                return Err(DeviceStoreError::Validation(
                    "LAN/SSH device requires a non-empty host".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

/// Atomic file-backed store for multi-device desktop profiles.
#[derive(Debug, Clone)]
pub struct DeviceStore {
    path: PathBuf,
}

impl DeviceStore {
    /// Creates a store bound to the given JSON path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the backing JSON file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads saved profiles, returning an empty default store if the file does not exist yet.
    pub fn load(&self) -> Result<DeviceStoreFile, DeviceStoreError> {
        if !self.path.exists() {
            return Ok(DeviceStoreFile::default());
        }
        let raw = fs::read_to_string(&self.path).map_err(|source| DeviceStoreError::Io {
            path: self.path.clone(),
            source,
        })?;
        serde_json::from_str(&raw).map_err(|source| DeviceStoreError::Parse {
            path: self.path.clone(),
            source,
        })
    }

    /// Atomically persists the full store file (`temp -> sync_all -> rename`).
    pub fn save(&self, store: &DeviceStoreFile) -> Result<(), DeviceStoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| DeviceStoreError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let tmp_path = self.path.with_extension("json.tmp");
        let payload =
            serde_json::to_vec_pretty(store).map_err(|source| DeviceStoreError::Parse {
                path: self.path.clone(),
                source,
            })?;
        let mut file = fs::File::create(&tmp_path).map_err(|source| DeviceStoreError::Io {
            path: tmp_path.clone(),
            source,
        })?;
        file.write_all(&payload)
            .and_then(|()| file.sync_all())
            .map_err(|source| DeviceStoreError::Io {
                path: tmp_path.clone(),
                source,
            })?;
        drop(file);
        fs::rename(&tmp_path, &self.path).map_err(|source| DeviceStoreError::Io {
            path: self.path.clone(),
            source,
        })
    }

    /// Inserts or updates a device profile by `id` and persists atomically.
    pub fn upsert(&self, profile: DeviceProfile) -> Result<DeviceStoreFile, DeviceStoreError> {
        validate_device_profile(&profile)?;
        let mut data = self.load()?;
        if let Some(existing) = data.devices.iter_mut().find(|d| d.id == profile.id) {
            *existing = profile;
        } else {
            data.devices.push(profile);
        }
        self.save(&data)?;
        Ok(data)
    }

    /// Removes a device profile by `id` and returns whether an entry was deleted.
    pub fn remove(&self, id: &str) -> Result<(bool, DeviceStoreFile), DeviceStoreError> {
        let mut data = self.load()?;
        let before = data.devices.len();
        data.devices.retain(|d| d.id != id);
        let removed = data.devices.len() != before;
        if data.active_device_id.as_deref() == Some(id) {
            data.active_device_id = data.devices.first().map(|d| d.id.clone());
        }
        if removed {
            self.save(&data)?;
        }
        Ok((removed, data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("strata-desktop-test-{name}-{nanos}.json"))
    }

    fn make_profile(id: &str, kind: DeviceKind, serial: Option<&str>) -> DeviceProfile {
        DeviceProfile {
            id: id.to_owned(),
            label: "Demo Device".to_owned(),
            kind,
            serial: serial.map(str::to_owned),
            host: "127.0.0.1".to_owned(),
            port: 7681,
            token: Some("abcdef123456".to_owned()),
            ssh_user: Some("root".to_owned()),
            ssh_port: Some(2222),
            model: Some("Demo_Device".to_owned()),
        }
    }

    #[test]
    fn round_trip_upsert_and_remove() {
        let path = temp_store_path("devices");
        let store = DeviceStore::new(&path);
        assert!(store.load().unwrap().devices.is_empty());

        let tablet = make_profile("demo-tablet", DeviceKind::Adb, Some("emulator-5554"));
        let saved = store.upsert(tablet.clone()).unwrap();
        assert_eq!(saved.devices.len(), 1);
        assert_eq!(store.load().unwrap().devices[0], tablet);

        let (removed, after) = store.remove("demo-tablet").unwrap();
        assert!(removed);
        assert!(after.devices.is_empty());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rejects_invalid_profiles() {
        assert!(validate_device_profile(&make_profile("bad/id", DeviceKind::Adb, None)).is_err());
        assert!(
            validate_device_profile(&make_profile("ok-id", DeviceKind::Adb, Some("   "))).is_err()
        );
    }
}
