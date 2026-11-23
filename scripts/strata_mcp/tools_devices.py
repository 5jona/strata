"""Multi-device management and Perfetto trace MCP tools (devices_list, device_select, perfetto_trace)."""

from __future__ import annotations

import os
import tempfile
import time
from typing import Any, Callable

from . import devices
from .transport import (
    B,
    I,
    S,
    argv_to_cmd,
    extract_ssh_overrides,
    invoke_exec,
    make_tool,
    opt_flag,
    scp_pull,
    ssh_exec,
)

DEVICE_TOOLS: list[dict[str, Any]] = [
    make_tool(
        "devices_list",
        "List all discovered Android/Strata devices (id, model, serial, usb/wireless status, sshHost, selected).",
        {},
        include_target=False,
    ),
    make_tool(
        "device_select",
        "Set or clear (empty device or clear=true) the session default target device.",
        {
            "device": {"type": "string", "description": "Device alias, ADB serial, model, or SSH host; empty to clear."},
            "clear": B,
        },
        include_target=False,
    ),
    make_tool(
        "perfetto_trace",
        "Capture, manage ring-buffer sessions (start/status/snapshot/stop), or analyze Perfetto traces via strata-debug perfetto.",
        {
            "action": {"type": "string", "enum": ["capture", "start", "status", "snapshot", "stop", "analyze"]},
            "preset": {"type": "string", "enum": ["ui-jank", "sched", "binder", "memory"]},
            "durationSec": I,
            "bufferKb": I,
            "sessionKey": S,
            "maxDurationSec": I,
            "keepRunning": B,
            "analyze": B,
            "remotePath": S,
            "pullToLocal": B,
            "localOut": S,
        },
    ),
]

DEVICE_TOOL_NAMES = {t["name"] for t in DEVICE_TOOLS}


def _handle_perfetto_trace(
    args: dict[str, Any],
    run_argv: Callable[[list[str], float], dict[str, Any]],
    pull_fn: Callable[..., dict[str, Any]],
) -> dict[str, Any]:
    action = str(args.get("action") or "capture")
    preset = str(args.get("preset") or "ui-jank")
    if action == "capture":
        dur = max(1, min(15, int(args.get("durationSec", 3))))
        cmd = ["strata-debug", "perfetto", "--preset", preset, "--duration", str(dur)]
        if args.get("bufferKb") is not None:
            opt_flag(cmd, "--buffer-kb", int(args["bufferKb"]))
        timeout = float(dur) + 25.0
    elif action == "start":
        cmd = ["strata-debug", "perfetto", "--action", "start", "--preset", preset]
        opt_flag(cmd, "--session", args.get("sessionKey"))
        if args.get("bufferKb") is not None:
            opt_flag(cmd, "--buffer-kb", int(args["bufferKb"]))
        if args.get("maxDurationSec") is not None:
            opt_flag(cmd, "--max-duration", max(60, min(3600, int(args["maxDurationSec"]))))
        timeout = 30.0
    elif action == "status":
        cmd = ["strata-debug", "perfetto", "--action", "status"]
        opt_flag(cmd, "--session", args.get("sessionKey"))
        timeout = 20.0
    elif action in {"snapshot", "stop"}:
        cmd = ["strata-debug", "perfetto", "--action", action]
        opt_flag(cmd, "--session", args.get("sessionKey"))
        if action == "snapshot":
            if args.get("keepRunning") is False:
                cmd.append("--no-keep-running")
            elif args.get("keepRunning") is True:
                cmd.append("--keep-running")
        timeout = 45.0
    elif action == "analyze":
        cmd = ["strata-debug", "perfetto", "--action", "analyze"]
        opt_flag(cmd, "--path", args.get("remotePath"))
        opt_flag(cmd, "--session", args.get("sessionKey"))
        timeout = 45.0
    else:
        raise ValueError(f"unsupported perfetto action: {action}")

    if action in {"capture", "snapshot", "stop"}:
        if args.get("analyze") is True:
            cmd.append("--analyze")
        elif args.get("analyze") is False:
            cmd.append("--no-analyze")

    res = run_argv(cmd, timeout)
    want_pull = action in {"capture", "snapshot", "stop"} and (bool(args.get("pullToLocal")) or bool(args.get("localOut")))
    if want_pull and res.get("rc") == 0:
        payload = res.get("json") if isinstance(res.get("json"), dict) else {}
        remote_path = str(
            payload.get("remotePath")
            or payload.get("tracePath")
            or payload.get("outputPath")
            or payload.get("path")
            or payload.get("out")
            or ""
        ).strip()
        if not remote_path:
            res["pulled"] = False
            res["pullError"] = "perfetto result did not contain a remote trace path"
            return res
        local_out = str(args.get("localOut") or "").strip()
        if not local_out:
            trace_dir = os.path.join(tempfile.gettempdir(), "strata-mcp-traces")
            tag = preset if action == "capture" else action
            local_out = os.path.join(trace_dir, f"trace-{tag}-{int(time.time() * 1000)}.perfetto-trace")
        ssh_kw = extract_ssh_overrides(args)
        if payload.get("hostScope") or remote_path.startswith("/data/"):
            ssh_kw["host_scope"] = True
        pull_res = pull_fn(remote_path, local_out, **ssh_kw) if ssh_kw else pull_fn(remote_path, local_out)
        if pull_res.get("ok"):
            res["pulled"] = True
            res["localPath"] = local_out
            res["remotePath"] = remote_path
        else:
            res["pulled"] = False
            res["pullError"] = pull_res.get("stderr", "pull failed")
    return res


def dispatch_device_tool(
    name: str,
    args: dict[str, Any],
    *,
    exec_fn: Callable[..., dict[str, Any]] = ssh_exec,
    pull_fn: Callable[..., dict[str, Any]] = scp_pull,
) -> Any:
    if name == "devices_list":
        devs = devices.discover_devices()
        sel = devices.get_selected_device()
        return {
            "ok": True,
            "selected": sel,
            "count": len(devs),
            "devices": [d.to_dict(selected_id=sel) for d in devs],
        }

    if name == "device_select":
        if args.get("clear") or not str(args.get("device") or "").strip():
            devices.select_device(None)
            return {"ok": True, "selected": None, "cleared": True}
        chosen = devices.select_device(str(args["device"]))
        if chosen is None:
            return {"ok": False, "selected": None, "error": f"failed to select device '{args['device']}'"}
        return {"ok": True, "selected": chosen.id, "device": chosen.to_dict(selected_id=chosen.id)}

    if name == "perfetto_trace":
        def run_argv(argv: list[str], timeout: float = 30.0) -> dict[str, Any]:
            return invoke_exec(exec_fn, argv_to_cmd(argv), timeout=timeout, args=args)

        return _handle_perfetto_trace(args, run_argv, pull_fn)

    raise KeyError(f"unknown device tool '{name}'")
