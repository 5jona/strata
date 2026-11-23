"""Android UI, app lifecycle, LSPosed, SQLite, and KernelSU subcommands for strata-debug CLI."""

from __future__ import annotations

import argparse
import json
from typing import Callable

import app_ops
import host_inspect
import lsposed_ops
import perfetto_analyze
import perfetto_ops
import perfetto_session
import ui_tree


def _emit(payload: object) -> int:
    print(json.dumps(payload, ensure_ascii=False))
    return 0


def _cmd_ui(args: argparse.Namespace) -> int:
    if args.ui_cmd == "dump":
        return _emit(
            ui_tree.dump_ui_tree(
                compressed=args.compressed,
                query=args.query,
                resource_id=args.resource_id,
                content_desc=args.desc,
                clickable_only=args.clickable_only,
                limit=args.limit,
            )
        )
    if args.ui_cmd == "tap":
        return _emit(
            ui_tree.tap_ui_node(
                text=args.text,
                resource_id=args.resource_id,
                content_desc=args.desc,
                index=args.index,
                method=args.method,
            )
        )
    return 1


def _cmd_app(args: argparse.Namespace) -> int:
    if args.app_cmd == "top":
        return _emit(app_ops.get_top_app())
    if args.app_cmd == "inspect":
        return _emit(app_ops.inspect_app(args.package))
    if args.app_cmd == "start":
        return _emit(
            app_ops.start_app(
                args.package,
                activity=args.activity,
                action=args.action,
                data_uri=args.data,
            )
        )
    if args.app_cmd == "stop":
        return _emit(app_ops.stop_app(args.package))
    if args.app_cmd == "clear":
        return _emit(app_ops.clear_app(args.package))
    if args.app_cmd == "install":
        return _emit(
            app_ops.install_apk(
                args.apk_path,
                replace=not args.no_replace,
                grant=args.grant,
                downgrade=args.downgrade,
            )
        )
    return 1


def _cmd_lsposed(args: argparse.Namespace) -> int:
    if args.lsposed_cmd == "modules":
        return _emit(lsposed_ops.query_lsposed_modules(module_filter=args.module))
    if args.lsposed_cmd == "logs":
        return _emit(
            lsposed_ops.read_lsposed_logs(
                module_filter=args.module,
                grep_pattern=args.grep,
                limit=args.limit,
            )
        )
    return 1


def _cmd_db(args: argparse.Namespace) -> int:
    if args.db_cmd == "tables":
        return _emit(host_inspect.db_list_tables(args.db_path))
    if args.db_cmd == "query":
        try:
            return _emit(host_inspect.db_query(args.db_path, args.sql, limit=args.limit))
        except ValueError as exc:
            _emit({"ok": False, "error": str(exc)})
            return 1
    return 1


def _cmd_ksu(args: argparse.Namespace) -> int:
    if args.ksu_cmd == "modules":
        return _emit(host_inspect.inspect_ksu_modules())
    if args.ksu_cmd == "inputs":
        return _emit(host_inspect.inspect_input_devices())
    if args.ksu_cmd == "sysfs":
        return _emit(host_inspect.read_sysfs_node(args.path))
    return 1


def _cmd_perfetto(args: argparse.Namespace) -> int:
    action = args.action
    if action == "start":
        buf = args.buffer_kb if args.buffer_kb is not None else perfetto_session.DEFAULT_RING_BUFFER_KB
        res = perfetto_session.start_ring_session(
            preset=args.preset,
            buffer_kb=buf,
            max_duration_sec=args.max_duration,
            session_key=args.session,
            out_path=args.out,
        )
    elif action == "status":
        res = perfetto_session.ring_session_status(session_key=args.session)
    elif action == "snapshot":
        res = perfetto_session.snapshot_ring_session(
            session_key=args.session,
            keep_running=args.keep_running,
            analyze=args.analyze,
        )
    elif action == "stop":
        res = perfetto_session.stop_ring_session(session_key=args.session, analyze=args.analyze)
    elif action == "analyze":
        target = args.path or args.out
        if not target and args.session:
            status = perfetto_session.ring_session_status(session_key=args.session)
            if not status.get("ok"):
                res = {"ok": False, "action": "analyze", "error": status.get("error", "invalid session key")}
                _emit(res)
                return 1
            target = str(status.get("remotePath") or "").strip() or None
        if not target:
            hint = f" (no trace metadata for session '{args.session}')" if args.session else ""
            res = {"ok": False, "action": "analyze", "error": f"--path is required for --action analyze{hint}"}
        else:
            res = perfetto_analyze.analyze_trace_file(target)
            if not args.path and not args.out and args.session and isinstance(res, dict):
                res.setdefault("sessionKey", args.session)
    else:
        buf = args.buffer_kb if args.buffer_kb is not None else perfetto_ops.DEFAULT_BUFFER_KB
        res = perfetto_ops.capture_perfetto(
            preset=args.preset,
            duration_sec=args.duration,
            buffer_kb=buf,
            out_path=args.out,
            analyze=args.analyze,
        )
    _emit(res)
    return 0 if res.get("ok") else 1


ANDROID_DISPATCH: dict[str, Callable[[argparse.Namespace], int]] = {
    "ui": _cmd_ui,
    "app": _cmd_app,
    "lsposed": _cmd_lsposed,
    "db": _cmd_db,
    "ksu": _cmd_ksu,
    "perfetto": _cmd_perfetto,
}


def register_android_subparsers(sub: argparse._SubParsersAction[argparse.ArgumentParser]) -> None:
    """Registers ui, app, lsposed, db, and ksu subcommand parsers."""
    p_ui = sub.add_parser("ui")
    ui_sub = p_ui.add_subparsers(dest="ui_cmd", required=True)
    p_ud = ui_sub.add_parser("dump")
    p_ud.add_argument("--compressed", action="store_true")
    p_ud.add_argument("--query")
    p_ud.add_argument("--resource-id")
    p_ud.add_argument("--desc")
    p_ud.add_argument("--clickable-only", action="store_true")
    p_ud.add_argument("--limit", type=int, default=80)
    p_ut = ui_sub.add_parser("tap")
    p_ut.add_argument("--text")
    p_ut.add_argument("--resource-id")
    p_ut.add_argument("--desc")
    p_ut.add_argument("--index", type=int, default=0)
    p_ut.add_argument("--method", choices=["input", "uinput"], default="input")

    p_app = sub.add_parser("app")
    app_sub = p_app.add_subparsers(dest="app_cmd", required=True)
    app_sub.add_parser("top")
    p_ai = app_sub.add_parser("inspect")
    p_ai.add_argument("package", nargs="?", default="auto")
    p_as = app_sub.add_parser("start")
    p_as.add_argument("package")
    p_as.add_argument("--activity")
    p_as.add_argument("--action")
    p_as.add_argument("--data")
    p_astop = app_sub.add_parser("stop")
    p_astop.add_argument("package")
    p_ac = app_sub.add_parser("clear")
    p_ac.add_argument("package")
    p_ain = app_sub.add_parser("install")
    p_ain.add_argument("apk_path")
    p_ain.add_argument("--grant", action=argparse.BooleanOptionalAction, default=True)
    p_ain.add_argument("--downgrade", action="store_true")
    p_ain.add_argument("--no-replace", action="store_true")

    p_lsp = sub.add_parser("lsposed")
    lsp_sub = p_lsp.add_subparsers(dest="lsposed_cmd", required=True)
    p_lm = lsp_sub.add_parser("modules")
    p_lm.add_argument("--module")
    p_ll = lsp_sub.add_parser("logs")
    p_ll.add_argument("--module")
    p_ll.add_argument("--grep")
    p_ll.add_argument("--limit", type=int, default=80)

    p_db = sub.add_parser("db")
    db_sub = p_db.add_subparsers(dest="db_cmd", required=True)
    p_dt = db_sub.add_parser("tables")
    p_dt.add_argument("db_path")
    p_dq = db_sub.add_parser("query")
    p_dq.add_argument("db_path")
    p_dq.add_argument("--sql", required=True)
    p_dq.add_argument("--limit", type=int, default=50)

    p_ksu = sub.add_parser("ksu")
    ksu_sub = p_ksu.add_subparsers(dest="ksu_cmd", required=True)
    ksu_sub.add_parser("modules")
    ksu_sub.add_parser("inputs")
    p_ks = ksu_sub.add_parser("sysfs")
    p_ks.add_argument("path")

    p_pf = sub.add_parser("perfetto")
    p_pf.add_argument(
        "--action",
        choices=["capture", "start", "status", "snapshot", "stop", "analyze"],
        default="capture",
    )
    p_pf.add_argument("--preset", choices=list(perfetto_ops.PRESETS), default="ui-jank")
    p_pf.add_argument("--duration", type=int, default=perfetto_ops.DEFAULT_DURATION_SEC)
    p_pf.add_argument("--buffer-kb", type=int, default=None)
    p_pf.add_argument("--max-duration", type=int, default=perfetto_session.DEFAULT_RING_DURATION_SEC)
    p_pf.add_argument("--session", default=perfetto_session.DEFAULT_SESSION_KEY)
    p_pf.add_argument("--keep-running", action=argparse.BooleanOptionalAction, default=True)
    p_pf.add_argument("--analyze", action=argparse.BooleanOptionalAction, default=True)
    p_pf.add_argument("--out")
    p_pf.add_argument("--path")
