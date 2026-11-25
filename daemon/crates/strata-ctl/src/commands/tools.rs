//! `strata tools` subcommand dispatcher.

use strata_protocol::rpc::{ToolsConfigGetParams, ToolsDoctorParams, ToolsStatusParams, methods};

use crate::client::StrataClient;
use crate::commands::tools_args::{
    parse_bootstrap_args, parse_cache_clean_args, parse_catalog_args, parse_config_set_args,
    parse_install_args, parse_list_args, parse_uninstall_args, parse_upgrade_args,
};
use crate::commands::{emit_typed, rpc_or_err};
use crate::format;

pub(crate) fn handle_tools(
    client: &mut StrataClient,
    container: Option<String>,
    rest: &[String],
    json_mode: bool,
) -> Result<i32, String> {
    let sub = rest.first().map_or("status", String::as_str);
    let args = rest.get(1..).unwrap_or(&[]);

    match sub {
        "status" => {
            let params = serde_json::to_value(ToolsStatusParams { container }).ok();
            let val = rpc_or_err(client.call(methods::TOOLS_STATUS, params))?;
            emit_typed(&val, json_mode, format::print_tools_status)?;
            Ok(0)
        }
        "list" => {
            let p = parse_list_args(container, args)?;
            let params = serde_json::to_value(p).ok();
            let val = rpc_or_err(client.call(methods::TOOLS_LIST, params))?;
            emit_typed(&val, json_mode, format::print_tools_list)?;
            Ok(0)
        }
        "catalog" => {
            let p = parse_catalog_args(container, args)?;
            let params = serde_json::to_value(p).ok();
            let val = rpc_or_err(client.call(methods::TOOLS_CATALOG, params))?;
            emit_typed(&val, json_mode, format::print_tools_catalog)?;
            Ok(0)
        }
        "config" => handle_config_subcommand(client, container, args, json_mode),
        "bootstrap" => {
            let p = parse_bootstrap_args(container, args)?;
            let val = serde_json::to_value(p).map_err(|e| e.to_string())?;
            client.run_streaming_rpc_task(methods::TOOLS_BOOTSTRAP, Some(val), json_mode)
        }
        "install" => {
            let p = parse_install_args(container, args)?;
            let val = serde_json::to_value(p).map_err(|e| e.to_string())?;
            client.run_streaming_rpc_task(methods::TOOLS_INSTALL, Some(val), json_mode)
        }
        "uninstall" | "rm" => {
            let p = parse_uninstall_args(container, args)?;
            let val = serde_json::to_value(p).map_err(|e| e.to_string())?;
            client.run_streaming_rpc_task(methods::TOOLS_UNINSTALL, Some(val), json_mode)
        }
        "upgrade" => {
            let p = parse_upgrade_args(container, args)?;
            let val = serde_json::to_value(p).map_err(|e| e.to_string())?;
            client.run_streaming_rpc_task(methods::TOOLS_UPGRADE, Some(val), json_mode)
        }
        "doctor" => {
            let params = serde_json::to_value(ToolsDoctorParams { container }).ok();
            let val = rpc_or_err(client.call(methods::TOOLS_DOCTOR, params))?;
            emit_typed(&val, json_mode, format::print_tools_doctor)?;
            Ok(0)
        }
        "cache-clean" => {
            let p = parse_cache_clean_args(container, args)?;
            let params = serde_json::to_value(p).ok();
            let val = rpc_or_err(client.call(methods::TOOLS_CACHE_CLEAN, params))?;
            emit_typed(&val, json_mode, format::print_tools_cache_clean)?;
            Ok(0)
        }
        other => Err(format!(
            "unknown tools action '{other}' (expected status|list|catalog|config|bootstrap|install|uninstall|upgrade|doctor|cache-clean)"
        )),
    }
}

fn handle_config_subcommand(
    client: &mut StrataClient,
    container: Option<String>,
    args: &[String],
    json_mode: bool,
) -> Result<i32, String> {
    let action = args.first().map_or("get", String::as_str);
    if action == "get" || action == "show" {
        let params = serde_json::to_value(ToolsConfigGetParams { container }).ok();
        let val = rpc_or_err(client.call(methods::TOOLS_CONFIG_GET, params))?;
        emit_typed(&val, json_mode, format::print_tools_config)?;
        return Ok(0);
    }
    if action == "set" {
        let p = parse_config_set_args(container, args.get(1..).unwrap_or(&[]))?;
        let params = serde_json::to_value(p).ok();
        let val = rpc_or_err(client.call(methods::TOOLS_CONFIG_SET, params))?;
        emit_typed(&val, json_mode, format::print_tools_config)?;
        return Ok(0);
    }
    Err(format!(
        "unknown tools config action '{action}' (expected get|set)"
    ))
}
