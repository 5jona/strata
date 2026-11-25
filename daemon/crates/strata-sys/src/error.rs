//! Structured error types for Linux and Android syscall wrappers.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Result alias for `strata-sys` operations.
pub type SysResult<T> = Result<T, SysError>;

/// Low-level system error returned by `strata-sys`.
#[derive(Debug, Error)]
pub enum SysError {
    #[error("{op} failed: {source}")]
    Io {
        op: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("{op} on '{path}' failed: {source}")]
    PathIo {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("path '{0}' contains interior NUL byte")]
    InvalidCString(String),
    #[error("image '{image}' is already mounted at '{existing_mount}' via '{loop_dev}'")]
    ImageBusy {
        image: PathBuf,
        existing_mount: PathBuf,
        loop_dev: PathBuf,
    },
    #[error("invalid argument for {op}: {detail}")]
    InvalidArgument { op: &'static str, detail: String },
}

impl SysError {
    pub(crate) fn last_os(op: &'static str) -> Self {
        Self::Io {
            op,
            source: io::Error::last_os_error(),
        }
    }

    pub(crate) fn path_os(op: &'static str, path: impl Into<PathBuf>) -> Self {
        Self::PathIo {
            op,
            path: path.into(),
            source: io::Error::last_os_error(),
        }
    }

    pub(crate) fn path_io(op: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::PathIo {
            op,
            path: path.into(),
            source,
        }
    }
}
