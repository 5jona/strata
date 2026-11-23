//! Transport-agnostic JSON-RPC 2.0 dispatcher shared by `strata.sock` and the SideVeil bridge.

use std::sync::Arc;
use std::time::Instant;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use strata_protocol::rpc::{
    BackupParams, ContainerConfigPatchParams, ContainerConfigResult, ContainerLogsParams,
    ContainerSelectorParams, DaemonHealthResult, HudSetParams, PingResult, SessionAcquireParams,
    SessionReleaseParams, SshKeyAddParams, TaskCancelParams, TaskLogsParams, TaskRunParams,
    error_codes, methods,
};
use strata_protocol::{DaemonSummarySnapshot, RpcRequest, RpcResponse};

use crate::config_store::ConfigStore;
use crate::manager::ContainerManager;
use crate::sideveil_seam::SideVeilBridgeSeam;
use crate::task_runner::TaskRunner;

/// Identifies the transport origin of an incoming JSON-RPC request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RpcCaller {
    /// Direct root client connected over `strata.sock` (e.g., `strata` CLI).
    LocalCtl { conn_id: u64 },
    /// Remote plugin client forwarded over SideVeil `hub.sock`.
    SideVeilBridge,
}

/// Central RPC dispatcher decoupled from Unix socket transport.
#[derive(Debug)]
pub(crate) struct Dispatcher {
    started_at: Instant,
    config_store: Arc<ConfigStore>,
    manager: Arc<ContainerManager>,
    task_runner: Arc<TaskRunner>,
    sideveil: Arc<SideVeilBridgeSeam>,
}

impl Dispatcher {
    #[must_use]
    pub(crate) fn new(
        started_at: Instant,
        config_store: Arc<ConfigStore>,
        manager: Arc<ContainerManager>,
        task_runner: Arc<TaskRunner>,
        sideveil: Arc<SideVeilBridgeSeam>,
    ) -> Self {
        Self {
            started_at,
            config_store,
            manager,
            task_runner,
            sideveil,
        }
    }

    /// Handles a single JSON-RPC 2.0 request and returns the serialized response envelope.
    pub(crate) async fn handle(&self, caller: RpcCaller, req: RpcRequest) -> RpcResponse {
        let id = req.id.clone();
        match self.dispatch(caller, &req.method, req.params).await {
            Ok(result) => RpcResponse::ok(id, result),
            Err((code, message)) => RpcResponse::err(id, code, message),
        }
    }

    async fn dispatch(
        &self,
        caller: RpcCaller,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, (i32, String)> {
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
                Ok(json!({ "container": container, "synced": true }))
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
                to_json(self.manager.backup_container(p).await?)
            }
            methods::CONTAINER_BACKUP_START => {
                let p: BackupParams = parse_required_params(params)?;
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
                RpcCaller::SideVeilBridge => Err((
                    error_codes::PERMISSION_DENIED,
                    "session.acquire is restricted to local strata.sock clients".to_owned(),
                )),
            },
            methods::SESSION_RELEASE => match caller {
                RpcCaller::LocalCtl { .. } => {
                    let p: SessionReleaseParams = parse_required_params(params)?;
                    let released = self.manager.release_session(p).await?;
                    Ok(json!({ "released": released }))
                }
                RpcCaller::SideVeilBridge => Err((
                    error_codes::PERMISSION_DENIED,
                    "session.release is restricted to local strata.sock clients".to_owned(),
                )),
            },
            methods::TASK_RUN => {
                let p: TaskRunParams = parse_required_params(params)?;
                to_json(self.task_runner.run_task(p).await?)
            }
            methods::TASK_CANCEL => {
                let p: TaskCancelParams = parse_required_params(params)?;
                let cancelled = self.task_runner.cancel_task(p).await?;
                Ok(json!({ "cancelled": cancelled }))
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
            methods::EVENTS_SUBSCRIBE => Ok(json!({ "subscribed": true })),
            other => Err((
                error_codes::METHOD_NOT_FOUND,
                format!("unknown method '{other}'"),
            )),
        }
    }

    fn uptime_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }
}

fn parse_optional_params<T: DeserializeOwned + Default>(
    params: Option<Value>,
) -> Result<T, (i32, String)> {
    match params {
        None | Some(Value::Null) => Ok(T::default()),
        Some(v) => serde_json::from_value(v)
            .map_err(|e| (error_codes::INVALID_PARAMS, format!("invalid params: {e}"))),
    }
}

fn parse_required_params<T: DeserializeOwned>(params: Option<Value>) -> Result<T, (i32, String)> {
    let Some(v) = params else {
        return Err((
            error_codes::INVALID_PARAMS,
            "missing required params object".to_owned(),
        ));
    };
    serde_json::from_value(v)
        .map_err(|e| (error_codes::INVALID_PARAMS, format!("invalid params: {e}")))
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, (i32, String)> {
    serde_json::to_value(value).map_err(|e| {
        (
            error_codes::INTERNAL_ERROR,
            format!("serialize result: {e}"),
        )
    })
}

fn current_uid() -> u32 {
    // SAFETY: `getuid(2)` is always safe and has no preconditions.
    unsafe { libc::getuid() }
}
