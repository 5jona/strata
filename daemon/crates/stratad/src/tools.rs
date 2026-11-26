//! Hybrid container toolchain manager (`mise`, `uv`, system package managers, core bridges, layout doctor).

mod action_cmds;
mod action_cmds_mut;
mod actions;
mod cache;
mod catalog;
mod catalog_bundles;
mod catalog_data;
mod config;
mod config_patch;
mod config_presets;
mod discovery;
mod doctor;
mod exec;
mod list;
mod list_sys;
mod shell_presets;
mod status;
mod status_parsers;
mod status_parsers_sys;
mod versions;
mod versions_parse;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use strata_protocol::EventEnvelope;
use strata_protocol::rpc::{
    TaskRunResult, ToolsBootstrapParams, ToolsCacheCleanParams, ToolsCacheCleanResult,
    ToolsCatalogParams, ToolsCatalogResult, ToolsConfigGetParams, ToolsConfigResult,
    ToolsConfigSetParams, ToolsDoctorParams, ToolsDoctorResult, ToolsInstallParams,
    ToolsListParams, ToolsListResult, ToolsStateChangedPayload, ToolsStatusParams,
    ToolsStatusResult, ToolsUninstallParams, ToolsUpgradeParams, ToolsUseParams,
    ToolsVersionsParams, ToolsVersionsResult, events,
};
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::error::{DaemonError, DaemonResult};
use crate::manager::ContainerManager;
use crate::sideveil_seam::{EventHub, PluginEventSink};
use crate::task_runner::TaskRunner;

/// Coordinates read-only tool diagnostics and streaming toolchain mutations per container.
#[derive(Debug)]
pub(crate) struct ToolsManager {
    pub(super) manager: Arc<ContainerManager>,
    pub(super) task_runner: Arc<TaskRunner>,
    pub(super) event_hub: Arc<EventHub>,
    pub(super) self_exe: PathBuf,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl ToolsManager {
    #[must_use]
    pub(crate) fn new(
        manager: Arc<ContainerManager>,
        task_runner: Arc<TaskRunner>,
        event_hub: Arc<EventHub>,
        self_exe: PathBuf,
    ) -> Self {
        Self {
            manager,
            task_runner,
            event_hub,
            self_exe,
            locks: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn status(
        &self,
        params: ToolsStatusParams,
    ) -> DaemonResult<ToolsStatusResult> {
        status::query_status(self, params).await
    }

    pub(crate) async fn list(&self, params: ToolsListParams) -> DaemonResult<ToolsListResult> {
        list::query_list(self, params).await
    }

    pub(crate) async fn catalog(
        &self,
        params: ToolsCatalogParams,
    ) -> DaemonResult<ToolsCatalogResult> {
        catalog::query_catalog(self, params).await
    }

    pub(crate) async fn versions(
        &self,
        params: ToolsVersionsParams,
    ) -> DaemonResult<ToolsVersionsResult> {
        versions::query_versions(self, params).await
    }

    pub(crate) async fn use_tool(&self, params: ToolsUseParams) -> DaemonResult<TaskRunResult> {
        versions::use_version(self, params).await
    }

    pub(crate) async fn config_get(
        &self,
        params: ToolsConfigGetParams,
    ) -> DaemonResult<ToolsConfigResult> {
        config::get_config(self, params).await
    }

    pub(crate) async fn config_set(
        &self,
        params: ToolsConfigSetParams,
    ) -> DaemonResult<ToolsConfigResult> {
        let (container, entry, _) = self
            .manager
            .resolve_container(params.container.as_deref())
            .await?;
        let _guard = self.try_acquire_write_lock(&container).await?;
        let res = config::set_config(self, &container, &entry, params).await?;
        self.emit_state_changed(container, "config_updated", None);
        Ok(res)
    }

    pub(crate) async fn bootstrap(
        &self,
        params: ToolsBootstrapParams,
    ) -> DaemonResult<TaskRunResult> {
        actions::bootstrap(self, params).await
    }

    pub(crate) async fn install(&self, params: ToolsInstallParams) -> DaemonResult<TaskRunResult> {
        actions::install(self, params).await
    }

    pub(crate) async fn uninstall(
        &self,
        params: ToolsUninstallParams,
    ) -> DaemonResult<TaskRunResult> {
        actions::uninstall(self, params).await
    }

    pub(crate) async fn upgrade(&self, params: ToolsUpgradeParams) -> DaemonResult<TaskRunResult> {
        actions::upgrade(self, params).await
    }

    pub(crate) async fn doctor(
        &self,
        params: ToolsDoctorParams,
    ) -> DaemonResult<ToolsDoctorResult> {
        doctor::query_doctor(self, params).await
    }

    pub(crate) async fn cache_clean(
        &self,
        params: ToolsCacheCleanParams,
    ) -> DaemonResult<ToolsCacheCleanResult> {
        cache::clean_cache(self, params).await
    }

    pub(super) async fn try_acquire_write_lock(
        &self,
        container_id: &str,
    ) -> DaemonResult<OwnedMutexGuard<()>> {
        let mutex = {
            let mut map = self.locks.lock().await;
            Arc::clone(
                map.entry(container_id.to_owned())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        mutex.try_lock_owned().map_err(|_| {
            DaemonError::InvalidState(format!(
                "another tools operation is already running for container '{container_id}'"
            ))
        })
    }

    pub(super) fn emit_state_changed(
        &self,
        container: String,
        reason: &str,
        task_id: Option<String>,
    ) {
        let payload = serde_json::to_value(ToolsStateChangedPayload {
            container,
            reason: reason.to_owned(),
            task_id,
        })
        .ok();
        self.event_hub
            .emit(EventEnvelope::new(events::TOOLS_STATE_CHANGED, payload));
    }
}
