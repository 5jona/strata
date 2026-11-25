//! Atomic configuration mutations for containers, web console settings, and managed services.

use strata_protocol::rpc::{ContainerServiceSpec, WebConfigureParams};
use strata_protocol::{ContainerConfig, StrataConfig, WebConsoleConfig};

use crate::config_store::{ConfigStore, persist_config};
use crate::error::{DaemonError, DaemonResult};

impl ConfigStore {
    /// Registers a newly created or cloned container atomically in `config.json`.
    pub(crate) async fn add_container(
        &self,
        id: String,
        cfg: ContainerConfig,
    ) -> DaemonResult<StrataConfig> {
        let _write_guard = self.write_lock.lock().await;
        let mut candidate = self.inner.read().await.clone();
        if candidate.containers.contains_key(&id) {
            return Err(DaemonError::ContainerAlreadyExists(id));
        }
        candidate.containers.insert(id, cfg);
        candidate.validate()?;
        self.commit_candidate(candidate).await
    }

    /// Removes a container from `config.json`, switching `defaultContainer` when needed.
    /// Refuses removing the last configured container so `StrataConfig` always stays valid.
    pub(crate) async fn remove_container(
        &self,
        id: &str,
        force: bool,
    ) -> DaemonResult<Option<String>> {
        let _write_guard = self.write_lock.lock().await;
        let mut candidate = self.inner.read().await.clone();
        if !candidate.containers.contains_key(id) {
            return Err(DaemonError::ContainerNotFound(id.to_owned()));
        }
        if candidate.containers.len() <= 1 {
            return Err(DaemonError::InvalidParams(
                "cannot delete the only configured container".to_owned(),
            ));
        }
        let mut new_default = None;
        if candidate.default_container == id {
            if !force {
                return Err(DaemonError::InvalidParams(format!(
                    "container '{id}' is the defaultContainer; pass force=true to delete and reassign default"
                )));
            }
            let replacement = candidate
                .containers
                .keys()
                .find(|k| k.as_str() != id)
                .cloned()
                .ok_or_else(|| DaemonError::Internal("missing replacement container".to_owned()))?;
            candidate.default_container = replacement.clone();
            new_default = Some(replacement);
        }
        candidate.containers.remove(id);
        candidate.validate()?;
        let _ = self.commit_candidate(candidate).await?;
        Ok(new_default)
    }

    /// Patches embedded HTTP/WS control-plane settings atomically.
    pub(crate) async fn update_web_config(
        &self,
        params: WebConfigureParams,
    ) -> DaemonResult<WebConsoleConfig> {
        let _write_guard = self.write_lock.lock().await;
        let mut candidate = self.inner.read().await.clone();
        if let Some(enabled) = params.enabled {
            candidate.web.enabled = enabled;
        }
        if let Some(lan) = params.lan_enabled {
            candidate.web.lan_enabled = lan;
        }
        if let Some(port) = params.port {
            candidate.web.port = port;
        }
        candidate.validate()?;
        let saved = self.commit_candidate(candidate).await?;
        Ok(saved.web)
    }

    /// Inserts or replaces a supervised background service specification on one container.
    pub(crate) async fn upsert_service(
        &self,
        container_id: &str,
        spec: ContainerServiceSpec,
    ) -> DaemonResult<()> {
        let _write_guard = self.write_lock.lock().await;
        let mut candidate = self.inner.read().await.clone();
        let Some(entry) = candidate.containers.get_mut(container_id) else {
            return Err(DaemonError::ContainerNotFound(container_id.to_owned()));
        };
        if let Some(existing) = entry.services.iter_mut().find(|s| s.id == spec.id) {
            *existing = spec;
        } else {
            entry.services.push(spec);
        }
        candidate.validate()?;
        let _ = self.commit_candidate(candidate).await?;
        Ok(())
    }

    /// Deletes one supervised background service specification from a container.
    pub(crate) async fn delete_service(
        &self,
        container_id: &str,
        service_id: &str,
    ) -> DaemonResult<bool> {
        let _write_guard = self.write_lock.lock().await;
        let mut candidate = self.inner.read().await.clone();
        let Some(entry) = candidate.containers.get_mut(container_id) else {
            return Err(DaemonError::ContainerNotFound(container_id.to_owned()));
        };
        let before = entry.services.len();
        entry.services.retain(|s| s.id != service_id);
        if entry.services.len() == before {
            return Ok(false);
        }
        candidate.validate()?;
        let _ = self.commit_candidate(candidate).await?;
        Ok(true)
    }

    async fn commit_candidate(&self, candidate: StrataConfig) -> DaemonResult<StrataConfig> {
        let path = self.path.clone();
        let to_persist = candidate.clone();
        tokio::task::spawn_blocking(move || persist_config(&path, &to_persist))
            .await
            .map_err(|e| DaemonError::join("persist config join", e))?
            .map_err(DaemonError::Internal)?;
        *self.inner.write().await = candidate.clone();
        Ok(candidate)
    }
}
