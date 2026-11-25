//! Structured runtime errors with direct mapping to Strata JSON-RPC error codes.

use std::io;
use std::path::PathBuf;

use strata_protocol::rpc::error_codes;
use strata_sys::SysError;
use thiserror::Error;

/// Result alias for `strata-runtime` operations.
pub type RuntimeResult<T> = Result<T, RuntimeError>;

/// High-level container runtime error.
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("rootfs image not found at '{0}'")]
    ImageMissing(PathBuf),
    #[error("image '{image}' is already mounted at '{existing_mount}' via '{loop_dev}'")]
    ImageBusy {
        image: PathBuf,
        existing_mount: PathBuf,
        loop_dev: PathBuf,
    },
    #[error("invalid rootfs at '{0}': {1}")]
    InvalidRootfs(PathBuf, &'static str),
    #[error("fsck failed on '{path}' with exit code {code}: {detail}")]
    FsckFailed {
        path: PathBuf,
        code: i32,
        detail: String,
    },
    #[error("invalid command or argument: {0}")]
    InvalidArgument(String),
    #[error("container process {0} is not running")]
    ProcessNotRunning(u32),
    #[error("system call failed: {0}")]
    Sys(#[from] SysError),
    #[error("I/O error during {op}: {source}")]
    Io {
        op: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

impl RuntimeError {
    /// Converts a `SysError`, promoting `SysError::ImageBusy` directly.
    #[must_use]
    pub fn from_sys(err: SysError) -> Self {
        match err {
            SysError::ImageBusy {
                image,
                existing_mount,
                loop_dev,
            } => Self::ImageBusy {
                image,
                existing_mount,
                loop_dev,
            },
            other => Self::Sys(other),
        }
    }

    /// Maps this runtime error to the canonical JSON-RPC error code.
    #[must_use]
    pub fn rpc_code(&self) -> i32 {
        match self {
            Self::ImageBusy { .. } => error_codes::IMAGE_BUSY,
            Self::InvalidArgument(_) => error_codes::INVALID_PARAMS,
            Self::ProcessNotRunning(_) => error_codes::CONTAINER_NOT_RUNNING,
            Self::Sys(SysError::ImageBusy { .. }) => error_codes::IMAGE_BUSY,
            _ => error_codes::INTERNAL_ERROR,
        }
    }
}
