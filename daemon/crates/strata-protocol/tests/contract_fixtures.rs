//! Golden wire fixture serde round-trip and default config parity tests.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use strata_protocol::rpc::{
    AttachLease, ContainerConfigPatchParams, ContainerStateChangedPayload, MaintenanceDonePayload,
    TaskExitPayload, TaskLogsResult,
};
use strata_protocol::{
    ContainerConfig, ContainerStatusSnapshot, DaemonSummarySnapshot, StrataConfig,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("strata-protocol must live under <repo>/daemon/crates/")
        .to_path_buf()
}

fn fixture_json(name: &str) -> Value {
    let path = repo_root().join("contracts/fixtures").join(name);
    let raw = fs::read_to_string(&path).expect("read fixture file");
    serde_json::from_str(&raw).expect("parse fixture json")
}

fn assert_roundtrip<T: DeserializeOwned + Serialize>(fixture_name: &str) -> T {
    let expected = fixture_json(fixture_name);
    let parsed: T = serde_json::from_value(expected.clone()).expect("deserialize fixture");
    let reencoded = serde_json::to_value(&parsed).expect("re-serialize fixture");
    assert_eq!(reencoded, expected, "round-trip drift in {fixture_name}");
    parsed
}

#[test]
fn valid_golden_fixtures_roundtrip_without_wire_drift() {
    let cfg: StrataConfig = assert_roundtrip("StrataConfig.default.json");
    assert!(cfg.validate().is_ok());
    assert_eq!(cfg, StrataConfig::default());

    let l5: ContainerConfig = assert_roundtrip("ContainerConfig.l5-diag.json");
    assert_eq!(l5.extra_mounts.len(), 3);

    let patch: ContainerConfigPatchParams =
        assert_roundtrip("ContainerConfigPatchParams.full.json");
    assert!(patch.image_path.is_some());
    assert!(patch.mount_dir.is_some());

    let _: ContainerStatusSnapshot = assert_roundtrip("ContainerStatusSnapshot.running.json");
    let _: DaemonSummarySnapshot = assert_roundtrip("DaemonSummarySnapshot.healthy.json");
    let _: ContainerStateChangedPayload =
        assert_roundtrip("ContainerStateChangedPayload.started.json");
    let _: MaintenanceDonePayload = assert_roundtrip("MaintenanceDonePayload.ok.json");
    let _: TaskLogsResult = assert_roundtrip("TaskLogsResult.completed.json");
    let _: TaskExitPayload = assert_roundtrip("TaskExitPayload.clean.json");
    let _: AttachLease = assert_roundtrip("AttachLease.attached.json");
}

#[test]
fn invalid_golden_fixtures_are_rejected_by_rust_deserializers() {
    let bad_port = fixture_json("ContainerConfig.invalid-port.json");
    let bad_port_data = bad_port.get("data").cloned().expect("wrapper data");
    assert!(serde_json::from_value::<ContainerConfig>(bad_port_data).is_err());

    let bad_phase = fixture_json("ContainerStatusSnapshot.invalid-phase.json");
    let bad_phase_data = bad_phase.get("data").cloned().expect("wrapper data");
    assert!(serde_json::from_value::<ContainerStatusSnapshot>(bad_phase_data).is_err());
}

#[test]
fn shipped_config_json_matches_strata_config_default() {
    let raw = fs::read_to_string(repo_root().join("module/config.json"))
        .expect("read module/config.json");
    let parsed: StrataConfig = serde_json::from_str(&raw).expect("parse module/config.json");
    assert!(parsed.validate().is_ok());
    assert_eq!(parsed, StrataConfig::default());
}
