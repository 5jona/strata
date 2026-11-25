//! Layout/env/sandbox diagnostics (`tools.doctor`) and bounded cache cleanup (`tools.cacheClean`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use strata_protocol::constants::{
    CONTAINER_ETC_ENVIRONMENT_PATH, CONTAINER_LAYOUT_PATH, CONTAINER_LAYOUT_SCHEMA_VERSION,
    CONTAINER_MISE_CACHE_DIR, CONTAINER_MISE_CONFIG_FILE, CONTAINER_PROFILE_ENV_PATH,
    CONTAINER_UV_CACHE_DIR,
};
use strata_protocol::rpc::{
    ToolDiagnosticItem, ToolDiagnosticStatus, ToolsCacheCleanParams, ToolsCacheCleanResult,
    ToolsDoctorParams, ToolsDoctorResult,
};
use strata_sys::mount;
use strata_sys::rootfs_io::RootDir;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::ToolsManager;
use crate::tools::exec::{
    load_container_tool_env_from_mount, read_root_utf8, run_container_command_bounded,
};
use crate::tools::status_parsers::dir_size_bytes;

pub(super) async fn query_doctor(
    tools: &ToolsManager,
    params: ToolsDoctorParams,
) -> DaemonResult<ToolsDoctorResult> {
    let (container, entry, _) = tools
        .manager
        .resolve_container(params.container.as_deref())
        .await?;
    let mount_dir = PathBuf::from(&entry.mount_dir);
    let effective_init = entry.effective_init_command();

    let mut items =
        tokio::task::spawn_blocking(move || collect_host_diagnostics(&mount_dir, &effective_init))
            .await
            .map_err(|e| DaemonError::join("tools.doctor fs join", e))?;

    let mut mise_doctor_json = None;
    if let Ok((_, ns_pid, cfg)) = tools.manager.running_target(Some(&container)).await {
        let mdir = PathBuf::from(&cfg.mount_dir);
        let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mdir))
            .await
            .map_err(|e| DaemonError::join("tools.doctor env join", e))?;

        if let Ok(sh_out) = run_container_command_bounded(
            &tools.self_exe,
            &container,
            ns_pid,
            &cfg,
            vec![
                "/bin/sh".to_owned(),
                "-lc".to_owned(),
                "printf 'PWD=%s\\nMISE=%s\\nPATH=%s\\n' \"$PWD\" \"$(command -v mise || true)\" \"$PATH\""
                    .to_owned(),
            ],
            env.clone(),
            Duration::from_secs(4),
        )
        .await
        {
            let has_mise = sh_out
                .stdout
                .lines()
                .any(|l| l.starts_with("MISE=/opt/strata/bin/mise"));
            let has_shims = sh_out
                .stdout
                .lines()
                .any(|l| l.starts_with("PATH=") && l.contains("/opt/strata/mise/shims"));
            items.push(ToolDiagnosticItem {
                id: "login-shell-env".to_owned(),
                category: "environment".to_owned(),
                status: if has_mise && has_shims {
                    ToolDiagnosticStatus::Ok
                } else {
                    ToolDiagnosticStatus::Warn
                },
                summary: if has_mise && has_shims {
                    "Non-interactive login shell exposes /opt/strata/bin/mise and shims on PATH"
                        .to_owned()
                } else {
                    format!(
                        "Login shell PATH/mise incomplete: {}",
                        sh_out.stdout.replace('\n', "; ")
                    )
                },
                remediation: (!has_mise).then(|| {
                    "Run tools.bootstrap or `strata tools bootstrap` to install mise into /opt/strata/bin"
                        .to_owned()
                }),
            });
        }

        if let Ok(doc_out) = run_container_command_bounded(
            &tools.self_exe,
            &container,
            ns_pid,
            &cfg,
            vec!["mise".to_owned(), "doctor".to_owned(), "--json".to_owned()],
            env,
            Duration::from_secs(6),
        )
        .await
        {
            // `mise doctor --json` intentionally exits 1 when diagnostics report issues while still writing valid JSON.
            let raw_json = doc_out.stdout.trim();
            if !raw_json.is_empty() {
                if let Some(diag) = parse_mise_doctor_json(raw_json) {
                    items.push(diag);
                }
                mise_doctor_json = Some(raw_json.to_owned());
            } else {
                items.push(ToolDiagnosticItem {
                    id: "mise-doctor".to_owned(),
                    category: "mise".to_owned(),
                    status: ToolDiagnosticStatus::Warn,
                    summary: "mise binary is missing or returned empty doctor output".to_owned(),
                    remediation: Some("Run tools.bootstrap to install mise".to_owned()),
                });
            }
        }
    }

    let ok = !items
        .iter()
        .any(|i| i.status == ToolDiagnosticStatus::Error);
    Ok(ToolsDoctorResult {
        container,
        ok,
        items,
        mise_doctor_json,
    })
}

pub(super) async fn clean_cache(
    tools: &ToolsManager,
    params: ToolsCacheCleanParams,
) -> DaemonResult<ToolsCacheCleanResult> {
    let (container, entry, _) = tools
        .manager
        .resolve_container(params.container.as_deref())
        .await?;
    let _guard = tools.try_acquire_write_lock(&container).await?;
    let mount_dir = PathBuf::from(&entry.mount_dir);
    let clean_mise = params.clean_mise;
    let clean_uv = params.clean_uv;
    let clean_apt = params.clean_apt;

    let before_bytes = tokio::task::spawn_blocking({
        let mdir = mount_dir.clone();
        move || measure_cleanable_bytes(&mdir)
    })
    .await
    .map_err(|e| DaemonError::join("tools.cacheClean before join", e))??;

    tokio::task::spawn_blocking({
        let mdir = mount_dir.clone();
        move || {
            if clean_mise {
                clear_dir_contents(&mdir.join(CONTAINER_MISE_CACHE_DIR.trim_start_matches('/')));
            }
            if clean_uv {
                clear_dir_contents(&mdir.join(CONTAINER_UV_CACHE_DIR.trim_start_matches('/')));
            }
        }
    })
    .await
    .map_err(|e| DaemonError::join("tools.cacheClean clear join", e))?;

    if let Ok((_, ns_pid, cfg)) = tools.manager.running_target(Some(&container)).await {
        if clean_apt {
            let _ = run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec!["apt-get".to_owned(), "clean".to_owned()],
                BTreeMap::new(),
                Duration::from_secs(15),
            )
            .await;
        }
        if params.prune_unused {
            let _ = run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec![
                    "mise".to_owned(),
                    "prune".to_owned(),
                    "--tools".to_owned(),
                    "-y".to_owned(),
                ],
                BTreeMap::new(),
                Duration::from_secs(15),
            )
            .await;
        }
    }

    let after_bytes = tokio::task::spawn_blocking(move || measure_cleanable_bytes(&mount_dir))
        .await
        .map_err(|e| DaemonError::join("tools.cacheClean after join", e))??;

    tools.emit_state_changed(container.clone(), "cache_cleaned", None);
    Ok(ToolsCacheCleanResult {
        container,
        before_bytes,
        after_bytes,
        freed_bytes: before_bytes.saturating_sub(after_bytes),
    })
}

fn collect_host_diagnostics(
    mount_dir: &Path,
    effective_init: &[String],
) -> Vec<ToolDiagnosticItem> {
    let mut items = Vec::new();
    let dropbear_env = effective_init
        .first()
        .is_some_and(|p| p.contains("dropbear"))
        && effective_init.iter().any(|arg| arg == "-e");
    items.push(ToolDiagnosticItem {
        id: "dropbear-env-pass".to_owned(),
        category: "ssh".to_owned(),
        status: if dropbear_env {
            ToolDiagnosticStatus::Ok
        } else {
            ToolDiagnosticStatus::Warn
        },
        summary: if dropbear_env {
            "Dropbear init command preserves container environment (-e)"
        } else {
            "Dropbear init command does not include -e environment passthrough"
        }
        .to_owned(),
        remediation: (!dropbear_env)
            .then(|| "Use Strata default init command with Dropbear -e enabled".to_owned()),
    });

    if !mount::is_ext4_mounted(mount_dir).unwrap_or(false) {
        items.push(ToolDiagnosticItem {
            id: "container-mounted".to_owned(),
            category: "runtime".to_owned(),
            status: ToolDiagnosticStatus::Error,
            summary: "Container rootfs is not mounted".to_owned(),
            remediation: Some("Start the container before running tools diagnostics".to_owned()),
        });
        return items;
    }

    let Ok(root) = RootDir::open(mount_dir) else {
        items.push(ToolDiagnosticItem {
            id: "rootfs-open".to_owned(),
            category: "layout".to_owned(),
            status: ToolDiagnosticStatus::Error,
            summary: "Failed to open container mount directory safely".to_owned(),
            remediation: None,
        });
        return items;
    };

    let layout_ver = read_root_utf8(&root, CONTAINER_LAYOUT_PATH.trim_start_matches('/'))
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("schemaVersion")?.as_u64())
        .and_then(|n| u32::try_from(n).ok());
    let has_env = root
        .open_file_ro_nofollow(Path::new(
            CONTAINER_PROFILE_ENV_PATH.trim_start_matches('/'),
        ))
        .is_ok()
        && root
            .open_file_ro_nofollow(Path::new(
                CONTAINER_ETC_ENVIRONMENT_PATH.trim_start_matches('/'),
            ))
            .is_ok()
        && root
            .open_file_ro_nofollow(Path::new(
                CONTAINER_MISE_CONFIG_FILE.trim_start_matches('/'),
            ))
            .is_ok()
        && root.is_dir(Path::new("workspace"));
    items.push(ToolDiagnosticItem {
        id: "canonical-layout".to_owned(),
        category: "layout".to_owned(),
        status: if has_env && layout_ver == Some(CONTAINER_LAYOUT_SCHEMA_VERSION) {
            ToolDiagnosticStatus::Ok
        } else {
            ToolDiagnosticStatus::Error
        },
        summary: if has_env && layout_ver == Some(CONTAINER_LAYOUT_SCHEMA_VERSION) {
            format!("Canonical layout schema v{CONTAINER_LAYOUT_SCHEMA_VERSION} verified")
        } else {
            "Canonical layout files or /workspace are incomplete".to_owned()
        },
        remediation: (!has_env)
            .then(|| "Run `strata dns` or restart container to re-sync layout".to_owned()),
    });

    let apt_sandbox = read_root_utf8(&root, "etc/apt/apt.conf.d/99strata-sandbox")
        .is_some_and(|c| c.contains("Sandbox::User \"root\""));
    items.push(ToolDiagnosticItem {
        id: "apt-sandbox-root".to_owned(),
        category: "apt".to_owned(),
        status: if apt_sandbox {
            ToolDiagnosticStatus::Ok
        } else {
            ToolDiagnosticStatus::Error
        },
        summary: if apt_sandbox {
            "APT sandbox configured for Android GKI root user".to_owned()
        } else {
            "Missing /etc/apt/apt.conf.d/99strata-sandbox (APT fails on paranoid_network)"
                .to_owned()
        },
        remediation: (!apt_sandbox)
            .then(|| "Restart container to re-sync 99strata-sandbox".to_owned()),
    });

    items
}

pub(super) fn parse_mise_doctor_json(raw: &str) -> Option<ToolDiagnosticItem> {
    let val: Value = serde_json::from_str(raw).ok()?;
    let errors = val
        .get("errors")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let warnings = val
        .get("warnings")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let status = if errors > 0 {
        ToolDiagnosticStatus::Error
    } else if warnings > 0 {
        ToolDiagnosticStatus::Warn
    } else {
        ToolDiagnosticStatus::Ok
    };
    let version = val
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    Some(ToolDiagnosticItem {
        id: "mise-doctor".to_owned(),
        category: "mise".to_owned(),
        status,
        summary: format!("mise {version}: {errors} error(s), {warnings} warning(s)"),
        remediation: (errors > 0)
            .then(|| "Check miseDoctorJson or run `mise doctor` inside container".to_owned()),
    })
}

fn measure_cleanable_bytes(mount_dir: &Path) -> DaemonResult<u64> {
    if !mount::is_ext4_mounted(mount_dir).unwrap_or(false) {
        return Err(DaemonError::ContainerNotRunning(
            "container must be running to clean tool caches".to_owned(),
        ));
    }
    Ok(
        dir_size_bytes(&mount_dir.join(CONTAINER_MISE_CACHE_DIR.trim_start_matches('/')))
            + dir_size_bytes(&mount_dir.join(CONTAINER_UV_CACHE_DIR.trim_start_matches('/')))
            + dir_size_bytes(&mount_dir.join("var/cache/apt/archives")),
    )
}

fn clear_dir_contents(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let p = entry.path();
        if let Ok(meta) = std::fs::symlink_metadata(&p) {
            if meta.is_dir() && !meta.file_type().is_symlink() {
                let _ = std::fs::remove_dir_all(&p);
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mise_doctor_json_reads_errors_even_on_nonzero_exit() {
        let raw = r#"{"version":"2025.4.0","errors":["shim missing"],"warnings":["age"]}"#;
        let item = parse_mise_doctor_json(raw).expect("parse doctor");
        assert_eq!(item.status, ToolDiagnosticStatus::Error);
        assert!(item.summary.contains("1 error(s), 1 warning(s)"));
    }
}
