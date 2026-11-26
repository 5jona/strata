//! Positional, injection-safe command builders for `bootstrap`, `install`, `uninstall`, and `upgrade`.

use strata_protocol::rpc::ToolManagerKind;
use strata_protocol::tools::validate_package_list;

use crate::error::{DaemonError, DaemonResult};
pub(super) use crate::tools::action_cmds_mut::{build_uninstall_command, build_upgrade_command};
use crate::tools::catalog::expand_bundle_specs;
use crate::tools::discovery::LiveToolBins;
use crate::tools::shell_presets::post_install_shell_hook;

pub(super) fn build_bootstrap_command(apt_pkgs: &[&str], mise_pkgs: &[&str]) -> Vec<String> {
    // Static script reads counts and installs via positional arguments — zero user input interpolated.
    let script = r#"set -eu
export DEBIAN_FRONTEND=noninteractive
apt_n=$1; mise_n=$2; shift 2
if [ "$apt_n" -gt 0 ] && command -v apt-get >/dev/null 2>&1; then
  apt-get update
  i=0
  while [ "$i" -lt "$apt_n" ]; do
    apt-get install -y --no-install-recommends -- "$1"
    shift
    i=$((i + 1))
  done
else
  i=0
  while [ "$i" -lt "$apt_n" ]; do shift; i=$((i + 1)); done
fi
mise_bin="$(command -v mise 2>/dev/null || true)"
if [ -z "$mise_bin" ]; then
  mkdir -p /root/.local/bin
  export MISE_INSTALL_PATH="/root/.local/bin/mise"
  curl -fsSL https://mise.run | sh
  mise_bin="/root/.local/bin/mise"
fi
if ! command -v uv >/dev/null 2>&1; then
  "$mise_bin" use -g --yes uv@latest
fi
if [ "$mise_n" -gt 0 ]; then
  "$mise_bin" use -g --yes "$@"
fi"#;

    let mut cmd = vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        script.to_owned(),
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
    bins: &LiveToolBins,
) -> DaemonResult<Vec<String>> {
    let mise_bin = bins.mise.as_deref().unwrap_or("mise");
    let uv_bin = bins.uv.as_deref().unwrap_or("uv");
    match manager {
        ToolManagerKind::Bundle => build_bundle_install_command(packages, mise_bin, uv_bin),
        ToolManagerKind::Mise => {
            let specs = validate_package_list(manager, packages, true)
                .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;
            let mut cmd = vec![mise_bin.to_owned()];
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
                "set -eu; uv_bin=$1; shift; for p in \"$@\"; do \"$uv_bin\" tool install \"$p\"; done".to_owned(),
                "strata-uv-install".to_owned(),
                uv_bin.to_owned(),
            ];
            cmd.extend(specs);
            Ok(cmd)
        }
        ToolManagerKind::Apt => sys_install_cmd(
            manager,
            packages,
            "set -eu; export DEBIAN_FRONTEND=noninteractive; apt-get update && apt-get install -y --no-install-recommends -- \"$@\"",
            "strata-apt-install",
        ),
        ToolManagerKind::Apk => sys_install_cmd(
            manager,
            packages,
            "set -eu; apk add --no-cache -- \"$@\"",
            "strata-apk-install",
        ),
        ToolManagerKind::Pacman => sys_install_cmd(
            manager,
            packages,
            "set -eu; pacman -Sy --noconfirm --needed -- \"$@\"",
            "strata-pacman-install",
        ),
        ToolManagerKind::Dnf => sys_install_cmd(
            manager,
            packages,
            "set -eu; dnf install -y -- \"$@\"",
            "strata-dnf-install",
        ),
        ToolManagerKind::System | ToolManagerKind::Manual => Err(DaemonError::InvalidParams(
            "system/manual binaries cannot be installed via tools.install".to_owned(),
        )),
    }
}

fn sys_install_cmd(
    manager: ToolManagerKind,
    packages: &[String],
    script: &str,
    argv0: &str,
) -> DaemonResult<Vec<String>> {
    let specs = validate_package_list(manager, packages, true)
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

fn build_bundle_install_command(
    packages: &[String],
    mise_bin: &str,
    uv_bin: &str,
) -> DaemonResult<Vec<String>> {
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
            _ => {}
        }
    }

    let mut post_hooks = String::new();
    for id in packages {
        if let Some(hook) = post_install_shell_hook(id.trim()) {
            post_hooks.push('\n');
            post_hooks.push_str(hook);
        }
    }

    let script = format!(
        r#"set -eu
export DEBIAN_FRONTEND=noninteractive
mise_bin=$1; uv_bin=$2; apt_n=$3; mise_n=$4; uv_n=$5; shift 5
if [ "$apt_n" -gt 0 ]; then
  apt-get update
  i=0
  while [ "$i" -lt "$apt_n" ]; do
    apt-get install -y --no-install-recommends -- "$1"
    shift; i=$((i + 1))
  done
fi
if [ "$mise_n" -gt 0 ]; then
  i=0
  while [ "$i" -lt "$mise_n" ]; do
    "$mise_bin" use -g --yes "$1"
    shift; i=$((i + 1))
  done
fi
if [ "$uv_n" -gt 0 ]; then
  for p in "$@"; do
    "$uv_bin" tool install "$p"
  done
fi
{post_hooks}"#
    );
    let mut cmd = vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        script,
        "strata-bundle-install".to_owned(),
        mise_bin.to_owned(),
        uv_bin.to_owned(),
        apt.len().to_string(),
        mise.len().to_string(),
        uv.len().to_string(),
    ];
    cmd.extend(apt);
    cmd.extend(mise);
    cmd.extend(uv);
    Ok(cmd)
}
