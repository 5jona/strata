//! Subcommand dispatch helpers for `ui`, `gesture`, `cdp`, `app`, and `system`.

use serde_json::{Value, json};

use crate::cli::{AppCmd, CdpCmd, GestureCmd, SystemCmd, UiCmd};
use crate::{app, cdp, gesture, system, ui};

pub(super) async fn dispatch_ui(sub: UiCmd) -> Value {
    match sub {
        UiCmd::Dump {
            compressed,
            query,
            resource_id,
            desc,
            clickable_only,
            compact,
            limit,
        } => {
            ui::dump_ui_tree(&ui::UiDumpOpts {
                compressed,
                query,
                resource_id,
                content_desc: desc,
                clickable_only,
                compact,
                limit,
            })
            .await
        }
        UiCmd::Tap {
            text,
            resource_id,
            desc,
            index,
            method,
        } => ui::tap_ui_node(text, resource_id, desc, index, &method).await,
    }
}

pub(super) async fn dispatch_gesture(sub: GestureCmd) -> Value {
    let kind = match sub {
        GestureCmd::Tap { x, y } => gesture::GestureKind::Tap { x, y },
        GestureCmd::LongPress { x, y, duration_ms } => {
            gesture::GestureKind::LongPress { x, y, duration_ms }
        }
        GestureCmd::Swipe {
            x0,
            y0,
            x1,
            y1,
            duration_ms,
        } => gesture::GestureKind::Swipe {
            x0,
            y0,
            x1,
            y1,
            duration_ms,
        },
        GestureCmd::EdgeSwipe {
            side,
            y_percent,
            distance_dp,
            duration_ms,
            cancel_back,
        } => gesture::GestureKind::EdgeSwipe {
            side,
            y_percent,
            distance_dp,
            duration_ms,
            cancel_back,
        },
    };
    gesture::run_gesture(kind).await
}

pub(super) async fn dispatch_cdp(sub: CdpCmd) -> Value {
    match sub {
        CdpCmd::ListTargets { target, pid } => cdp::list_targets(target.as_deref(), pid).await,
        CdpCmd::Eval {
            expression,
            target,
            pid,
            url_match,
            timeout,
        } => {
            cdp::eval_js(
                &expression,
                target.as_deref(),
                pid,
                url_match.as_deref(),
                timeout,
            )
            .await
        }
        CdpCmd::Audit {
            target,
            pid,
            url_match,
            eval_js,
            timeout,
        } => {
            cdp::audit_webview(
                target.as_deref(),
                pid,
                url_match.as_deref(),
                eval_js.as_deref(),
                timeout,
            )
            .await
        }
        CdpCmd::Console {
            target,
            pid,
            url_match,
            duration,
            timeout,
        } => {
            cdp::collect_console(
                target.as_deref(),
                pid,
                url_match.as_deref(),
                duration,
                timeout,
            )
            .await
        }
        CdpCmd::Forward { .. } => {
            json!({"ok": false, "error": "cdp forward must run as a standalone process"})
        }
    }
}

pub(super) async fn dispatch_app(sub: AppCmd) -> Value {
    match sub {
        AppCmd::Inspect { package, top_only } => app::inspect_app(&package, top_only).await,
        AppCmd::Control {
            action,
            package,
            activity,
            intent_action,
            data_uri,
        } => {
            app::control_app(
                &action,
                &package,
                activity.as_deref(),
                intent_action.as_deref(),
                data_uri.as_deref(),
            )
            .await
        }
        AppCmd::Install {
            apk_path,
            no_replace,
            no_grant,
            downgrade,
        } => app::install_apk(&apk_path, !no_replace, !no_grant, downgrade).await,
    }
}

pub(super) async fn dispatch_system(sub: SystemCmd) -> Value {
    match sub {
        SystemCmd::Pstore { dir } => system::collect_pstore(&dir).await,
        SystemCmd::Dmesg { limit, filter } => system::collect_dmesg(limit, filter.as_deref()).await,
        SystemCmd::Tracing { dir, limit } => system::inspect_tracing(&dir, limit),
        SystemCmd::Ksu { mode, path } => system::inspect_ksu(&mode, path.as_deref()).await,
        SystemCmd::Lsposed {
            mode,
            module,
            grep,
            limit,
        } => system::inspect_lsposed(&mode, module.as_deref(), grep.as_deref(), limit).await,
        SystemCmd::Sqlite {
            mode,
            db_path,
            sql,
            limit,
        } => system::sqlite_inspect(&db_path, &mode, sql.as_deref(), limit).await,
        SystemCmd::KernelProbe { preset, duration } => {
            system::run_kernel_probe(&preset, duration).await
        }
    }
}
