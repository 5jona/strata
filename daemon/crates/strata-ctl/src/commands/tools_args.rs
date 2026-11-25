//! CLI argument parsers for `strata tools` subcommands.

use strata_protocol::rpc::{
    ToolGroupKind, ToolManagerKind, ToolMirrorPreset, ToolsBootstrapParams, ToolsBootstrapProfile,
    ToolsCacheCleanParams, ToolsCatalogParams, ToolsConfigSetParams, ToolsInstallParams,
    ToolsListParams, ToolsUninstallParams, ToolsUpgradeParams,
};

pub(super) fn parse_list_args(
    container: Option<String>,
    args: &[String],
) -> Result<ToolsListParams, String> {
    let mut p = ToolsListParams {
        container,
        ..Default::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--group" => p.group = Some(parse_group(next_val(args, &mut i, "--group")?)?),
            "--outdated" => p.check_outdated = true,
            other => return Err(format!("unknown tools list flag '{other}'")),
        }
        i += 1;
    }
    Ok(p)
}

pub(super) fn parse_catalog_args(
    container: Option<String>,
    args: &[String],
) -> Result<ToolsCatalogParams, String> {
    let mut p = ToolsCatalogParams {
        container,
        ..Default::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--group" => p.group = Some(parse_group(next_val(args, &mut i, "--group")?)?),
            "--query" | "-q" => p.query = Some(next_val(args, &mut i, "--query")?.to_owned()),
            other => return Err(format!("unknown tools catalog flag '{other}'")),
        }
        i += 1;
    }
    Ok(p)
}

pub(super) fn parse_config_set_args(
    container: Option<String>,
    args: &[String],
) -> Result<ToolsConfigSetParams, String> {
    let mut p = ToolsConfigSetParams {
        container,
        ..Default::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--preset" => {
                p.mirror_preset = Some(parse_preset(next_val(args, &mut i, "--preset")?)?);
            }
            "--apt-mirror" => {
                p.apt_mirror_url = Some(next_val(args, &mut i, "--apt-mirror")?.to_owned());
            }
            "--node-mirror" => {
                p.node_mirror_url = Some(next_val(args, &mut i, "--node-mirror")?.to_owned());
            }
            "--pypi" => p.pypi_index_url = Some(next_val(args, &mut i, "--pypi")?.to_owned()),
            "--npm" => p.npm_registry_url = Some(next_val(args, &mut i, "--npm")?.to_owned()),
            "--rustup" => {
                p.rustup_dist_server = Some(next_val(args, &mut i, "--rustup")?.to_owned());
            }
            "--goproxy" => p.goproxy = Some(next_val(args, &mut i, "--goproxy")?.to_owned()),
            "--http-proxy" => {
                p.http_proxy = Some(next_val(args, &mut i, "--http-proxy")?.to_owned());
            }
            "--https-proxy" => {
                p.https_proxy = Some(next_val(args, &mut i, "--https-proxy")?.to_owned());
            }
            "--no-proxy" => p.no_proxy = Some(next_val(args, &mut i, "--no-proxy")?.to_owned()),
            "--min-age" => {
                p.minimum_release_age = Some(next_val(args, &mut i, "--min-age")?.to_owned());
            }
            "--github-token" => {
                p.github_token = Some(next_val(args, &mut i, "--github-token")?.to_owned());
            }
            "--jobs" => {
                let raw = next_val(args, &mut i, "--jobs")?;
                p.jobs = Some(
                    raw.parse::<u32>()
                        .map_err(|_| format!("invalid --jobs '{raw}'"))?,
                );
            }
            other => return Err(format!("unknown tools config set flag '{other}'")),
        }
        i += 1;
    }
    Ok(p)
}

pub(super) fn parse_bootstrap_args(
    container: Option<String>,
    args: &[String],
) -> Result<ToolsBootstrapParams, String> {
    let mut p = ToolsBootstrapParams {
        container,
        ..Default::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--profile" => {
                let raw = next_val(args, &mut i, "--profile")?;
                p.profile = Some(match raw {
                    "minimal" => ToolsBootstrapProfile::Minimal,
                    "dev" => ToolsBootstrapProfile::Dev,
                    "debug" => ToolsBootstrapProfile::Debug,
                    _ => return Err(format!("unknown --profile '{raw}' (minimal|dev|debug)")),
                });
            }
            "--preset" => {
                p.mirror_preset = Some(parse_preset(next_val(args, &mut i, "--preset")?)?);
            }
            other => return Err(format!("unknown tools bootstrap flag '{other}'")),
        }
        i += 1;
    }
    Ok(p)
}

pub(super) fn parse_install_args(
    container: Option<String>,
    args: &[String],
) -> Result<ToolsInstallParams, String> {
    let mut manager = ToolManagerKind::Mise;
    let mut pin = true;
    let mut packages = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manager" | "-m" => manager = parse_manager(next_val(args, &mut i, "--manager")?)?,
            "--no-pin" => pin = false,
            val if val.starts_with('-') => return Err(format!("unknown flag '{val}'")),
            pkg => packages.push(pkg.to_owned()),
        }
        i += 1;
    }
    if packages.is_empty() {
        return Err(
            "usage: strata tools install [--manager mise|apt|uv|bundle] <pkg...>".to_owned(),
        );
    }
    Ok(ToolsInstallParams {
        container,
        manager,
        packages,
        pin,
    })
}

pub(super) fn parse_uninstall_args(
    container: Option<String>,
    args: &[String],
) -> Result<ToolsUninstallParams, String> {
    let mut manager = ToolManagerKind::Mise;
    let mut packages = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manager" | "-m" => manager = parse_manager(next_val(args, &mut i, "--manager")?)?,
            val if val.starts_with('-') => return Err(format!("unknown flag '{val}'")),
            pkg => packages.push(pkg.to_owned()),
        }
        i += 1;
    }
    if packages.is_empty() {
        return Err("usage: strata tools uninstall [--manager mise|apt|uv] <pkg...>".to_owned());
    }
    Ok(ToolsUninstallParams {
        container,
        manager,
        packages,
    })
}

pub(super) fn parse_upgrade_args(
    container: Option<String>,
    args: &[String],
) -> Result<ToolsUpgradeParams, String> {
    let mut p = ToolsUpgradeParams {
        container,
        ..Default::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manager" | "-m" => {
                p.manager = Some(parse_manager(next_val(args, &mut i, "--manager")?)?);
            }
            "--bump" => p.bump = true,
            val if val.starts_with('-') => return Err(format!("unknown flag '{val}'")),
            pkg => p.packages.push(pkg.to_owned()),
        }
        i += 1;
    }
    Ok(p)
}

pub(super) fn parse_cache_clean_args(
    container: Option<String>,
    args: &[String],
) -> Result<ToolsCacheCleanParams, String> {
    let mut p = ToolsCacheCleanParams {
        container,
        clean_mise: true,
        clean_uv: true,
        clean_apt: true,
        prune_unused: false,
    };
    for arg in args {
        match arg.as_str() {
            "--no-mise" => p.clean_mise = false,
            "--no-uv" => p.clean_uv = false,
            "--no-apt" => p.clean_apt = false,
            "--prune" => p.prune_unused = true,
            other => return Err(format!("unknown tools cache-clean flag '{other}'")),
        }
    }
    Ok(p)
}

fn next_val<'a>(args: &'a [String], idx: &mut usize, flag: &str) -> Result<&'a str, String> {
    let val = args
        .get(*idx + 1)
        .ok_or_else(|| format!("missing value for {flag}"))?;
    *idx += 1;
    Ok(val.as_str())
}

fn parse_manager(raw: &str) -> Result<ToolManagerKind, String> {
    match raw {
        "mise" => Ok(ToolManagerKind::Mise),
        "apt" => Ok(ToolManagerKind::Apt),
        "uv" => Ok(ToolManagerKind::Uv),
        "bundle" => Ok(ToolManagerKind::Bundle),
        _ => Err(format!(
            "unknown --manager '{raw}' (expected mise|apt|uv|bundle)"
        )),
    }
}

fn parse_group(raw: &str) -> Result<ToolGroupKind, String> {
    match raw {
        "core-bridge" => Ok(ToolGroupKind::CoreBridge),
        "mise-runtimes" => Ok(ToolGroupKind::MiseRuntimes),
        "cli-dev" => Ok(ToolGroupKind::CliDev),
        "debug-l3-l5" => Ok(ToolGroupKind::DebugL3L5),
        "python-uv" => Ok(ToolGroupKind::PythonUv),
        _ => Err(format!("unknown --group '{raw}'")),
    }
}

fn parse_preset(raw: &str) -> Result<ToolMirrorPreset, String> {
    match raw {
        "tuna" => Ok(ToolMirrorPreset::Tuna),
        "official" => Ok(ToolMirrorPreset::Official),
        "custom" => Ok(ToolMirrorPreset::Custom),
        _ => Err(format!(
            "unknown --preset '{raw}' (expected tuna|official|custom)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_install_config_and_bootstrap_flags() {
        let inst = parse_install_args(
            Some("main".to_owned()),
            &[
                "--manager".to_owned(),
                "mise".to_owned(),
                "python@3.12".to_owned(),
                "ripgrep".to_owned(),
            ],
        )
        .expect("parse install");
        assert_eq!(inst.manager, ToolManagerKind::Mise);
        assert!(inst.pin);
        assert_eq!(inst.packages, vec!["python@3.12", "ripgrep"]);

        let cfg = parse_config_set_args(
            None,
            &[
                "--preset".to_owned(),
                "tuna".to_owned(),
                "--jobs".to_owned(),
                "8".to_owned(),
            ],
        )
        .expect("parse config");
        assert_eq!(cfg.mirror_preset, Some(ToolMirrorPreset::Tuna));
        assert_eq!(cfg.jobs, Some(8));

        let boot = parse_bootstrap_args(None, &["--profile".to_owned(), "debug".to_owned()])
            .expect("parse bootstrap");
        assert_eq!(boot.profile, Some(ToolsBootstrapProfile::Debug));
    }
}
