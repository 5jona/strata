//! Typed daemon errors with deterministic mapping to Strata JSON-RPC 2.0 error codes.

use strata_protocol::ConfigValidationError;
use strata_protocol::rpc::error_codes;
use strata_runtime::RuntimeError;
use thiserror::Error;

/// Result alias for `stratad` control-plane and lifecycle operations.
pub(crate) type DaemonResult<T> = Result<T, DaemonError>;

/// Structured error returned by `stratad` managers, config store, task runner, and RPC dispatcher.
#[derive(Debug, Error)]
pub(crate) enum DaemonError {
    #[error("unknown method '{0}'")]
    MethodNotFound(String),
    #[error("{0}")]
    InvalidParams(String),
    #[error("{0}")]
    PermissionDenied(String),
    #[error("container '{0}' not found")]
    ContainerNotFound(String),
    #[error("container '{0}' missing after patch")]
    ContainerMissingAfterPatch(String),
    #[error("stop running container '{0}' before entering full host-runtime debug mode")]
    ContainerAlreadyRunning(String),
    #[error("{0}")]
    ContainerNotRunning(String),
    #[error("container '{0}' has active temporary session leases")]
    TempLeaseActive(String),
    #[error("container '{0}' already exists")]
    ContainerAlreadyExists(String),
    #[error("task '{0}' not found")]
    TaskNotFound(String),
    #[error("job '{0}' not found")]
    JobNotFound(String),
    #[error("service '{0}' not found")]
    ServiceNotFound(String),
    #[error("{0}")]
    InvalidState(String),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("{0}")]
    Internal(String),
}

impl From<ConfigValidationError> for DaemonError {
    fn from(err: ConfigValidationError) -> Self {
        Self::InvalidParams(err.to_string())
    }
}

impl DaemonError {
    /// Wraps a `tokio::task::JoinError` as an internal daemon error preserving the operation label.
    #[must_use]
    pub(crate) fn join(label: impl std::fmt::Display, err: tokio::task::JoinError) -> Self {
        Self::Internal(format!("{label}: {err}"))
    }

    /// Maps this daemon error to its canonical Strata JSON-RPC 2.0 error code.
    #[must_use]
    pub(crate) fn code(&self) -> i32 {
        match self {
            Self::MethodNotFound(_) => error_codes::METHOD_NOT_FOUND,
            Self::InvalidParams(_) => error_codes::INVALID_PARAMS,
            Self::PermissionDenied(_) => error_codes::PERMISSION_DENIED,
            Self::ContainerNotFound(_) | Self::ContainerMissingAfterPatch(_) => {
                error_codes::CONTAINER_NOT_FOUND
            }
            Self::ContainerAlreadyRunning(_) => error_codes::CONTAINER_ALREADY_RUNNING,
            Self::ContainerNotRunning(_) => error_codes::CONTAINER_NOT_RUNNING,
            Self::TempLeaseActive(_) => error_codes::IMAGE_BUSY,
            Self::ContainerAlreadyExists(_) => error_codes::CONTAINER_ALREADY_EXISTS,
            Self::TaskNotFound(_) => error_codes::TASK_NOT_FOUND,
            Self::JobNotFound(_) => error_codes::JOB_NOT_FOUND,
            Self::ServiceNotFound(_) => error_codes::SERVICE_NOT_FOUND,
            Self::InvalidState(_) => error_codes::INVALID_STATE,
            Self::Runtime(err) => err.rpc_code(),
            Self::Internal(_) => error_codes::INTERNAL_ERROR,
        }
    }

    /// Converts this error into the `(code, message)` pair expected by `RpcResponse::err`.
    #[must_use]
    pub(crate) fn to_rpc_tuple(&self) -> (i32, String) {
        (self.code(), self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_daemon_errors_to_canonical_rpc_codes_and_messages() {
        assert_eq!(
            DaemonError::MethodNotFound("foo.bar".into()).to_rpc_tuple(),
            (
                error_codes::METHOD_NOT_FOUND,
                "unknown method 'foo.bar'".to_owned()
            )
        );
        assert_eq!(
            DaemonError::ContainerNotFound("main".into()).to_rpc_tuple(),
            (
                error_codes::CONTAINER_NOT_FOUND,
                "container 'main' not found".to_owned()
            )
        );
        assert_eq!(
            DaemonError::TempLeaseActive("main".into()).to_rpc_tuple(),
            (
                error_codes::IMAGE_BUSY,
                "container 'main' has active temporary session leases".to_owned()
            )
        );
        assert_eq!(
            DaemonError::TaskNotFound("task-1".into()).to_rpc_tuple(),
            (
                error_codes::TASK_NOT_FOUND,
                "task 'task-1' not found".to_owned()
            )
        );
        let validation_err: DaemonError = ConfigValidationError::InvalidSchemaVersion(0).into();
        assert_eq!(validation_err.code(), error_codes::INVALID_PARAMS);
    }
}
