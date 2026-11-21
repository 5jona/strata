//! Minimal audited Linux and Android system-call wrappers and `/proc`/`/sys` parsers.

pub mod caps;
pub mod cgroup;
pub mod devfs;
pub mod error;
pub mod inotify;
pub mod loopdev;
pub mod mount;
pub mod namespace;
pub mod net;
pub mod pidfd;
pub mod procfs;
pub mod selinux;

pub use error::{SysError, SysResult};
