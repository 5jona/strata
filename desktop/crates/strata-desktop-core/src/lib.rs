//! Platform-agnostic core services for Strata Desktop (multi-device registry,
//! ADB discovery & tunnel management, native terminal/VS Code launcher, and HTTP probe).

pub mod adb;
pub mod devices;
pub mod launcher;
pub mod probe;

/// Default embedded HTTP/WebSocket control-plane port exposed by `stratad`.
pub const DEFAULT_REMOTE_API_PORT: u16 = 7681;

/// Default Dropbear SSH listen port inside Strata containers.
pub const DEFAULT_SSH_PORT: u16 = 2222;

/// Canonical Android path storing the `stratad` Bearer token (`0600 root:root`).
pub const REMOTE_WEB_TOKEN_PATH: &str = "/data/adb/strata/run/web.token";
