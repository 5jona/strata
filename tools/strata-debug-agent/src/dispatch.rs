//! Top-level subcommand dispatcher shared by single-shot CLI and persistent `serve-rpc`.

mod subcmds;

use serde_json::{Value, json};

use crate::cli::Command;
use crate::{health, host, logcat, observe, perfetto, visual};
use subcmds::{dispatch_app, dispatch_cdp, dispatch_gesture, dispatch_system, dispatch_ui};

/// Dispatches any CLI subcommand and returns its structured JSON payload.
pub(crate) async fn dispatch(cmd: Command) -> Value {
    match cmd {
        Command::Health {
            package,
            no_sf,
            no_strata,
            include_sideveil,
            no_sideveil,
        } => {
            let inc_sv = if no_sideveil {
                Some(false)
            } else if include_sideveil {
                Some(true)
            } else {
                None
            };
            health::collect_health(&package, !no_sf, !no_strata, inc_sv).await
        }
        Command::Visual {
            roi,
            out,
            format,
            quality,
            max_width,
            golden,
            burst_sheet,
            burst_interval_ms,
            inline_base64,
        } => {
            visual::run_visual(&visual::VisualOpts {
                roi,
                out_path: out,
                format,
                quality,
                max_width,
                golden_raw: golden,
                burst_sheet,
                burst_interval_ms,
                inline_base64,
            })
            .await
        }
        Command::Ui { sub } => dispatch_ui(sub).await,
        Command::Gesture { sub } => dispatch_gesture(sub).await,
        Command::Logcat {
            mark,
            cursor,
            priority,
            tag,
            pid,
            limit,
            no_advance,
        } => {
            if mark {
                logcat::mark_cursor(&cursor).await
            } else {
                logcat::read_incremental(
                    &cursor,
                    &priority,
                    tag.as_deref(),
                    pid,
                    limit,
                    !no_advance,
                )
                .await
            }
        }
        Command::Cdp { sub } => dispatch_cdp(sub).await,
        Command::Observe {
            action_rpc,
            action_shell,
            action_cdp_eval,
            action_gesture,
            cursor,
            settle_ms,
            package,
            priority,
            tag,
            no_cdp,
            cdp_pid,
            visual,
            roi,
            visual_out,
            inline_image,
            max_width,
        } => {
            let rpc = action_rpc.and_then(|v| (v.len() == 2).then(|| (v[0].clone(), v[1].clone())));
            let gest = match action_gesture.as_deref() {
                Some(raw) => match serde_json::from_str::<Value>(raw) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        return json!({"ok": false, "error": format!("invalid action-gesture JSON: {e}")});
                    }
                },
                None => None,
            };
            observe::run_observe(&observe::ObserveOpts {
                action_rpc: rpc,
                action_shell,
                action_cdp_eval,
                action_gesture: gest,
                cursor,
                settle_ms,
                package,
                priority,
                tag,
                include_cdp: !no_cdp,
                cdp_pid,
                include_visual: visual,
                roi,
                visual_out,
                inline_image,
                max_width,
            })
            .await
        }
        Command::App { sub } => dispatch_app(sub).await,
        Command::System { sub } => dispatch_system(sub).await,
        Command::Perfetto {
            action,
            preset,
            duration,
            buffer_kb,
            max_duration,
            session,
            out,
            path,
        } => match action.as_str() {
            "start" => {
                perfetto::start_ring(
                    &preset,
                    buffer_kb.unwrap_or(16_384),
                    max_duration,
                    &session,
                    out.as_deref(),
                )
                .await
            }
            "status" => perfetto::ring_status(&session).await,
            "snapshot" => perfetto::stop_or_snapshot(&session, true).await,
            "stop" => perfetto::stop_or_snapshot(&session, false).await,
            "info" | "analyze" => match path.or(out) {
                Some(p) => perfetto::info_trace(&p).await,
                None => {
                    json!({"ok": false, "error": "--path is required for perfetto info/analyze"})
                }
            },
            _ => {
                perfetto::capture(&preset, duration, buffer_kb.unwrap_or(8192), out.as_deref())
                    .await
            }
        },
        Command::Exec {
            scope,
            timeout,
            command,
        } => host::exec_shell(&scope, &command, timeout).await,
        Command::ServeRpc => json!({"ok": false, "error": "nested serve-rpc not allowed"}),
    }
}
