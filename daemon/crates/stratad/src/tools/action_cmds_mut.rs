//! Injection-safe command builders for `tools.uninstall` and `tools.upgrade` + unit tests.

use strata_protocol::rpc::ToolManagerKind;
use strata_protocol::tools::{validate_package_list, validate_package_spec};

use crate::error::{DaemonError, DaemonResult};
use crate::tools::discovery::LiveToolBins;

fn mise_spec_has_version(spec: &str) -> bool {
    let after_backend = match spec.split_once(':') {
        Some((_, rest)) => rest,
        None => spec,
    };
    match after_backend.rfind('@') {
        Some(pos) => pos > 0 && pos > after_backend.rfind('/').unwrap_or(0),
        None => false,
    }
}

pub(super) fn build_uninstall_command(
    manager: ToolManagerKind,
    packages: &[String],
    bins: &LiveToolBins,
) -> DaemonResult<Vec<String>> {
    let mise_bin = bins.mise.as_deref().unwrap_or("mise");
    let uv_bin = bins.uv.as_deref().unwrap_or("uv");
    match manager {
        ToolManagerKind::Mise => {
            let specs = validate_package_list(manager, packages, false)
                .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
            let (pinned, unpinned): (Vec<String>, Vec<String>) =
                specs.into_iter().partition(|s| mise_spec_has_version(s));
            let script = if unpinned.is_empty() {
                "set -eu; mise_bin=$1; shift; for p in \"$@\"; do \"$mise_bin\" uninstall \"$p\"; done"
            } else {
                "set -eu; mise_bin=$1; pin_n=$2; shift 2; i=0; while [ \"$i\" -lt \"$pin_n\" ]; do \"$mise_bin\" uninstall \"$1\"; shift; i=$((i + 1)); done; for p in \"$@\"; do \"$mise_bin\" unuse -g \"$p\" >/dev/null 2>&1 || true; \"$mise_bin\" uninstall \"$p\" || true; done"
            };
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                script.to_owned(),
                "strata-mise-uninstall".to_owned(),
                mise_bin.to_owned(),
            ];
            if !unpinned.is_empty() {
                cmd.push(pinned.len().to_string());
            }
            cmd.extend(pinned);
            cmd.extend(unpinned);
            Ok(cmd)
        }
        ToolManagerKind::Uv => {
            let specs = validate_package_list(manager, packages, false)
                .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "set -eu; uv_bin=$1; shift; for p in \"$@\"; do case \"$p\" in python@*) \"$uv_bin\" python uninstall \"${p#python@}\" ;; *) \"$uv_bin\" tool uninstall \"$p\" ;; esac; done".to_owned(),
                "strata-uv-uninstall".to_owned(),
                uv_bin.to_owned(),
            ];
            cmd.extend(specs);
            Ok(cmd)
        }
        ToolManagerKind::Apt => sys_uninstall_cmd(
            manager,
            packages,
            "set -eu; export DEBIAN_FRONTEND=noninteractive; apt-get remove -y -- \"$@\"",
            "strata-apt-uninstall",
        ),
        ToolManagerKind::Apk => sys_uninstall_cmd(
            manager,
            packages,
            "set -eu; apk del -- \"$@\"",
            "strata-apk-uninstall",
        ),
        ToolManagerKind::Pacman => sys_uninstall_cmd(
            manager,
            packages,
            "set -eu; pacman -R --noconfirm -- \"$@\"",
            "strata-pacman-uninstall",
        ),
        ToolManagerKind::Dnf => sys_uninstall_cmd(
            manager,
            packages,
            "set -eu; dnf remove -y -- \"$@\"",
            "strata-dnf-uninstall",
        ),
        ToolManagerKind::Bundle | ToolManagerKind::System | ToolManagerKind::Manual => {
            Err(DaemonError::InvalidParams(
                "tools.uninstall supports mise, uv, apt, apk, pacman, or dnf managers".to_owned(),
            ))
        }
    }
}

fn sys_uninstall_cmd(
    manager: ToolManagerKind,
    packages: &[String],
    script: &str,
    argv0: &str,
) -> DaemonResult<Vec<String>> {
    let specs = validate_package_list(manager, packages, false)
        .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
    let mut cmd = vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        script.to_owned(),
        argv0.to_owned(),
    ];
    cmd.extend(specs);
    Ok(cmd)
}

pub(super) fn build_upgrade_command(
    manager: Option<ToolManagerKind>,
    packages: &[String],
    bump: bool,
    bins: &LiveToolBins,
) -> DaemonResult<Vec<String>> {
    let target = manager.unwrap_or(ToolManagerKind::Mise);
    let validated: Vec<String> = packages
        .iter()
        .map(|s| validate_package_spec(target, s, true))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
    let mise_bin = bins.mise.as_deref().unwrap_or("mise");
    let uv_bin = bins.uv.as_deref().unwrap_or("uv");

    match manager {
        None => {
            let bump_flag = if bump { "--bump" } else { "" };
            let script = format!(
                r#"set -eu
mise_bin=$1; uv_bin=$2
if command -v "$mise_bin" >/dev/null 2>&1 || [ -x "$mise_bin" ]; then "$mise_bin" upgrade --yes {bump_flag}; fi
if command -v "$uv_bin" >/dev/null 2>&1 || [ -x "$uv_bin" ]; then "$uv_bin" tool upgrade --all || true; fi
if command -v apt-get >/dev/null 2>&1; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update && apt-get upgrade -y
elif command -v apk >/dev/null 2>&1; then
  apk upgrade
elif command -v pacman >/dev/null 2>&1; then
  pacman -Syu --noconfirm
elif command -v dnf >/dev/null 2>&1; then
  dnf upgrade -y
fi"#
            );
            Ok(vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                script,
                "strata-upgrade-all".to_owned(),
                mise_bin.to_owned(),
                uv_bin.to_owned(),
            ])
        }
        Some(ToolManagerKind::Mise) => {
            let mut cmd = vec![
                mise_bin.to_owned(),
                "upgrade".to_owned(),
                "--yes".to_owned(),
            ];
            if bump {
                cmd.push("--bump".to_owned());
            }
            cmd.extend(validated);
            Ok(cmd)
        }
        Some(ToolManagerKind::Uv) if validated.is_empty() => Ok(vec![
            uv_bin.to_owned(),
            "tool".to_owned(),
            "upgrade".to_owned(),
            "--all".to_owned(),
        ]),
        Some(ToolManagerKind::Uv) => {
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "set -eu; uv_bin=$1; shift; for p in \"$@\"; do \"$uv_bin\" tool upgrade \"$p\"; done".to_owned(),
                "strata-uv-upgrade".to_owned(),
                uv_bin.to_owned(),
            ];
            cmd.extend(validated);
            Ok(cmd)
        }
        Some(ToolManagerKind::Apt) if validated.is_empty() => Ok(vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            "set -eu; export DEBIAN_FRONTEND=noninteractive; apt-get update && apt-get upgrade -y"
                .to_owned(),
        ]),
        Some(ToolManagerKind::Apt) => {
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "set -eu; export DEBIAN_FRONTEND=noninteractive; apt-get update && apt-get install -y --only-upgrade -- \"$@\"".to_owned(),
                "strata-apt-upgrade".to_owned(),
            ];
            cmd.extend(validated);
            Ok(cmd)
        }
        Some(ToolManagerKind::Apk) if validated.is_empty() => Ok(vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            "set -eu; apk upgrade".to_owned(),
        ]),
        Some(ToolManagerKind::Apk) => {
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "set -eu; apk upgrade -- \"$@\"".to_owned(),
                "strata-apk-upgrade".to_owned(),
            ];
            cmd.extend(validated);
            Ok(cmd)
        }
        Some(ToolManagerKind::Pacman) if validated.is_empty() => Ok(vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            "set -eu; pacman -Syu --noconfirm".to_owned(),
        ]),
        Some(ToolManagerKind::Pacman) => {
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "set -eu; pacman -S --noconfirm -- \"$@\"".to_owned(),
                "strata-pacman-upgrade".to_owned(),
            ];
            cmd.extend(validated);
            Ok(cmd)
        }
        Some(ToolManagerKind::Dnf) if validated.is_empty() => Ok(vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            "set -eu; dnf upgrade -y".to_owned(),
        ]),
        Some(ToolManagerKind::Dnf) => {
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "set -eu; dnf upgrade -y -- \"$@\"".to_owned(),
                "strata-dnf-upgrade".to_owned(),
            ];
            cmd.extend(validated);
            Ok(cmd)
        }
        Some(ToolManagerKind::Bundle | ToolManagerKind::System | ToolManagerKind::Manual) => {
            Err(DaemonError::InvalidParams(
                "tools.upgrade supports mise, uv, apt, apk, pacman, dnf, or omitted manager"
                    .to_owned(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::action_cmds::build_install_command;

    #[test]
    fn install_and_uninstall_block_dangerous_packages_and_use_discovered_bins() {
        let bins = LiveToolBins {
            mise: Some("/root/.local/bin/mise".to_owned()),
            uv: Some("/root/.local/bin/uv".to_owned()),
        };
        assert!(
            build_install_command(
                ToolManagerKind::Apt,
                &["openssh-server".to_owned()],
                true,
                &bins
            )
            .is_err()
        );
        assert!(
            build_install_command(
                ToolManagerKind::Mise,
                &["node;rm -rf /".to_owned()],
                true,
                &bins
            )
            .is_err()
        );
        assert!(
            build_uninstall_command(ToolManagerKind::Apt, &["dropbear-bin".to_owned()], &bins)
                .is_err()
        );
        let cmd = build_install_command(
            ToolManagerKind::Mise,
            &["python@3.12".to_owned()],
            true,
            &bins,
        )
        .expect("valid mise install");
        assert_eq!(
            cmd,
            vec!["/root/.local/bin/mise", "use", "-g", "--yes", "python@3.12"]
        );

        let pinned_uninst =
            build_uninstall_command(ToolManagerKind::Mise, &["python@3.11.9".to_owned()], &bins)
                .expect("pinned uninstall");
        assert!(!pinned_uninst[2].contains("unuse"));

        let scoped_unpinned = build_uninstall_command(
            ToolManagerKind::Mise,
            &["npm:@biomejs/biome".to_owned()],
            &bins,
        )
        .expect("unpinned scoped npm uninstall");
        assert!(scoped_unpinned[2].contains("unuse"));

        let scoped_pinned = build_uninstall_command(
            ToolManagerKind::Mise,
            &["npm:@biomejs/biome@2.0".to_owned()],
            &bins,
        )
        .expect("pinned scoped npm uninstall");
        assert!(!scoped_pinned[2].contains("unuse"));

        let apk_targeted = build_upgrade_command(
            Some(ToolManagerKind::Apk),
            &["ripgrep".to_owned()],
            false,
            &bins,
        )
        .expect("targeted apk upgrade");
        assert_eq!(apk_targeted.last().map(String::as_str), Some("ripgrep"));
        assert!(apk_targeted[2].contains("apk upgrade --"));
    }
}
