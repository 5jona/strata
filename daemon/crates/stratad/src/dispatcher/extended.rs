//! Extended RPC method routing for capabilities, web server, distro catalog,
//! container lifecycle/jobs, SSH key removal, and managed container services.

use serde::de::DeserializeOwned;
use serde_json::Value;
use strata_protocol::rpc::{
    ContainerCloneParams, ContainerCreateParams, ContainerDeleteParams, ContainerJobSelectorParams,
    ContainerResizeParams, ContainerSelectorParams, DaemonLogsParams, DistroCatalogParams,
    ServiceLogsParams, ServiceSelectorParams, ServiceUpsertParams, SshKeyRemoveParams,
    ToolsBootstrapParams, ToolsCacheCleanParams, ToolsCatalogParams, ToolsConfigGetParams,
    ToolsConfigSetParams, ToolsDoctorParams, ToolsInstallParams, ToolsListParams,
    ToolsStatusParams, ToolsUninstallParams, ToolsUpgradeParams, WebConfigureParams, methods,
};
use strata_sys::kernel_caps;

use crate::dispatcher::{Dispatcher, parse_optional_params, parse_required_params, to_json};
use crate::error::{DaemonError, DaemonResult};

impl Dispatcher {
    /// Handles extended methods (capabilities, web, catalog, provisioning jobs, services).
    /// Returns `Ok(Some(val))` when handled, or `Ok(None)` when `method` is unknown.
    pub(super) async fn dispatch_extended(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> DaemonResult<Option<Value>> {
        let val = match method {
            methods::DAEMON_CAPABILITIES => {
                let caps = tokio::task::spawn_blocking(kernel_caps::probe_kernel_capabilities)
                    .await
                    .map_err(|e| DaemonError::join("probe kernel caps join", e))?;
                to_json(caps)?
            }
            methods::DAEMON_LOGS => {
                let p: DaemonLogsParams = parse_optional_params(params)?;
                to_json(self.manager.read_daemon_logs(p).await?)?
            }
            methods::WEB_STATUS => to_json(self.web.status().await)?,
            methods::WEB_CONFIGURE => {
                let p: WebConfigureParams = parse_optional_params(params)?;
                to_json(self.web.configure(p).await?)?
            }
            methods::WEB_ROTATE_TOKEN => to_json(self.web.rotate_token().await?)?,
            methods::DISTRO_CATALOG => {
                let p: DistroCatalogParams = parse_optional_params(params)?;
                to_json(self.catalog.fetch_catalog(p).await?)?
            }
            methods::CONTAINER_CREATE => {
                let p: ContainerCreateParams = parse_required_params(params)?;
                to_json(self.jobs.start_create_job(p).await?)?
            }
            methods::CONTAINER_DELETE => {
                let p: ContainerDeleteParams = parse_required_params(params)?;
                to_json(self.manager.delete_container(p).await?)?
            }
            methods::CONTAINER_RESIZE => {
                let p: ContainerResizeParams = parse_required_params(params)?;
                to_json(self.manager.resize_container(p).await?)?
            }
            methods::CONTAINER_CLONE => {
                let p: ContainerCloneParams = parse_required_params(params)?;
                to_json(self.manager.clone_container(p).await?)?
            }
            methods::CONTAINER_JOB_STATUS => {
                let p: ContainerJobSelectorParams = parse_required_params(params)?;
                to_json(self.jobs.job_status(&p.job_id).await?)?
            }
            methods::CONTAINER_JOB_CANCEL => {
                let p: ContainerJobSelectorParams = parse_required_params(params)?;
                to_json(self.jobs.job_cancel(&p.job_id).await?)?
            }
            methods::CONTAINER_SSH_KEY_REMOVE => {
                let p: SshKeyRemoveParams = parse_required_params(params)?;
                to_json(self.manager.remove_ssh_key(p).await?)?
            }
            methods::SERVICE_LIST => {
                let p: ContainerSelectorParams = parse_optional_params(params)?;
                to_json(self.services.list(p.container.as_deref()).await?)?
            }
            methods::SERVICE_UPSERT => {
                let p: ServiceUpsertParams = parse_required_params(params)?;
                to_json(self.services.upsert(p).await?)?
            }
            methods::SERVICE_DELETE => {
                let p: ServiceSelectorParams = parse_required_params(params)?;
                to_json(self.services.delete(p).await?)?
            }
            methods::SERVICE_START => {
                let p: ServiceSelectorParams = parse_required_params(params)?;
                to_json(self.services.start(p).await?)?
            }
            methods::SERVICE_STOP => {
                let p: ServiceSelectorParams = parse_required_params(params)?;
                to_json(self.services.stop(p).await?)?
            }
            methods::SERVICE_RESTART => {
                let p: ServiceSelectorParams = parse_required_params(params)?;
                to_json(self.services.restart(p).await?)?
            }
            methods::SERVICE_LOGS => {
                let p: ServiceLogsParams = parse_required_params(params)?;
                to_json(self.services.logs(p).await?)?
            }
            methods::TOOLS_STATUS => {
                let p: ToolsStatusParams = parse_optional_params(params)?;
                to_json(self.tools.status(p).await?)?
            }
            methods::TOOLS_LIST => {
                let p: ToolsListParams = parse_optional_params(params)?;
                to_json(self.tools.list(p).await?)?
            }
            methods::TOOLS_CATALOG => {
                let p: ToolsCatalogParams = parse_optional_params(params)?;
                to_json(self.tools.catalog(p).await?)?
            }
            methods::TOOLS_CONFIG_GET => {
                let p: ToolsConfigGetParams = parse_optional_params(params)?;
                to_json(self.tools.config_get(p).await?)?
            }
            methods::TOOLS_CONFIG_SET => {
                let p: ToolsConfigSetParams = parse_required_params(params)?;
                to_json(self.tools.config_set(p).await?)?
            }
            methods::TOOLS_BOOTSTRAP => {
                let p: ToolsBootstrapParams = parse_optional_params(params)?;
                to_json(self.tools.bootstrap(p).await?)?
            }
            methods::TOOLS_INSTALL => {
                let p: ToolsInstallParams = parse_required_params(params)?;
                to_json(self.tools.install(p).await?)?
            }
            methods::TOOLS_UNINSTALL => {
                let p: ToolsUninstallParams = parse_required_params(params)?;
                to_json(self.tools.uninstall(p).await?)?
            }
            methods::TOOLS_UPGRADE => {
                let p: ToolsUpgradeParams = parse_optional_params(params)?;
                to_json(self.tools.upgrade(p).await?)?
            }
            methods::TOOLS_DOCTOR => {
                let p: ToolsDoctorParams = parse_optional_params(params)?;
                to_json(self.tools.doctor(p).await?)?
            }
            methods::TOOLS_CACHE_CLEAN => {
                let p: ToolsCacheCleanParams = parse_optional_params(params)?;
                to_json(self.tools.cache_clean(p).await?)?
            }
            _ => return Ok(None),
        };
        Ok(Some(val))
    }
}

#[allow(dead_code)]
fn _assert_de<T: DeserializeOwned>() {}
