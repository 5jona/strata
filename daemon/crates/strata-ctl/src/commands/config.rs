//! Container configuration CLI subcommands (`strata config ...`).

use strata_protocol::config::{BindMount, l5_diag_mounts};
use strata_protocol::constants::L5_DIAG_MOUNT_PATHS;
use strata_protocol::rpc::{
    ContainerConfigPatchParams, ContainerConfigResult, ContainerSelectorParams, methods,
};

use crate::client::StrataClient;
use crate::commands::{print_json, rpc_or_err};

pub(crate) fn handle_config(
    client: &mut StrataClient,
    container: Option<String>,
    rest: &[String],
    json_mode: bool,
) -> Result<i32, String> {
    let action = rest.first().map_or("show", String::as_str);
    match action {
        "show" => {
            let params = serde_json::to_value(ContainerSelectorParams { container }).ok();
            let val = rpc_or_err(client.call(methods::CONTAINER_CONFIG_GET, params))?;
            if json_mode {
                print_json(&val);
            } else {
                let res: ContainerConfigResult =
                    serde_json::from_value(val).map_err(|e| e.to_string())?;
                println!("{}", serde_json::to_string_pretty(&res).unwrap_or_default());
            }
            Ok(0)
        }
        "reload" => {
            let val = rpc_or_err(client.call(methods::DAEMON_RELOAD_CONFIG, None))?;
            if json_mode {
                print_json(&val);
            } else {
                println!("Reloaded configuration from disk.");
            }
            Ok(0)
        }
        "set-l5-diag" => {
            let Some(arg) = rest.get(1) else {
                return Err("usage: strata config set-l5-diag <on|off>".to_owned());
            };
            let enable = parse_on_off(arg)?;
            let get_params = serde_json::to_value(ContainerSelectorParams {
                container: container.clone(),
            })
            .ok();
            let current_val = rpc_or_err(client.call(methods::CONTAINER_CONFIG_GET, get_params))?;
            let current: ContainerConfigResult =
                serde_json::from_value(current_val).map_err(|e| e.to_string())?;
            let updated_mounts = merge_l5_diag_mounts(&current.config.extra_mounts, enable);
            let patch = ContainerConfigPatchParams {
                container,
                extra_mounts: Some(updated_mounts),
                ..ContainerConfigPatchParams::default()
            };
            let val = rpc_or_err(client.call(
                methods::CONTAINER_CONFIG_PATCH,
                serde_json::to_value(patch).ok(),
            ))?;
            if json_mode {
                print_json(&val);
            } else {
                println!("Updated container configuration (set-l5-diag={arg}).");
            }
            Ok(0)
        }
        "set-image" | "set-mount" | "set-hostname" | "set-port" | "set-host-bridge"
        | "set-host-runtime" | "set-char-devs" | "set-mounts" => {
            let Some(arg) = rest.get(1) else {
                return Err(format!("usage: strata config {action} <value>"));
            };
            let patch = build_config_patch(container, action, arg)?;
            let val = rpc_or_err(client.call(
                methods::CONTAINER_CONFIG_PATCH,
                serde_json::to_value(patch).ok(),
            ))?;
            if json_mode {
                print_json(&val);
            } else {
                println!("Updated container configuration ({action}={arg}).");
            }
            Ok(0)
        }
        other => Err(format!(
            "unknown config action '{other}' (expected show|reload|set-image|set-mount|\
             set-hostname|set-port|set-host-bridge|set-host-runtime|set-char-devs|\
             set-mounts|set-l5-diag)"
        )),
    }
}

pub(crate) fn build_config_patch(
    container: Option<String>,
    action: &str,
    arg: &str,
) -> Result<ContainerConfigPatchParams, String> {
    let mut patch = ContainerConfigPatchParams {
        container,
        ..ContainerConfigPatchParams::default()
    };
    match action {
        "set-image" => patch.image_path = Some(arg.to_owned()),
        "set-mount" => patch.mount_dir = Some(arg.to_owned()),
        "set-hostname" => patch.hostname = Some(arg.to_owned()),
        "set-port" => {
            let port = arg
                .parse::<u16>()
                .map_err(|_| format!("invalid port '{arg}'"))?;
            patch.ssh_port = Some(port);
        }
        "set-host-bridge" => patch.host_bridge = Some(parse_bool_arg(arg, action)?),
        "set-host-runtime" => patch.host_runtime = Some(parse_bool_arg(arg, action)?),
        "set-char-devs" => patch.extra_char_devs = Some(parse_char_devs(arg)?),
        "set-mounts" => patch.extra_mounts = Some(parse_mount_specs(arg)?),
        other => return Err(format!("unsupported config patch action '{other}'")),
    }
    Ok(patch)
}

fn parse_bool_arg(raw: &str, action: &str) -> Result<bool, String> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!(
            "invalid boolean '{raw}' for {action} (expected true|false)"
        )),
    }
}

pub(crate) fn parse_on_off(raw: &str) -> Result<bool, String> {
    match raw {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!(
            "invalid toggle '{raw}' for set-l5-diag (expected on|off)"
        )),
    }
}

pub(crate) fn parse_char_devs(raw: &str) -> Result<Vec<String>, String> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    if trimmed.is_empty() {
        return Err("expected comma-separated char devices or 'none'".to_owned());
    }
    let mut devs = Vec::new();
    for item in trimmed.split(',') {
        let dev = item.trim();
        if dev.is_empty() {
            return Err(format!("empty device entry in '{raw}'"));
        }
        if !devs.iter().any(|existing| existing == dev) {
            devs.push(dev.to_owned());
        }
    }
    Ok(devs)
}

pub(crate) fn parse_mount_specs(raw: &str) -> Result<Vec<BindMount>, String> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    if trimmed.is_empty() {
        return Err("expected comma-separated mount specs or 'none'".to_owned());
    }
    trimmed.split(',').map(parse_single_mount_spec).collect()
}

fn parse_single_mount_spec(spec: &str) -> Result<BindMount, String> {
    let parts: Vec<&str> = spec.trim().split(':').collect();
    let (host_path, container_path, read_only) = match parts.as_slice() {
        [src, dst] => (*src, *dst, false),
        [src, dst, "ro"] => (*src, *dst, true),
        [src, dst, "rw"] => (*src, *dst, false),
        _ => {
            return Err(format!(
                "invalid mount spec '{spec}' (expected src:dst, src:dst:ro, or src:dst:rw)"
            ));
        }
    };
    if host_path.is_empty() || container_path.is_empty() {
        return Err(format!(
            "invalid mount spec '{spec}': source and destination must be non-empty"
        ));
    }
    Ok(BindMount {
        host_path: host_path.to_owned(),
        container_path: container_path.to_owned(),
        read_only,
    })
}

pub(crate) fn merge_l5_diag_mounts(existing: &[BindMount], enable: bool) -> Vec<BindMount> {
    let mut out: Vec<BindMount> = existing
        .iter()
        .filter(|bm| !L5_DIAG_MOUNT_PATHS.contains(&bm.container_path.as_str()))
        .cloned()
        .collect();
    if enable {
        out.extend(l5_diag_mounts());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_config_patch_subcommands() {
        let p = build_config_patch(None, "set-host-bridge", "false").expect("host-bridge");
        assert_eq!(p.host_bridge, Some(false));
        assert!(build_config_patch(None, "set-host-bridge", "yes").is_err());

        let p = build_config_patch(Some("lab".to_owned()), "set-host-runtime", "true")
            .expect("host-runtime");
        assert_eq!(p.container.as_deref(), Some("lab"));
        assert_eq!(p.host_runtime, Some(true));

        let p =
            build_config_patch(None, "set-char-devs", "uinput,net/tun,uinput").expect("char-devs");
        assert_eq!(
            p.extra_char_devs,
            Some(vec!["uinput".to_owned(), "net/tun".to_owned()])
        );

        let p = build_config_patch(None, "set-char-devs", "none").expect("char-devs none");
        assert_eq!(p.extra_char_devs, Some(Vec::new()));
    }

    #[test]
    fn parses_mount_specs_and_rejects_malformed() {
        let mounts = parse_mount_specs("/data/a:/mnt/a,/data/b:/mnt/b:ro,/data/c:/mnt/c:rw")
            .expect("valid mounts");
        assert_eq!(mounts.len(), 3);
        assert!(!mounts[0].read_only);
        assert!(mounts[1].read_only);
        assert!(!mounts[2].read_only);
        assert_eq!(parse_mount_specs("none").expect("none"), Vec::new());

        assert!(parse_mount_specs("/only-one").is_err());
        assert!(parse_mount_specs("/a:/b:invalid").is_err());
        assert!(parse_mount_specs(":/b:ro").is_err());
    }

    #[test]
    fn merges_and_strips_l5_diag_mounts_without_clobbering_user_mounts() {
        assert!(parse_on_off("on").expect("on"));
        assert!(!parse_on_off("off").expect("off"));
        assert!(parse_on_off("true").is_err());

        let user_mount = BindMount {
            host_path: "/data/local/tmp/work".to_owned(),
            container_path: "/mnt/work".to_owned(),
            read_only: false,
        };
        let enabled = merge_l5_diag_mounts(std::slice::from_ref(&user_mount), true);
        assert_eq!(enabled.len(), 4);
        assert_eq!(enabled[0], user_mount);
        assert_eq!(enabled[1..], l5_diag_mounts()[..]);

        // Enabling twice stays idempotent.
        let enabled_again = merge_l5_diag_mounts(&enabled, true);
        assert_eq!(enabled_again, enabled);

        // Disabling removes only canonical L5 mounts and keeps user mounts intact.
        let disabled = merge_l5_diag_mounts(&enabled, false);
        assert_eq!(disabled, vec![user_mount]);
    }
}
