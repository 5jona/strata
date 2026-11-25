//! Per-container supervised background service lifecycle (`service.*` RPCs).
//! Spawns services via `stratad __exec` (`setns` + `setsid`) with zero-polling `pidfd` supervision.

mod runner;
mod state;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use strata_protocol::EventEnvelope;
use strata_protocol::config::validate::validate_service_spec;
use strata_protocol::rpc::{
    ContainerServiceRuntime, ServiceDeleteResult, ServiceLogsParams, ServiceLogsResult,
    ServiceSelectorParams, ServiceStateChangedPayload, ServiceStatus, ServiceUpsertParams, events,
};
use tokio::sync::RwLock;

use crate::config_store::ConfigStore;
use crate::error::{DaemonError, DaemonResult};
use crate::manager::ContainerManager;
use crate::sideveil_seam::{EventHub, PluginEventSink};
use runner::{spawn_service_loop, terminate_process_group};
use state::{MAX_SERVICE_LOG_LINES, ServiceSlot};

/// Manages declared background services across all Strata containers.
#[derive(Debug)]
pub(crate) struct ServiceSupervisor {
    pub(super) config_store: Arc<ConfigStore>,
    pub(super) manager: Arc<ContainerManager>,
    pub(super) event_hub: Arc<EventHub>,
    pub(super) self_exe: PathBuf,
    slots: RwLock<HashMap<(String, String), Arc<ServiceSlot>>>,
}

impl ServiceSupervisor {
    #[must_use]
    pub(crate) fn new(
        config_store: Arc<ConfigStore>,
        manager: Arc<ContainerManager>,
        event_hub: Arc<EventHub>,
        self_exe: PathBuf,
    ) -> Self {
        Self {
            config_store,
            manager,
            event_hub,
            self_exe,
            slots: RwLock::new(HashMap::new()),
        }
    }

    async fn get_or_create_slot(&self, container: &str, service_id: &str) -> Arc<ServiceSlot> {
        let key = (container.to_owned(), service_id.to_owned());
        if let Some(existing) = self.slots.read().await.get(&key) {
            return Arc::clone(existing);
        }
        let mut map = self.slots.write().await;
        Arc::clone(
            map.entry(key)
                .or_insert_with(|| Arc::new(ServiceSlot::new())),
        )
    }

    /// Lists all configured services and their live runtime status for one container.
    pub(crate) async fn list(
        &self,
        container: Option<&str>,
    ) -> DaemonResult<Vec<ContainerServiceRuntime>> {
        let (cid, cfg, _) = self.manager.resolve_container(container).await?;
        let mut out = Vec::with_capacity(cfg.services.len());
        for spec in &cfg.services {
            let slot = self.get_or_create_slot(&cid, &spec.id).await;
            let guard = slot.state.read().await;
            out.push(guard.snapshot(spec));
        }
        Ok(out)
    }

    /// Inserts or updates a service definition in `config.json` atomically.
    pub(crate) async fn upsert(
        self: &Arc<Self>,
        params: ServiceUpsertParams,
    ) -> DaemonResult<ContainerServiceRuntime> {
        let (cid, _, _) = self
            .manager
            .resolve_container(params.container.as_deref())
            .await?;
        let mut spec = params.spec;
        spec.max_restarts = spec.max_restarts.clamp(1, 50);
        spec.backoff_ms = spec.backoff_ms.clamp(250, 60_000);
        validate_service_spec(&cid, &spec)?;

        self.config_store.upsert_service(&cid, spec.clone()).await?;
        let slot = self.get_or_create_slot(&cid, &spec.id).await;
        let snapshot = slot.state.read().await.snapshot(&spec);
        Ok(snapshot)
    }

    /// Stops and removes a service definition from `config.json`.
    pub(crate) async fn delete(
        self: &Arc<Self>,
        params: ServiceSelectorParams,
    ) -> DaemonResult<ServiceDeleteResult> {
        let (cid, _, _) = self
            .manager
            .resolve_container(params.container.as_deref())
            .await?;
        self.stop_one(&cid, &params.service_id).await;
        let deleted = self
            .config_store
            .delete_service(&cid, &params.service_id)
            .await?;
        if !deleted {
            return Err(DaemonError::ServiceNotFound(params.service_id));
        }
        self.slots
            .write()
            .await
            .remove(&(cid.clone(), params.service_id.clone()));
        Ok(ServiceDeleteResult {
            container: cid,
            service_id: params.service_id,
            deleted: true,
        })
    }

    /// Starts one declared service inside a running container.
    pub(crate) async fn start(
        self: &Arc<Self>,
        params: ServiceSelectorParams,
    ) -> DaemonResult<ContainerServiceRuntime> {
        let (cid, ns_pid, cfg) = self
            .manager
            .running_target(params.container.as_deref())
            .await?;
        let spec = cfg
            .services
            .iter()
            .find(|s| s.id == params.service_id)
            .cloned()
            .ok_or_else(|| DaemonError::ServiceNotFound(params.service_id.clone()))?;

        let slot = self.get_or_create_slot(&cid, &spec.id).await;
        let _guard = slot.lock.lock().await;
        {
            let st = slot.state.read().await;
            if st.status == ServiceStatus::Running && st.pid.is_some() {
                return Ok(st.snapshot(&spec));
            }
        }
        let generation = {
            let mut st = slot.state.write().await;
            st.desired_running = true;
            st.generation = st.generation.saturating_add(1);
            st.restart_count = 0;
            st.last_error = None;
            st.generation
        };
        spawn_service_loop(
            Arc::clone(self),
            cid,
            spec.clone(),
            cfg.cap_profile(),
            ns_pid,
            Arc::clone(&slot),
            generation,
        );
        tokio::task::yield_now().await;
        let snapshot = slot.state.read().await.snapshot(&spec);
        Ok(snapshot)
    }

    /// Gracefully stops one running service (`-pgid` SIGTERM -> SIGKILL).
    pub(crate) async fn stop(
        self: &Arc<Self>,
        params: ServiceSelectorParams,
    ) -> DaemonResult<ContainerServiceRuntime> {
        let (cid, cfg, _) = self
            .manager
            .resolve_container(params.container.as_deref())
            .await?;
        let spec = cfg
            .services
            .iter()
            .find(|s| s.id == params.service_id)
            .cloned()
            .ok_or_else(|| DaemonError::ServiceNotFound(params.service_id.clone()))?;
        self.stop_one(&cid, &spec.id).await;
        let slot = self.get_or_create_slot(&cid, &spec.id).await;
        let snapshot = slot.state.read().await.snapshot(&spec);
        Ok(snapshot)
    }

    /// Restarts one service cleanly.
    pub(crate) async fn restart(
        self: &Arc<Self>,
        params: ServiceSelectorParams,
    ) -> DaemonResult<ContainerServiceRuntime> {
        let _ = self.stop(params.clone()).await?;
        self.start(params).await
    }

    /// Reads the bounded stdout/stderr ring buffer for one service.
    pub(crate) async fn logs(&self, params: ServiceLogsParams) -> DaemonResult<ServiceLogsResult> {
        let (cid, cfg, _) = self
            .manager
            .resolve_container(params.container.as_deref())
            .await?;
        if !cfg.services.iter().any(|s| s.id == params.service_id) {
            return Err(DaemonError::ServiceNotFound(params.service_id));
        }
        let slot = self.get_or_create_slot(&cid, &params.service_id).await;
        let limit = params
            .limit
            .unwrap_or(MAX_SERVICE_LOG_LINES)
            .min(MAX_SERVICE_LOG_LINES);
        let guard = slot.state.read().await;
        let start = guard.logs.len().saturating_sub(limit);
        let lines = guard.logs.iter().skip(start).cloned().collect();
        Ok(ServiceLogsResult {
            container: cid,
            service_id: params.service_id,
            lines,
        })
    }

    /// Auto-starts all `auto_start == true` services when a container transitions to `Running`.
    pub(crate) async fn start_auto_services(self: &Arc<Self>, container_id: &str) {
        let Ok((_, cfg, _)) = self.manager.resolve_container(Some(container_id)).await else {
            return;
        };
        for spec in cfg.services.into_iter().filter(|s| s.auto_start) {
            let _ = self
                .start(ServiceSelectorParams {
                    container: Some(container_id.to_owned()),
                    service_id: spec.id,
                })
                .await;
        }
    }

    /// Gracefully terminates all supervised services inside `container_id` before container teardown.
    pub(crate) async fn stop_container_services(&self, container_id: &str) {
        let ids: Vec<String> = self
            .slots
            .read()
            .await
            .keys()
            .filter(|(c, _)| c == container_id)
            .map(|(_, s)| s.clone())
            .collect();
        for sid in ids {
            self.stop_one(container_id, &sid).await;
        }
    }

    /// Evicts all runtime service slots belonging to a deleted container.
    pub(crate) async fn evict_container_slots(&self, container_id: &str) {
        self.stop_container_services(container_id).await;
        self.slots
            .write()
            .await
            .retain(|(c, _), _| c != container_id);
    }

    /// Stops all supervised services across all containers during daemon shutdown.
    pub(crate) async fn stop_all(&self) {
        let keys: Vec<(String, String)> = self.slots.read().await.keys().cloned().collect();
        for (cid, sid) in keys {
            self.stop_one(&cid, &sid).await;
        }
    }

    async fn stop_one(&self, container_id: &str, service_id: &str) {
        let slot = {
            let map = self.slots.read().await;
            map.get(&(container_id.to_owned(), service_id.to_owned()))
                .cloned()
        };
        let Some(slot) = slot else {
            return;
        };
        let _guard = slot.lock.lock().await;
        let (prev_status, pid, restart_count) = {
            let mut st = slot.state.write().await;
            let prev = st.status;
            st.desired_running = false;
            st.generation = st.generation.saturating_add(1);
            let pid = st.pid.take();
            st.started_at = None;
            st.status = ServiceStatus::Stopped;
            (prev, pid, st.restart_count)
        };
        if let Some(pgid) = pid {
            let _ = tokio::task::spawn_blocking(move || terminate_process_group(pgid)).await;
        }
        if prev_status != ServiceStatus::Stopped {
            self.emit_state_changed(
                container_id,
                service_id,
                prev_status,
                ServiceStatus::Stopped,
                None,
                restart_count,
                None,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_state_changed(
        &self,
        container: &str,
        service_id: &str,
        prev_status: ServiceStatus,
        status: ServiceStatus,
        pid: Option<u32>,
        restart_count: u32,
        last_error: Option<String>,
    ) {
        let payload = ServiceStateChangedPayload {
            container: container.to_owned(),
            service_id: service_id.to_owned(),
            prev_status,
            status,
            pid,
            restart_count,
            last_error,
        };
        self.event_hub.emit(EventEnvelope::new(
            events::SERVICE_STATE_CHANGED,
            serde_json::to_value(payload).ok(),
        ));
    }
}
