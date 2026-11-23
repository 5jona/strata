"""L4 atomic single-shot observe step: optional mark -> act -> settle -> collect structured signals."""

from __future__ import annotations

import json
import os
import subprocess
import time
from typing import Any

import cdp_bridge
import gesture_replay
import logcat_cursor
import sys_probe
import visual_check

HOST_EXEC = "/usr/local/bin/host-exec"
STRATA_BIN = "/data/adb/ksu/bin/strata"


def _run_action(
    *,
    action_rpc: tuple[str, str] | None = None,
    action_shell: str | None = None,
    action_cdp_eval: str | None = None,
    action_gesture: dict[str, Any] | None = None,
    cdp_target: str | None = None,
    cdp_pid: int | None = None,
) -> dict[str, Any] | None:
    if action_rpc:
        method, params_json = action_rpc
        res = subprocess.run(
            [HOST_EXEC, STRATA_BIN, "--json", "rpc", method, params_json],
            capture_output=True,
            text=True,
            timeout=15.0,
            check=False,
        )
        try:
            body: Any = json.loads(res.stdout) if res.stdout.strip() else None
        except json.JSONDecodeError:
            body = res.stdout.strip()
        return {"type": "rpc", "method": method, "rc": res.returncode, "result": body, "stderr": res.stderr.strip()[:240]}

    if action_shell:
        res = subprocess.run(
            [HOST_EXEC, "/system/bin/sh", "-c", action_shell],
            capture_output=True,
            text=True,
            timeout=15.0,
            check=False,
        )
        return {
            "type": "shell",
            "rc": res.returncode,
            "stdout": res.stdout.strip()[-1000:],
            "stderr": res.stderr.strip()[-400:],
        }

    if action_cdp_eval:
        out = cdp_bridge.eval_js(action_cdp_eval, target=cdp_target, pid=cdp_pid)
        return {"type": "cdp-eval", "ok": out.get("ok"), "evalResult": out.get("value"), "error": out.get("error")}

    if action_gesture:
        kind = action_gesture.get("kind", "edge-swipe")
        geom = gesture_replay.get_display_geometry()
        w, h, dpi = geom["logical_w"], geom["logical_h"], geom["dpi"]
        pw, ph, rot = geom["phys_w"], geom["phys_h"], geom["rotation"]
        with gesture_replay.UInputTouchDevice(pw, ph, logical_width=w, logical_height=h, rotation=rot) as dev:
            if kind == "tap":
                dev.tap(int(action_gesture["x"]), int(action_gesture["y"]))
            elif kind == "long-press":
                dev.tap(
                    int(action_gesture["x"]),
                    int(action_gesture["y"]),
                    hold_ms=int(action_gesture.get("durationMs", 600)),
                )
            elif kind == "swipe":
                dev.swipe(
                    int(action_gesture["x0"]),
                    int(action_gesture["y0"]),
                    int(action_gesture["x1"]),
                    int(action_gesture["y1"]),
                    duration_ms=int(action_gesture.get("durationMs", 240)),
                )
            else:
                pts = gesture_replay.compute_edge_swipe_points(
                    w,
                    h,
                    dpi,
                    side=str(action_gesture.get("side", "right")),
                    y_percent=float(action_gesture.get("yPercent", 50.0)),
                    distance_dp=float(action_gesture.get("distanceDp", 96.0)),
                    duration_ms=int(action_gesture.get("durationMs", 240)),
                    cancel_back=bool(action_gesture.get("cancelBack", False)),
                )
                dev.replay_points(pts)
        return {
            "type": "gesture",
            "kind": kind,
            "rotation": rot,
            "logicalSize": [w, h],
            "physicalSize": [pw, ph],
            "screen": {"width": w, "height": h, "dpi": dpi},
        }

    return None


def run_observe_step(
    *,
    action_rpc: tuple[str, str] | None = None,
    action_shell: str | None = None,
    action_cdp_eval: str | None = None,
    action_gesture: dict[str, Any] | None = None,
    settle_ms: int = 350,
    package: str = "auto",
    logcat_priority: str = "W",
    logcat_tag: str | None = None,
    include_cdp: bool = True,
    cdp_pid: int | None = None,
    include_visual: bool = False,
    no_visual: bool = False,
    roi_spec: str | None = None,
    visual_out: str | None = None,
    golden_raw: str | None = None,
    cursor_path: str | None = None,
) -> dict[str, Any]:
    t0 = time.monotonic()
    resolved_pkg: str | None = (
        sys_probe.detect_foreground_package() if package in ("", "auto") else package
    )
    has_action = any(x is not None for x in (action_rpc, action_shell, action_cdp_eval, action_gesture))
    eff_cursor = cursor_path or logcat_cursor.DEFAULT_CURSOR_PATH

    cursor_before: dict[str, Any] | None = None
    if cursor_path is None:
        cursor_before = logcat_cursor.mark_cursor(eff_cursor)
    elif has_action and not os.path.isfile(eff_cursor):
        cursor_before = logcat_cursor.mark_cursor(eff_cursor)

    action_result = None
    action_error = None
    try:
        action_result = _run_action(
            action_rpc=action_rpc,
            action_shell=action_shell,
            action_cdp_eval=action_cdp_eval,
            action_gesture=action_gesture,
            cdp_target=resolved_pkg,
            cdp_pid=cdp_pid,
        )
    except Exception as exc:
        action_error = str(exc)

    if settle_ms > 0 and (has_action or cursor_path is None):
        time.sleep(settle_ms / 1000.0)

    logs = logcat_cursor.read_incremental(
        eff_cursor, priority=logcat_priority, tag=logcat_tag, limit=60, advance=True
    )
    sys_info = sys_probe.collect_sys_probe(
        package=resolved_pkg or "unknown", include_sideveil=None
    )

    cdp_info: dict[str, Any] | None = None
    if include_cdp:
        try:
            cdp_info = cdp_bridge.audit_webview(target=resolved_pkg, pid=cdp_pid)
        except Exception as exc:
            cdp_info = {"ok": False, "error": str(exc)}

    vis_info: dict[str, Any] | None = None
    want_visual = (not no_visual) and (include_visual or bool(roi_spec) or bool(visual_out))
    if want_visual:
        try:
            vis_info = visual_check.run_visual_check(
                roi_spec=roi_spec, out_path=visual_out, golden_raw=golden_raw
            )
        except Exception as exc:
            vis_info = {"ok": False, "error": str(exc)}

    return {
        "ok": action_error is None,
        "elapsedMs": int((time.monotonic() - t0) * 1000),
        "cursorPath": eff_cursor,
        "cursorBefore": cursor_before.get("ts") if cursor_before else None,
        "actionResult": action_result,
        "actionError": action_error,
        "logcat": logs,
        "sysProbe": sys_info,
        "cdp": cdp_info,
        "visual": vis_info,
    }
