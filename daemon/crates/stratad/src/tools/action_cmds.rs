//! Positional, injection-safe command builders for `bootstrap`, `install`, `uninstall`, and `upgrade`.

use strata_protocol::constants::CONTAINER_OPT_BIN_DIR;
use strata_protocol::rpc::ToolManagerKind;
use strata_protocol::tools::{validate_package_list, validate_package_spec};

use crate::error::{DaemonError, DaemonResult};
use crate::tools::catalog::expand_bundle_specs;

pub(super) fn build_bootstrap_command(apt_pkgs: &[&str], mise_pkgs: &[&str]) -> Vec<String> {
    let bin = CONTAINER_OPT_BIN_DIR;
    // Static script reads counts and installs via positional arguments — zero user input interpolated.
    let script = format!(
        "set -eu
export DEBIAN_FRONTEND=noninteractive
apt_n=$1; mise_n=$2; shift 2
if [ \"$apt_n\" -gt 0 ]; then
  apt-get update
  i=0
  while [ \"$i\" -lt \"$apt_n\" ]; do
    apt-get install -y --no-install-recommends -- \"$1\"
    shift
    i=$((i + 1))
  done
fi
if [ ! -x \"{bin}/mise\" ] || ! \"{bin}/mise\" --version >/dev/null 2>&1; then
  export MISE_INSTALL_PATH=\"{bin}/mise\"
  curl -fsSL https://mise.run | sh
fi
\"{bin}/mise\" use -g --yes uv@latest
uv_bin=$(\"{bin}/mise\" which uv 2>/dev/null || true)
if [ -n \"$uv_bin\" ] && [ -x \"$uv_bin\" ] && [ ! -e \"{bin}/uv\" ]; then
  ln -sf \"$uv_bin\" \"{bin}/uv\"
fi
if [ \"$mise_n\" -gt 0 ]; then
  \"{bin}/mise\" use -g --yes \"$@\"
fi"
    );

    let mut cmd = vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        script,
        "strata-bootstrap".to_owned(),
        apt_pkgs.len().to_string(),
        mise_pkgs.len().to_string(),
    ];
    cmd.extend(apt_pkgs.iter().map(|s| (*s).to_owned()));
    cmd.extend(mise_pkgs.iter().map(|s| (*s).to_owned()));
    cmd
}

pub(super) fn build_install_command(
    manager: ToolManagerKind,
    packages: &[String],
    pin: bool,
) -> DaemonResult<Vec<String>> {
    match manager {
        ToolManagerKind::Bundle => build_bundle_install_command(packages),
        ToolManagerKind::Mise => {
            let specs = validate_package_list(manager, packages, true)
                .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
            let mut cmd = vec![format!("{CONTAINER_OPT_BIN_DIR}/mise")];
            if pin {
                cmd.push("use".to_owned());
                cmd.push("-g".to_owned());
            } else {
                cmd.push("install".to_owned());
            }
            cmd.push("--yes".to_owned());
            cmd.extend(specs);
            Ok(cmd)
        }
        ToolManagerKind::Uv => {
            let specs = validate_package_list(manager, packages, true)
                .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                format!(
                    "set -eu; for p in \"$@\"; do \"{CONTAINER_OPT_BIN_DIR}/uv\" tool install \"$p\"; done"
                ),
                "strata-uv-install".to_owned(),
            ];
            cmd.extend(specs);
            Ok(cmd)
        }
        ToolManagerKind::Apt => {
            let specs = validate_package_list(manager, packages, true)
                .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "set -eu; export DEBIAN_FRONTEND=noninteractive; apt-get update && apt-get install -y --no-install-recommends -- \"$@\"".to_owned(),
                "strata-apt-install".to_owned(),
            ];
            cmd.extend(specs);
            Ok(cmd)
        }
        ToolManagerKind::System => Err(DaemonError::InvalidParams(
            "system bridge binaries cannot be installed via tools.install".to_owned(),
        )),
    }
}

fn build_bundle_install_command(packages: &[String]) -> DaemonResult<Vec<String>> {
    if packages.is_empty() {
        return Err(DaemonError::InvalidParams(
            "bundle list must not be empty".to_owned(),
        ));
    }
    let expanded = expand_bundle_specs(packages)?;
    let mut apt = Vec::new();
    let mut mise = Vec::new();
    let mut uv = Vec::new();
    for (mgr, spec) in expanded {
        match mgr {
            ToolManagerKind::Apt => apt.push(spec),
            ToolManagerKind::Mise => mise.push(spec),
            ToolManagerKind::Uv => uv.push(spec),
            ToolManagerKind::Bundle | ToolManagerKind::System => {}
        }
    }
    let bin = CONTAINER_OPT_BIN_DIR;
    let script = format!(
        "set -eu
export DEBIAN_FRONTEND=noninteractive
apt_n=$1; mise_n=$2; uv_n=$3; shift 3
if [ \"$apt_n\" -gt 0 ]; then
  apt-get update
  i=0
  while [ \"$i\" -lt \"$apt_n\" ]; do
    apt-get install -y --no-install-recommends -- \"$1\"
    shift; i=$((i + 1))
  done
fi
if [ \"$mise_n\" -gt 0 ]; then
  i=0
  while [ \"$i\" -lt \"$mise_n\" ]; do
    \"{bin}/mise\" use -g --yes \"$1\"
    shift; i=$((i + 1))
  done
fi
if [ \"$uv_n\" -gt 0 ]; then
  for p in \"$@\"; do
    \"{bin}/uv\" tool install \"$p\"
  done
fi"
    );
    let mut cmd = vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        script,
        "strata-bundle-install".to_owned(),
        apt.len().to_string(),
        mise.len().to_string(),
        uv.len().to_string(),
    ];
    cmd.extend(apt);
    cmd.extend(mise);
    cmd.extend(uv);
    Ok(cmd)
}

pub(super) fn build_uninstall_command(
    manager: ToolManagerKind,
    packages: &[String],
) -> DaemonResult<Vec<String>> {
    match manager {
        ToolManagerKind::Mise => {
            let specs = validate_package_list(manager, packages, false)
                .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                format!(
                    "set -eu; for p in \"$@\"; do \"{CONTAINER_OPT_BIN_DIR}/mise\" unuse -g \"$p\" >/dev/null 2>&1 || true; \"{CONTAINER_OPT_BIN_DIR}/mise\" uninstall \"$p\" || true; done"
                ),
                "strata-mise-uninstall".to_owned(),
            ];
            cmd.extend(specs);
            Ok(cmd)
        }
        ToolManagerKind::Uv => {
            let specs = validate_package_list(manager, packages, false)
                .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                format!(
                    "set -eu; for p in \"$@\"; do \"{CONTAINER_OPT_BIN_DIR}/uv\" tool uninstall \"$p\"; done"
                ),
                "strata-uv-uninstall".to_owned(),
            ];
            cmd.extend(specs);
            Ok(cmd)
        }
        ToolManagerKind::Apt => {
            let specs = validate_package_list(manager, packages, false)
                .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "set -eu; export DEBIAN_FRONTEND=noninteractive; apt-get remove -y -- \"$@\""
                    .to_owned(),
                "strata-apt-uninstall".to_owned(),
            ];
            cmd.extend(specs);
            Ok(cmd)
        }
        ToolManagerKind::Bundle | ToolManagerKind::System => Err(DaemonError::InvalidParams(
            "tools.uninstall supports mise, uv, or apt managers".to_owned(),
        )),
    }
}

pub(super) fn build_upgrade_command(
    manager: Option<ToolManagerKind>,
    packages: &[String],
    bump: bool,
) -> DaemonResult<Vec<String>> {
    let target = manager.unwrap_or(ToolManagerKind::Mise);
    let validated: Vec<String> = packages
        .iter()
        .map(|s| validate_package_spec(target, s, true))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;

    match manager {
        None => {
            let bin = CONTAINER_OPT_BIN_DIR;
            let bump_flag = if bump { "--bump" } else { "" };
            let script = format!(
                "set -eu
if [ -x \"{bin}/mise\" ]; then \"{bin}/mise\" upgrade --yes {bump_flag}; fi
if [ -x \"{bin}/uv\" ]; then \"{bin}/uv\" tool upgrade --all || true; fi
export DEBIAN_FRONTEND=noninteractive
apt-get update && apt-get upgrade -y"
            );
            Ok(vec!["/bin/sh".to_owned(), "-c".to_owned(), script])
        }
        Some(ToolManagerKind::Mise) => {
            let mut cmd = vec![
                format!("{CONTAINER_OPT_BIN_DIR}/mise"),
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
            format!("{CONTAINER_OPT_BIN_DIR}/uv"),
            "tool".to_owned(),
            "upgrade".to_owned(),
            "--all".to_owned(),
        ]),
        Some(ToolManagerKind::Uv) => {
            let mut cmd = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                format!(
                    "set -eu; for p in \"$@\"; do \"{CONTAINER_OPT_BIN_DIR}/uv\" tool upgrade \"$p\"; done"
                ),
                "strata-uv-upgrade".to_owned(),
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
        Some(ToolManagerKind::Bundle | ToolManagerKind::System) => Err(DaemonError::InvalidParams(
            "tools.upgrade supports mise, uv, apt, or omitted manager".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_and_uninstall_block_dangerous_packages_and_injections() {
        assert!(
            build_install_command(ToolManagerKind::Apt, &["openssh-server".to_owned()], true)
                .is_err()
        );
        assert!(
            build_install_command(ToolManagerKind::Mise, &["node;rm -rf /".to_owned()], true)
                .is_err()
        );
        assert!(
            build_uninstall_command(ToolManagerKind::Apt, &["dropbear-bin".to_owned()]).is_err()
        );
        let cmd = build_install_command(ToolManagerKind::Mise, &["python@3.12".to_owned()], true)
            .expect("valid mise install");
        assert_eq!(
            cmd,
            vec!["/opt/strata/bin/mise", "use", "-g", "--yes", "python@3.12"]
        );
    }
}
