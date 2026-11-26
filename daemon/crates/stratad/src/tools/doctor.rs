//! Layout, environment, and APT sandbox diagnostics (`tools.doctor`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use strata_protocol::constants::{
    CONTAINER_ETC_ENVIRONMENT_PATH, CONTAINER_LAYOUT_PATH, CONTAINER_LAYOUT_SCHEMA_VERSION,
    CONTAINER_PROFILE_ENV_PATH,
};
use strata_protocol::rpc::{
    ToolDiagnosticItem, ToolDiagnosticStatus, ToolsDoctorParams, ToolsDoctorResult,
};
use strata_sys::mount;
use strata_sys::rootfs_io::RootDir;

use crate::error::{DaemonError, DaemonResult};
use crate::tools::ToolsManager;
use crate::tools::discovery::resolve_live_tool_bins;
use crate::tools::exec::{
    load_container_tool_env_from_mount, read_root_utf8, run_container_command_bounded,
};

pub(super) async fn query_doctor(
    tools: &ToolsManager,
    params: ToolsDoctorParams,
) -> DaemonResult<ToolsDoctorResult> {
    let (container, entry, _) = tools
        .manager
        .resolve_container(params.container.as_deref())
        .await?;
    let mount_dir = PathBuf::from(&entry.mount_dir);
    let ssh_enabled = entry.ssh.enabled;
    let effective_init = entry.effective_init_command();

    let mut items = tokio::task::spawn_blocking(move || {
        collect_host_diagnostics(&mount_dir, ssh_enabled, &effective_init)
    })
    .await
    .map_err(|e| DaemonError::join("tools.doctor fs join", e))?;

    let mut mise_doctor_json = None;
    if let Ok((_, ns_pid, cfg)) = tools.manager.running_target(Some(&container)).await {
        let mdir = PathBuf::from(&cfg.mount_dir);
        let env = tokio::task::spawn_blocking(move || load_container_tool_env_from_mount(&mdir))
            .await
            .map_err(|e| DaemonError::join("tools.doctor env join", e))?;
        let bins = resolve_live_tool_bins(tools, &container, ns_pid, &cfg, &env).await;

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
            let mise_line = sh_out
                .stdout
                .lines()
                .find_map(|l| l.strip_prefix("MISE="))
                .unwrap_or("")
                .trim();
            let has_mise = !mise_line.is_empty();
            let has_shims = sh_out.stdout.lines().any(|l| {
                l.starts_with("PATH=")
                    && (l.contains("/root/.local/share/mise/shims")
                        || l.contains("/opt/strata/mise/shims"))
            });
            items.push(ToolDiagnosticItem {
                id: "login-shell-env".to_owned(),
                category: "environment".to_owned(),
                status: if has_mise && has_shims {
                    ToolDiagnosticStatus::Ok
                } else {
                    ToolDiagnosticStatus::Warn
                },
                summary: if has_mise && has_shims {
                    format!("Login shell exposes mise ({mise_line}) and shims on PATH")
                } else {
                    format!(
                        "Login shell PATH/mise incomplete: {}",
                        sh_out.stdout.replace('\n', "; ")
                    )
                },
                remediation: (!has_mise).then(|| {
                    "Run tools.bootstrap or `strata tools bootstrap` to install mise into ~/.local/bin"
                        .to_owned()
                }),
            });
        }

        if let Some(mise_bin) = bins.mise {
            if let Ok(doc_out) = run_container_command_bounded(
                &tools.self_exe,
                &container,
                ns_pid,
                &cfg,
                vec![mise_bin, "doctor".to_owned(), "--json".to_owned()],
                env,
                Duration::from_secs(6),
            )
            .await
            {
                let raw_json = doc_out.stdout.trim();
                if !raw_json.is_empty() {
                    if let Some(diag) = parse_mise_doctor_json(raw_json) {
                        items.push(diag);
                    }
                    mise_doctor_json = Some(raw_json.to_owned());
                }
            }
        } else {
            items.push(ToolDiagnosticItem {
                id: "mise-doctor".to_owned(),
                category: "mise".to_owned(),
                status: ToolDiagnosticStatus::Warn,
                summary: "mise binary was not found on standard paths".to_owned(),
                remediation: Some(
                    "Run tools.bootstrap or install mise into ~/.local/bin".to_owned(),
                ),
            });
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

fn collect_host_diagnostics(
    mount_dir: &Path,
    ssh_enabled: bool,
    effective_init: &[String],
) -> Vec<ToolDiagnosticItem> {
    let mut items = Vec::new();
    if ssh_enabled {
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
    }

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
        && root.is_dir(Path::new("workspace"));
    items.push(ToolDiagnosticItem {
        id: "canonical-layout".to_owned(),
        category: "layout".to_owned(),
        status: if has_env && layout_ver == Some(CONTAINER_LAYOUT_SCHEMA_VERSION) {
            ToolDiagnosticStatus::Ok
        } else {
            ToolDiagnosticStatus::Warn
        },
        summary: if has_env && layout_ver == Some(CONTAINER_LAYOUT_SCHEMA_VERSION) {
            format!("Standard XDG layout schema v{CONTAINER_LAYOUT_SCHEMA_VERSION} verified")
        } else {
            "Container layout metadata or /workspace needs refresh".to_owned()
        },
        remediation: (!(has_env && layout_ver == Some(CONTAINER_LAYOUT_SCHEMA_VERSION)))
            .then(|| "Run `strata dns` or restart container to refresh layout".to_owned()),
    });

    if root
        .open_file_ro_nofollow(Path::new("usr/bin/apt-get"))
        .is_ok()
    {
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
    }

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
