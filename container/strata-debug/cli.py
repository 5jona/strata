#!/usr/bin/env python3
"""Unified CLI entry point for `/usr/local/bin/strata-debug`."""

from __future__ import annotations

import argparse
import json
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

import cdp_bridge
import cli_android
import forward_bridge
import gesture_replay
import kernel_probe
import logcat_cursor
import observe_step
import sys_probe
import visual_check


def _emit(payload: object) -> int:
    print(json.dumps(payload, ensure_ascii=False))
    return 0


def _cmd_cdp(args: argparse.Namespace) -> int:
    if args.cdp_cmd == "list-targets":
        return _emit(cdp_bridge.list_targets(target=args.target, pid=args.pid))
    if args.cdp_cmd == "serve":
        matched = cdp_bridge.match_devtools_socket(target=args.target, pid=args.pid)
        if not matched:
            return _emit({"ok": False, "error": "no devtools socket found"})
        return cdp_bridge.serve_cdp_proxy(args.port, matched["socket"])
    if args.cdp_cmd == "eval":
        return _emit(cdp_bridge.eval_js(
            args.expression, target=args.target, pid=args.pid, url_match=args.url_match, timeout=args.timeout
        ))
    if args.cdp_cmd == "console":
        return _emit(cdp_bridge.collect_console(
            target=args.target, pid=args.pid, url_match=args.url_match,
            duration_s=args.duration, timeout=args.timeout,
        ))
    if args.cdp_cmd == "audit":
        return _emit(cdp_bridge.audit_webview(
            target=args.target, pid=args.pid,
            url_contains=args.url_contains or args.url_match,
            eval_js=args.eval_js, timeout=args.timeout,
        ))
    return 1


def _cmd_forward(args: argparse.Namespace) -> int:
    fwd_args: list[str] = []
    if args.list_forwards:
        fwd_args = ["--list"]
    elif args.remove_all:
        fwd_args = ["--remove-all"]
    elif args.remove:
        fwd_args = ["--remove", args.remove]
    else:
        if args.no_rebind:
            fwd_args.append("--no-rebind")
        fwd_args.extend(args.endpoints)
    rc, msg = forward_bridge.handle_forward(fwd_args)
    if args.list_forwards:
        lines = [line for line in msg.splitlines() if line.strip()]
        _emit({"ok": rc == 0, "forwards": lines})
        return rc
    payload: dict[str, object] = {"ok": rc == 0}
    if msg:
        payload["error" if rc != 0 else "message"] = msg
    elif len(args.endpoints) == 2 and not args.remove and not args.remove_all:
        payload["local"] = args.endpoints[0]
        payload["remote"] = args.endpoints[1]
    _emit(payload)
    return rc


def _cmd_gesture(args: argparse.Namespace) -> int:
    geom = gesture_replay.get_display_geometry()
    lw, lh, dpi = args.width or geom["logical_w"], args.height or geom["logical_h"], args.dpi or geom["dpi"]
    pw, ph, rot = geom["phys_w"], geom["phys_h"], geom["rotation"]
    meta = {"rotation": rot, "logicalSize": [lw, lh], "physicalSize": [pw, ph]}
    with gesture_replay.UInputTouchDevice(pw, ph, logical_width=lw, logical_height=lh, rotation=rot) as dev:
        if args.gesture_cmd == "edge-swipe":
            pts = gesture_replay.compute_edge_swipe_points(
                lw, lh, dpi, side=args.side, y_percent=args.y_percent,
                distance_dp=args.distance_dp, duration_ms=args.duration_ms, cancel_back=args.cancel_back,
            )
            dev.replay_points(pts)
            return _emit({"ok": True, "gesture": "edge-swipe", "points": len(pts), "screen": [lw, lh, dpi], **meta})
        if args.gesture_cmd == "tap":
            dev.tap(args.x, args.y)
            return _emit({"ok": True, "gesture": "tap", "x": args.x, "y": args.y, **meta})
        if args.gesture_cmd == "long-press":
            dev.tap(args.x, args.y, hold_ms=args.duration_ms)
            return _emit({"ok": True, "gesture": "long-press", "x": args.x, "y": args.y, "durationMs": args.duration_ms, **meta})
        if args.gesture_cmd == "swipe":
            dev.swipe(args.x0, args.y0, args.x1, args.y1, duration_ms=args.duration_ms)
            return _emit({"ok": True, "gesture": "swipe", **meta})
    return 1


def _cmd_logcat(args: argparse.Namespace) -> int:
    if args.mark:
        return _emit({"ok": True, "marked": logcat_cursor.mark_cursor(args.cursor)})
    return _emit(logcat_cursor.read_incremental(
        args.cursor, priority=args.priority, tag=args.tag, pid=args.pid, limit=args.limit, advance=not args.no_advance
    ))


def _cmd_sys_probe(args: argparse.Namespace) -> int:
    inc_sv: bool | None = False if args.no_sideveil else (True if args.include_sideveil else None)
    return _emit(sys_probe.collect_sys_probe(
        package=args.package, include_surfaceflinger=not args.no_sf, include_sideveil=inc_sv
    ))


def _cmd_visual(args: argparse.Namespace) -> int:
    return _emit(visual_check.run_visual_check(
        roi_spec=args.roi, out_path=args.out, fmt=args.format, quality=args.quality,
        golden_raw=args.golden, burst_count=4 if args.burst_sheet else 1,
        burst_interval_ms=args.burst_interval_ms, max_width=args.max_width,
    ))


def _cmd_kernel(args: argparse.Namespace) -> int:
    if args.kernel_cmd == "pstore":
        return _emit(kernel_probe.collect_pstore(args.dir))
    if args.kernel_cmd == "dmesg":
        return _emit(kernel_probe.collect_dmesg(limit=args.limit, filter_pattern=args.filter))
    if args.kernel_cmd == "tracing":
        return _emit(kernel_probe.inspect_tracing(args.dir, limit=args.limit))
    if args.kernel_cmd == "trace":
        return _emit(kernel_probe.run_trace_preset(args.preset, duration_s=args.duration))
    return 1


def _cmd_observe(args: argparse.Namespace) -> int:
    rpc = tuple(args.action_rpc) if args.action_rpc else None
    gesture = json.loads(args.action_gesture) if args.action_gesture else None
    return _emit(observe_step.run_observe_step(
        action_rpc=rpc, action_shell=args.action_shell, action_cdp_eval=args.action_cdp_eval,  # type: ignore[arg-type]
        action_gesture=gesture, settle_ms=args.settle_ms, package=args.package, logcat_priority=args.priority,
        logcat_tag=args.tag, include_cdp=not args.no_cdp, cdp_pid=args.cdp_pid,
        include_visual=args.visual, no_visual=args.no_visual,
        roi_spec=args.roi, visual_out=args.visual_out, golden_raw=args.golden, cursor_path=args.cursor,
    ))


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="strata-debug", description="Strata L1-L5 Android debug CLI")
    sub = p.add_subparsers(dest="cmd", required=True)

    p_cdp = sub.add_parser("cdp")
    cdp_sub = p_cdp.add_subparsers(dest="cdp_cmd", required=True)
    p_cl = cdp_sub.add_parser("list-targets")
    p_cl.add_argument("--target")
    p_cl.add_argument("--pid", type=int)
    p_cs = cdp_sub.add_parser("serve")
    p_cs.add_argument("--port", type=int, default=9222)
    p_cs.add_argument("--target", default=None)
    p_cs.add_argument("--pid", type=int)
    p_ce = cdp_sub.add_parser("eval")
    p_ce.add_argument("expression")
    p_ce.add_argument("--target", default=None)
    p_ce.add_argument("--pid", type=int)
    p_ce.add_argument("--url-match")
    p_ce.add_argument("--timeout", type=float, default=5.0)
    p_cc = cdp_sub.add_parser("console")
    p_cc.add_argument("--target", default=None)
    p_cc.add_argument("--pid", type=int)
    p_cc.add_argument("--url-match")
    p_cc.add_argument("--duration", type=float, default=2.0)
    p_cc.add_argument("--timeout", type=float, default=5.0)
    p_ca = cdp_sub.add_parser("audit")
    p_ca.add_argument("--target", default=None)
    p_ca.add_argument("--pid", type=int)
    p_ca.add_argument("--url-contains")
    p_ca.add_argument("--url-match")
    p_ca.add_argument("--eval-js")
    p_ca.add_argument("--timeout", type=float, default=5.0)

    p_f = sub.add_parser("forward")
    p_f.add_argument("--list", dest="list_forwards", action="store_true")
    p_f.add_argument("--remove")
    p_f.add_argument("--remove-all", action="store_true")
    p_f.add_argument("--no-rebind", action="store_true")
    p_f.add_argument("endpoints", nargs="*")

    p_g = sub.add_parser("gesture")
    p_g.add_argument("--width", type=int)
    p_g.add_argument("--height", type=int)
    p_g.add_argument("--dpi", type=int)
    g_sub = p_g.add_subparsers(dest="gesture_cmd", required=True)
    p_ge = g_sub.add_parser("edge-swipe")
    p_ge.add_argument("--side", choices=["left", "right"], default="right")
    p_ge.add_argument("--y-percent", type=float, default=50.0)
    p_ge.add_argument("--distance-dp", type=float, default=96.0)
    p_ge.add_argument("--duration-ms", type=int, default=240)
    p_ge.add_argument("--cancel-back", action="store_true")
    p_gt = g_sub.add_parser("tap")
    p_gt.add_argument("x", type=int)
    p_gt.add_argument("y", type=int)
    p_gl = g_sub.add_parser("long-press")
    p_gl.add_argument("x", type=int)
    p_gl.add_argument("y", type=int)
    p_gl.add_argument("--duration-ms", type=int, default=600)
    p_gs = g_sub.add_parser("swipe")
    p_gs.add_argument("x0", type=int)
    p_gs.add_argument("y0", type=int)
    p_gs.add_argument("x1", type=int)
    p_gs.add_argument("y1", type=int)
    p_gs.add_argument("--duration-ms", type=int, default=240)

    p_l = sub.add_parser("logcat")
    p_l.add_argument("--mark", action="store_true")
    p_l.add_argument("--cursor", default=logcat_cursor.DEFAULT_CURSOR_PATH)
    p_l.add_argument("--priority", default="W")
    p_l.add_argument("--tag")
    p_l.add_argument("--pid", type=int)
    p_l.add_argument("--limit", type=int, default=100)
    p_l.add_argument("--no-advance", action="store_true")

    p_sp = sub.add_parser("sys-probe")
    p_sp.add_argument("--package", default="auto")
    p_sp.add_argument("--no-sf", action="store_true")
    p_sp.add_argument("--include-sideveil", action="store_true")
    p_sp.add_argument("--no-sideveil", action="store_true")

    p_v = sub.add_parser("visual")
    p_v.add_argument("--roi", help="x,y,w,h")
    p_v.add_argument("--out")
    p_v.add_argument("--format", default="webp")
    p_v.add_argument("--quality", type=int, default=75)
    p_v.add_argument("--max-width", type=int)
    p_v.add_argument("--golden")
    p_v.add_argument("--burst-sheet", action="store_true")
    p_v.add_argument("--burst-interval-ms", type=int, default=80)

    p_k = sub.add_parser("kernel")
    k_sub = p_k.add_subparsers(dest="kernel_cmd", required=True)
    p_kp = k_sub.add_parser("pstore")
    p_kp.add_argument("--dir", default="/sys/fs/pstore")
    p_kd = k_sub.add_parser("dmesg")
    p_kd.add_argument("--limit", type=int, default=80)
    p_kd.add_argument("--filter")
    p_ktr = k_sub.add_parser("tracing")
    p_ktr.add_argument("--dir", default="/sys/kernel/tracing")
    p_ktr.add_argument("--limit", type=int, default=40)
    p_kt = k_sub.add_parser("trace")
    p_kt.add_argument("preset", choices=["guard-audit", "net-uid", "sched-lat"])
    p_kt.add_argument("--duration", type=int, default=5)

    p_o = sub.add_parser("observe")
    p_o.add_argument("--action-rpc", nargs=2, metavar=("METHOD", "JSON"))
    p_o.add_argument("--action-shell")
    p_o.add_argument("--action-cdp-eval")
    p_o.add_argument("--action-gesture", help="JSON gesture spec")
    p_o.add_argument("--cursor", help="Incremental logcat cursor path")
    p_o.add_argument("--settle-ms", type=int, default=350)
    p_o.add_argument("--package", default="auto")
    p_o.add_argument("--priority", default="W")
    p_o.add_argument("--tag")
    p_o.add_argument("--no-cdp", action="store_true")
    p_o.add_argument("--cdp-pid", type=int)
    p_o.add_argument("--visual", action="store_true")
    p_o.add_argument("--no-visual", action="store_true")
    p_o.add_argument("--roi")
    p_o.add_argument("--visual-out")
    p_o.add_argument("--golden")

    cli_android.register_android_subparsers(sub)
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    dispatch = {
        "cdp": _cmd_cdp,
        "forward": _cmd_forward,
        "gesture": _cmd_gesture,
        "logcat": _cmd_logcat,
        "sys-probe": _cmd_sys_probe,
        "visual": _cmd_visual,
        "kernel": _cmd_kernel,
        "observe": _cmd_observe,
        **cli_android.ANDROID_DISPATCH,
    }
    return dispatch[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
