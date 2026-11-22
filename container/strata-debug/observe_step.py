"""L4 atomic single-shot observe step: mark -> act -> settle -> collect structured signals."""

from __future__ import annotations

import json
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
    cdp_target: str = "pochita.sideveil.host",
    cdp_pid: int | None = None,
) -> dict[str, Any] | None:
    if action_rpc:
        method, params_json = action_rpc
        res = subprocess.run(
            [HOST_EXEC, STRATA_BIN, "--json", "rpc", method, params_json],
            capture_output=True,
            text=True,
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
            check=False,
        )
        return {
            "type": "shell",
            "rc": res.returncode,
            "stdout": res.stdout.strip()[-1000:],
            "stderr": res.stderr.strip()[-400:],
        }

    if action_cdp_eval:
        out = cdp_bridge.audit_webview(target=cdp_target, pid=cdp_pid, eval_js=action_cdp_eval)
        return {"type": "cdp-eval", "ok": out.get("ok"), "evalResult": out.get("evalResult"), "error": out.get("error")}

    if action_gesture:
        kind = action_gesture.get("kind", "edge-swipe")
        w, h, dpi = gesture_replay.get_screen_metrics()
        with gesture_replay.UInputTouchDevice(w, h) as dev:
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
        return {"type": "gesture", "kind": kind, "screen": {"width": w, "height": h, "dpi": dpi}}

    return None


def run_observe_step(
    *,
    action_rpc: tuple[str, str] | None = None,
    action_shell: str | None = None,
    action_cdp_eval: str | None = None,
    action_gesture: dict[str, Any] | None = None,
    settle_ms: int = 350,
    package: str = "pochita.sideveil.host",
    logcat_priority: str = "W",
    logcat_tag: str | None = None,
    include_cdp: bool = True,
    cdp_pid: int | None = None,
    include_visual: bool = False,
    roi_spec: str | None = None,
    visual_out: str | None = None,
    golden_raw: str | None = None,
    cursor_path: str = logcat_cursor.DEFAULT_CURSOR_PATH,
) -> dict[str, Any]:
    t0 = time.monotonic()
    cursor_before = logcat_cursor.mark_cursor(cursor_path)

    action_result = None
    action_error = None
    try:
        action_result = _run_action(
            action_rpc=action_rpc,
            action_shell=action_shell,
            action_cdp_eval=action_cdp_eval,
            action_gesture=action_gesture,
            cdp_target=package,
            cdp_pid=cdp_pid,
        )
    except Exception as exc:
        action_error = str(exc)

    if settle_ms > 0:
        time.sleep(settle_ms / 1000.0)

    logs = logcat_cursor.read_incremental(
        cursor_path, priority=logcat_priority, tag=logcat_tag, limit=60, advance=True
    )
    sys_info = sys_probe.collect_sys_probe(package=package, include_sideveil=True)

    cdp_info: dict[str, Any] | None = None
    if include_cdp:
        try:
            cdp_info = cdp_bridge.audit_webview(target=package, pid=cdp_pid)
        except Exception as exc:
            cdp_info = {"ok": False, "error": str(exc)}

    vis_info: dict[str, Any] | None = None
    if include_visual or roi_spec or visual_out:
        try:
            vis_info = visual_check.run_visual_check(
                roi_spec=roi_spec, out_path=visual_out, golden_raw=golden_raw
            )
        except Exception as exc:
            vis_info = {"ok": False, "error": str(exc)}

    return {
        "ok": action_error is None,
        "elapsedMs": int((time.monotonic() - t0) * 1000),
        "cursorBefore": cursor_before.get("ts"),
        "actionResult": action_result,
        "actionError": action_error,
        "logcat": logs,
        "sysProbe": sys_info,
        "cdp": cdp_info,
        "visual": vis_info,
    }
