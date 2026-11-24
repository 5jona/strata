//! Transport-agnostic JSON-RPC 2.0 dispatcher shared by `strata.sock`, HTTP `/api/rpc`, and SideVeil.

mod extended;

use std::sync::Arc;
use std::time::Instant;

use serde::de::DeserializeOwned;
use serde_json::Value;
use strata_protocol::rpc::{
    BackupParams, ContainerConfigPatchParams, ContainerConfigResult, ContainerLogsParams,
    ContainerSelectorParams, DaemonHealthResult, EventsSubscribeResult, HudSetParams, PingResult,
    SessionAcquireParams, SessionReleaseParams, SessionReleaseResult, SshKeyAddParams,
    SyncDnsResult, TaskCancelParams, TaskCancelResult, TaskLogsParams, TaskRunParams, methods,
};
use strata_protocol::{DaemonSummarySnapshot, RpcRequest, RpcResponse};

use crate::catalog::CatalogClient;
use crate::config_store::ConfigStore;
use crate::error::{DaemonError, DaemonResult};
use crate::jobs::ProvisionJobManager;
use crate::manager::ContainerManager;
use crate::services::ServiceSupervisor;
use crate::sideveil_seam::SideVeilBridgeSeam;
use crate::task_runner::TaskRunner;
use crate::web::WebServerController;

/// Identifies the transport origin of an incoming JSON-RPC request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RpcCaller {
    /// Direct root client connected over `strata.sock` (e.g., `strata` CLI).
    LocalCtl { conn_id: u64 },
    /// Remote plugin client forwarded over SideVeil `hub.sock`.
    SideVeilBridge,
    /// Authenticated HTTP/WS client connected over the embedded PC control plane.
    WebConsole,
}

/// Central RPC dispatcher decoupled from Unix socket and HTTP transports.
#[derive(Debug)]
pub(crate) struct Dispatcher {
    started_at: Instant,
    config_store: Arc<ConfigStore>,
    pub(super) manager: Arc<ContainerManager>,
    task_runner: Arc<TaskRunner>,
    sideveil: Arc<SideVeilBridgeSeam>,
    pub(super) web: Arc<WebServerController>,
    pub(super) catalog: Arc<CatalogClient>,
    pub(super) jobs: Arc<ProvisionJobManager>,
    pub(super) services: Arc<ServiceSupervisor>,
}

impl Dispatcher {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        started_at: Instant,
        config_store: Arc<ConfigStore>,
        manager: Arc<ContainerManager>,
        task_runner: Arc<TaskRunner>,
        sideveil: Arc<SideVeilBridgeSeam>,
        web: Arc<WebServerController>,
        catalog: Arc<CatalogClient>,
        jobs: Arc<ProvisionJobManager>,
        services: Arc<ServiceSupervisor>,
    ) -> Self {
        Self {
            started_at,
            config_store,
            manager,
            task_runner,
            sideveil,
            web,
            catalog,
            jobs,
            services,
        }
    }

    /// Handles a single JSON-RPC 2.0 request and returns the serialized response envelope.
    pub(crate) async fn handle(&self, caller: RpcCaller, req: RpcRequest) -> RpcResponse {
        let id = req.id.clone();
        match self.dispatch(caller, &req.method, req.params).await {
            Ok(result) => RpcResponse::ok(id, result),
            Err(err) => {
                let (code, message) = err.to_rpc_tuple();
                RpcResponse::err(id, code, message)
            }
        }
    }

    async fn dispatch(
        &self,
        caller: RpcCaller,
        method: &str,
        params: Option<Value>,
    ) -> DaemonResult<Value> {
        match method {
            methods::DAEMON_PING => to_json(PingResult {
                pong: true,
                pid: std::process::id(),
                uptime_ms: self.uptime_ms(),
            }),
            methods::DAEMON_HEALTH => to_json(DaemonHealthResult {
                ok: true,
                pid: std::process::id(),
                uptime_ms: self.uptime_ms(),
                running_containers: self.manager.running_count().await,
            }),
            methods::DAEMON_STATUS => {
                let cfg = self.config_store.snapshot().await;
                to_json(DaemonSummarySnapshot {
                    pid: std::process::id(),
                    uid: current_uid(),
                    uptime_ms: self.uptime_ms(),
                    default_container: cfg.default_container,
                    total_containers: cfg.containers.len(),
                    running_containers: self.manager.running_count().await,
                    active_tasks: self.task_runner.active_count().await,
                    sideveil_connected: self.sideveil.is_connected(),
                })
            }
            methods::DAEMON_RELOAD_CONFIG => {
                let cfg = self.config_store.reload_from_disk().await?;
                self.manager.sync_running_containers().await;
                self.sideveil.notify_config_changed();
                self.web.notify_config_changed();
                to_json(cfg)
            }
            methods::CONTAINER_LIST => to_json(self.manager.list_containers().await),
            methods::CONTAINER_STATUS => {
                let p: ContainerSelectorParams = parse_optional_params(params)?;
                to_json(
                    self.manager
                        .container_status(p.container.as_deref())
                        .await?,
                )
            }
            methods::CONTAINER_START => {
                let p: ContainerSelectorParams = parse_optional_params(params)?;
                to_json(self.manager.start_container(p.container.as_deref()).await?)
            }
            methods::CONTAINER_STOP => {
                let p: ContainerSelectorParams = parse_optional_params(params)?;
                to_json(self.manager.stop_container(p.container.as_deref()).await?)
            }
            methods::CONTAINER_RESTART => {
                let p: ContainerSelectorParams = parse_optional_params(params)?;
                to_json(
                    self.manager
                        .restart_container(p.container.as_deref())
                        .await?,
                )
            }
            methods::CONTAINER_LOGS => {
                let p: ContainerLogsParams = parse_optional_params(params)?;
                to_json(self.manager.read_logs(p).await?)
            }
            methods::CONTAINER_SYNC_DNS => {
                let p: ContainerSelectorParams = parse_optional_params(params)?;
                let container = self.manager.sync_dns(p.container.as_deref()).await?;
                to_json(SyncDnsResult {
                    container,
                    synced: true,
                })
            }
            methods::CONTAINER_COMPACT => {
                let p: ContainerSelectorParams = parse_optional_params(params)?;
                to_json(
                    self.manager
                        .compact_container(p.container.as_deref())
                        .await?,
                )
            }
            methods::CONTAINER_FSCK => {
                let p: ContainerSelectorParams = parse_optional_params(params)?;
                to_json(self.manager.fsck_container(p.container.as_deref()).await?)
            }
            methods::CONTAINER_BACKUP => {
                let p: BackupParams = parse_required_params(params)?;
                enforce_backup_path_policy(caller, &p.output_path)?;
                to_json(self.manager.backup_container(p).await?)
            }
            methods::CONTAINER_BACKUP_START => {
                let p: BackupParams = parse_required_params(params)?;
                enforce_backup_path_policy(caller, &p.output_path)?;
                to_json(self.manager.backup_container_async(p).await?)
            }
            methods::CONTAINER_CONFIG_GET => {
                let p: ContainerSelectorParams = parse_optional_params(params)?;
                let (container, config, _) = self
                    .manager
                    .resolve_container(p.container.as_deref())
                    .await?;
                to_json(ContainerConfigResult { container, config })
            }
            methods::CONTAINER_CONFIG_PATCH => {
                let p: ContainerConfigPatchParams = parse_required_params(params)?;
                if !matches!(caller, RpcCaller::LocalCtl { .. }) && p.host_runtime == Some(true) {
                    return Err(DaemonError::PermissionDenied(
                        "enabling hostRuntime is restricted to local strata.sock clients"
                            .to_owned(),
                    ));
                }
                let (container, config) = self.config_store.patch_container(p).await?;
                to_json(ContainerConfigResult { container, config })
            }
            methods::CONTAINER_SSH_KEY_LIST => {
                let p: ContainerSelectorParams = parse_optional_params(params)?;
                to_json(self.manager.ssh_keys(p.container.as_deref(), None).await?)
            }
            methods::CONTAINER_SSH_KEY_ADD => {
                let p: SshKeyAddParams = parse_required_params(params)?;
                let target = p.container.clone();
                to_json(self.manager.ssh_keys(target.as_deref(), Some(p)).await?)
            }
            methods::SESSION_ACQUIRE => match caller {
                RpcCaller::LocalCtl { conn_id } => {
                    let p: SessionAcquireParams = parse_optional_params(params)?;
                    to_json(self.manager.acquire_session(conn_id, p).await?)
                }
                RpcCaller::SideVeilBridge | RpcCaller::WebConsole => {
                    Err(DaemonError::PermissionDenied(
                        "session.acquire is restricted to local strata.sock clients".to_owned(),
                    ))
                }
            },
            methods::SESSION_RELEASE => match caller {
                RpcCaller::LocalCtl { .. } => {
                    let p: SessionReleaseParams = parse_required_params(params)?;
                    let released = self.manager.release_session(p).await?;
                    to_json(SessionReleaseResult { released })
                }
                RpcCaller::SideVeilBridge | RpcCaller::WebConsole => {
                    Err(DaemonError::PermissionDenied(
                        "session.release is restricted to local strata.sock clients".to_owned(),
                    ))
                }
            },
            methods::TASK_RUN => {
                let p: TaskRunParams = parse_required_params(params)?;
                to_json(self.task_runner.run_task(p).await?)
            }
            methods::TASK_CANCEL => {
                let p: TaskCancelParams = parse_required_params(params)?;
                let cancelled = self.task_runner.cancel_task(p).await?;
                to_json(TaskCancelResult { cancelled })
            }
            methods::TASK_LIST => to_json(self.task_runner.list_tasks().await),
            methods::TASK_LOGS => {
                let p: TaskLogsParams = parse_required_params(params)?;
                to_json(self.task_runner.task_logs(p).await?)
            }
            methods::HUD_STATE => to_json(self.sideveil.hud_state()),
            methods::HUD_SET => {
                let p: HudSetParams = parse_optional_params(params)?;
                to_json(self.sideveil.update_hud(p))
            }
            methods::EVENTS_SUBSCRIBE => to_json(EventsSubscribeResult { subscribed: true }),
            other => match self.dispatch_extended(other, params).await? {
                Some(val) => Ok(val),
                None => Err(DaemonError::MethodNotFound(other.to_owned())),
            },
        }
    }

    fn uptime_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }
}

pub(super) fn parse_optional_params<T: DeserializeOwned + Default>(
    params: Option<Value>,
) -> DaemonResult<T> {
    match params {
        None | Some(Value::Null) => Ok(T::default()),
        Some(v) => serde_json::from_value(v)
            .map_err(|e| DaemonError::InvalidParams(format!("invalid params: {e}"))),
    }
}

pub(super) fn parse_required_params<T: DeserializeOwned>(params: Option<Value>) -> DaemonResult<T> {
    let Some(v) = params else {
        return Err(DaemonError::InvalidParams(
            "missing required params object".to_owned(),
        ));
    };
    serde_json::from_value(v)
        .map_err(|e| DaemonError::InvalidParams(format!("invalid params: {e}")))
}

pub(super) fn to_json<T: serde::Serialize>(value: T) -> DaemonResult<Value> {
    serde_json::to_value(value).map_err(|e| DaemonError::Internal(format!("serialize result: {e}")))
}

const REMOTE_BACKUP_JAIL: &str = "/data/local/strata/backups";

fn enforce_backup_path_policy(caller: RpcCaller, output_path: &str) -> DaemonResult<()> {
    if matches!(caller, RpcCaller::LocalCtl { .. }) {
        return Ok(());
    }
    let path = std::path::Path::new(output_path.trim());
    let jail = std::path::Path::new(REMOTE_BACKUP_JAIL);
    strata_runtime::provision::ensure_within_jail(jail, path)
        .map_err(|e| DaemonError::PermissionDenied(format!("remote backup path rejected: {e}")))?;
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && meta.file_type().is_symlink()
    {
        return Err(DaemonError::PermissionDenied(
            "remote backup target must not be a symlink".to_owned(),
        ));
    }
    Ok(())
}

fn current_uid() -> u32 {
    // SAFETY: `getuid(2)` is always safe and has no preconditions.
    unsafe { libc::getuid() }
}
