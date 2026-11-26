//! Binary-safe `adb-host` file push/pull using `/data/local/tmp` staging with guaranteed cleanup.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::process::Command;

use super::adb_forward::{shell_quote, wrap_su_command};

pub(super) const TRANSFER_TIMEOUT: Duration = Duration::from_secs(90);

pub(super) fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

pub(super) fn temp_local_path(dst: &Path) -> PathBuf {
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    parent.join(format!(
        ".{name}.strata-tmp-{}-{}",
        std::process::id(),
        now_nanos()
    ))
}

pub(super) async fn adb_host_push(
    serial: &str,
    local_path: &str,
    remote_path: &str,
) -> Result<u64, String> {
    let meta = tokio::fs::metadata(local_path)
        .await
        .map_err(|e| format!("stat '{local_path}' failed: {e}"))?;
    let stage = format!(
        "/data/local/tmp/strata_xfer_push_{}_{}",
        std::process::id(),
        now_nanos()
    );
    let push_out = tokio::time::timeout(
        TRANSFER_TIMEOUT,
        Command::new("adb")
            .args(["-s", serial, "push", local_path, &stage])
            .output(),
    )
    .await
    .map_err(|_| "adb push timed out".to_owned())?
    .map_err(|e| format!("spawn adb push failed: {e}"))?;

    if !push_out.status.success() {
        cleanup_adb_stage(serial, &stage).await;
        return Err(format!(
            "adb push failed: {}",
            String::from_utf8_lossy(&push_out.stderr).trim()
        ));
    }

    let q_stage = shell_quote(&stage);
    let q_remote = shell_quote(remote_path);
    let move_cmd = format!(
        "/system/bin/sh -c '/system/bin/cat {q_stage} > {q_remote} && /system/bin/rm -f {q_stage}'"
    );
    let mv_out = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("adb")
            .args(["-s", serial, "shell", &wrap_su_command(&move_cmd)])
            .output(),
    )
    .await
    .map_err(|_| "adb host move timed out".to_owned())?
    .map_err(|e| format!("spawn adb shell move failed: {e}"))?;

    if !mv_out.status.success() {
        cleanup_adb_stage(serial, &stage).await;
        return Err(format!(
            "adb host write failed: {}",
            String::from_utf8_lossy(&mv_out.stderr).trim()
        ));
    }
    Ok(meta.len())
}

pub(super) async fn adb_host_pull(
    serial: &str,
    remote_path: &str,
    local_path: &str,
) -> Result<u64, String> {
    let dst = Path::new(local_path);
    if let Some(parent) = dst.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let stage = format!(
        "/data/local/tmp/strata_xfer_pull_{}_{}",
        std::process::id(),
        now_nanos()
    );
    let q_stage = shell_quote(&stage);
    let q_remote = shell_quote(remote_path);
    let stage_cmd = format!(
        "/system/bin/sh -c '/system/bin/cp {q_remote} {q_stage} && /system/bin/chmod 0644 {q_stage}'"
    );
    let cp_out = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("adb")
            .args(["-s", serial, "shell", &wrap_su_command(&stage_cmd)])
            .output(),
    )
    .await
    .map_err(|_| "adb host stage timed out".to_owned())?
    .map_err(|e| format!("spawn adb shell stage failed: {e}"))?;

    if !cp_out.status.success() {
        cleanup_adb_stage(serial, &stage).await;
        return Err(format!(
            "adb host read failed: {}",
            String::from_utf8_lossy(&cp_out.stderr).trim()
        ));
    }

    let tmp = temp_local_path(dst);
    let tmp_str = tmp.to_string_lossy().into_owned();
    let pull_out = tokio::time::timeout(
        TRANSFER_TIMEOUT,
        Command::new("adb")
            .args(["-s", serial, "pull", &stage, &tmp_str])
            .output(),
    )
    .await;
    cleanup_adb_stage(serial, &stage).await;

    match pull_out {
        Ok(Ok(o)) if o.status.success() => {}
        Ok(Ok(o)) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(format!(
                "adb pull failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(format!("spawn adb pull failed: {e}"));
        }
        Err(_) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err("adb pull timed out".to_owned());
        }
    }
    let size = tokio::fs::metadata(&tmp)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    tokio::fs::rename(&tmp, dst)
        .await
        .map_err(|e| format!("rename to '{local_path}' failed: {e}"))?;
    Ok(size)
}

async fn cleanup_adb_stage(serial: &str, stage: &str) {
    let rm = format!("/system/bin/rm -f {}", shell_quote(stage));
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new("adb")
            .args(["-s", serial, "shell", &wrap_su_command(&rm)])
            .output(),
    )
    .await;
}
