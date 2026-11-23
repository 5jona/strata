//! Platform-agnostic configuration, capability profiles, lifecycle state machine,
//! and JSON-RPC 2.0 wire types shared across Strata components.

pub mod capabilities;
pub mod config;
pub mod constants;
pub mod generated;
pub mod rpc;
pub mod state;

pub use capabilities::{CapMask, CapProfile};
pub use config::{
    BindMount, ConfigValidationError, ContainerConfig, SideVeilBridgeConfig, SshConfig,
    StrataConfig, validate_container_id,
};
pub use rpc::{EventEnvelope, RpcError, RpcRequest, RpcResponse};
pub use state::{
    ContainerDiskStats, ContainerPhase, ContainerStatusSnapshot, DaemonSummarySnapshot,
    RestartBackoff,
};
