//! Authoritative configuration store with atomic persistence and zero-polling `inotify` watching.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use strata_protocol::rpc::{ContainerConfigPatchParams, error_codes};
use strata_protocol::{ContainerConfig, StrataConfig};
use strata_runtime::fs_atomic;
use strata_sys::inotify::InotifyWatcher;
use tokio::io::unix::AsyncFd;
use tokio::sync::RwLock;

use crate::manager::ContainerManager;
use crate::sideveil_seam::SideVeilBridgeSeam;

/// Shared authoritative configuration holder backed by `/data/adb/strata/config.json`.
#[derive(Debug)]
pub(crate) struct ConfigStore {
    path: PathBuf,
    inner: RwLock<StrataConfig>,
}

impl ConfigStore {
    /// Loads and validates `path`, or atomically seeds defaults if the file does not exist (`ENOENT`).
    /// Never overwrites an existing file on parse or I/O errors.
    pub(crate) fn load_or_seed(path: PathBuf) -> Result<Self, String> {
        let cfg = match fs::read_to_string(&path) {
            Ok(raw) => {
                let parsed: StrataConfig = serde_json::from_str(&raw)
                    .map_err(|e| format!("failed to parse '{}': {e}", path.display()))?;
                parsed
                    .validate()
                    .map_err(|e| format!("invalid config '{}': {e}", path.display()))?;
                parsed
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let default_cfg = StrataConfig::default();
                persist_config(&path, &default_cfg)?;
                default_cfg
            }
            Err(err) => {
                return Err(format!("failed to read '{}': {err}", path.display()));
            }
        };

        Ok(Self {
            path,
            inner: RwLock::new(cfg),
        })
    }

    /// Returns a cloned snapshot of the current authoritative configuration.
    pub(crate) async fn snapshot(&self) -> StrataConfig {
        self.inner.read().await.clone()
    }

    /// Re-reads and validates `config.json` from disk, updating in-memory state if valid.
    pub(crate) async fn reload_from_disk(&self) -> Result<StrataConfig, (i32, String)> {
        let raw = fs::read_to_string(&self.path).map_err(|e| {
            (
                error_codes::INTERNAL_ERROR,
                format!("read '{}': {e}", self.path.display()),
            )
        })?;
        let parsed: StrataConfig = serde_json::from_str(&raw)
            .map_err(|e| (error_codes::INVALID_PARAMS, format!("parse config: {e}")))?;
        parsed
            .validate()
            .map_err(|e| (error_codes::INVALID_PARAMS, e.to_string()))?;

        let mut guard = self.inner.write().await;
        *guard = parsed.clone();
        Ok(parsed)
    }

    /// Patches fields on a single container, validates cross-container invariants, and persists atomically.
    pub(crate) async fn patch_container(
        &self,
        params: ContainerConfigPatchParams,
    ) -> Result<(String, ContainerConfig), (i32, String)> {
        let mut guard = self.inner.write().await;
        let mut candidate = guard.clone();
        let target_id = candidate
            .resolve_container_id(params.container.as_deref())
            .to_owned();

        let Some(entry) = candidate.containers.get_mut(&target_id) else {
            return Err((
                error_codes::CONTAINER_NOT_FOUND,
                format!("container '{target_id}' not found"),
            ));
        };

        if let Some(v) = params.enabled {
            entry.enabled = v;
        }
        if let Some(v) = params.auto_start {
            entry.auto_start = v;
        }
        if let Some(v) = params.image_path {
            entry.image_path = v;
        }
        if let Some(v) = params.mount_dir {
            entry.mount_dir = v;
        }
        if let Some(v) = params.hostname {
            entry.hostname = v;
        }
        if let Some(v) = params.ssh_port {
            entry.ssh.port = v;
        }
        if let Some(v) = params.host_bridge {
            entry.host_bridge = v;
        }
        if let Some(v) = params.host_runtime {
            entry.host_runtime = v;
        }
        if let Some(v) = params.extra_char_devs {
            entry.extra_char_devs = v;
        }
        if let Some(v) = params.extra_mounts {
            entry.extra_mounts = v;
        }

        candidate
            .validate()
            .map_err(|e| (error_codes::INVALID_PARAMS, e.to_string()))?;

        persist_config(&self.path, &candidate).map_err(|e| (error_codes::INTERNAL_ERROR, e))?;

        let updated = candidate
            .containers
            .get(&target_id)
            .cloned()
            .ok_or_else(|| {
                (
                    error_codes::CONTAINER_NOT_FOUND,
                    format!("container '{target_id}' missing after patch"),
                )
            })?;
        *guard = candidate;
        Ok((target_id, updated))
    }

    /// Spawns a zero-polling `inotify` watcher on the parent directory of `config.json`.
    pub(crate) fn spawn_inotify_watcher(
        self: &Arc<Self>,
        manager: Arc<ContainerManager>,
        sideveil: Arc<SideVeilBridgeSeam>,
    ) {
        let Some(parent_dir) = self.path.parent().map(PathBuf::from) else {
            return;
        };
        let Ok(watcher) = InotifyWatcher::watch_dir(&parent_dir) else {
            return;
        };
        // SAFETY: `InotifyWatcher` exclusively owns its non-blocking `OwnedFd` for its entire lifetime.
        let Ok(async_fd) = (unsafe { AsyncFd::register(watcher) }) else {
            return;
        };
        let store = Arc::clone(self);

        tokio::spawn(async move {
            loop {
                let Ok(mut guard) = async_fd.readable().await else {
                    break;
                };
                let drained = guard.get_inner().drain_events().unwrap_or(false);
                guard.clear_ready();
                if drained && store.reload_from_disk().await.is_ok() {
                    manager.sync_running_containers().await;
                    sideveil.notify_config_changed();
                }
            }
        });
    }
}

fn persist_config(path: &Path, cfg: &StrataConfig) -> Result<(), String> {
    let mut rendered =
        serde_json::to_string_pretty(cfg).map_err(|e| format!("serialize config: {e}"))?;
    rendered.push('\n');
    fs_atomic::write_atomic(path, rendered.as_bytes(), 0o600)
        .map_err(|e| format!("persist '{}': {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_protocol::BindMount;

    #[tokio::test]
    async fn patch_container_updates_and_validates_extra_devs_and_mounts() {
        let dir = std::env::temp_dir().join(format!("strata-cfg-patch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("config.json");

        let store = ConfigStore::load_or_seed(path).expect("seed config");
        let (id, updated) = store
            .patch_container(ContainerConfigPatchParams {
                extra_char_devs: Some(vec!["uinput".to_owned()]),
                extra_mounts: Some(vec![BindMount {
                    host_path: "/data/local/tmp/work".to_owned(),
                    container_path: "/mnt/work".to_owned(),
                    read_only: false,
                }]),
                ..ContainerConfigPatchParams::default()
            })
            .await
            .expect("valid patch should succeed");
        assert_eq!(id, "main");
        assert_eq!(updated.extra_char_devs, vec!["uinput"]);
        assert_eq!(updated.extra_mounts.len(), 1);

        let err = store
            .patch_container(ContainerConfigPatchParams {
                extra_char_devs: Some(vec!["block/sda".to_owned()]),
                ..ContainerConfigPatchParams::default()
            })
            .await;
        assert!(err.is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
