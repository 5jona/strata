"""Core Strata MCP tools (14 generalized debugging, CDP, logcat, visual, gesture, and kernel tools)."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
from typing import Any, Callable

from .transport import (
    B,
    I,
    N,
    S,
    argv_to_cmd,
    extract_ssh_overrides,
    invoke_exec,
    make_tool,
    open_ssh_port_forward,
    opt_flag,
    resolve_ssh_target,
    scp_pull,
    ssh_exec,
)

CORE_TOOLS: list[dict[str, Any]] = [
    make_tool("observe_step", "Atomic L4 observe: mark/cursor -> action -> settle -> logcat + sys-probe + CDP + visual.", {
        "actionRpc": {"type": "array", "items": S, "minItems": 2, "maxItems": 2},
        "actionShell": S, "actionCdpEval": S, "actionGesture": {"type": "object"}, "cursor": S,
        "settleMs": I, "package": S, "priority": S, "tag": S,
        "includeCdp": B, "includeVisual": B, "noVisual": B, "roi": S,
    }),
    make_tool("cdp_list_targets", "List discovered WebView/Chrome DevTools abstract sockets and /json/list targets.", {
        "target": S, "pid": I,
    }),
    make_tool("cdp_eval", "Evaluate JavaScript in a WebView target over abstract UDS CDP WebSocket.", {
        "expression": S, "target": S, "pid": I, "urlMatch": S, "timeoutSec": N,
    }, req=["expression"]),
    make_tool("cdp_console", "Capture WebView console events and uncaught exceptions over CDP.", {
        "target": S, "pid": I, "urlMatch": S, "durationSec": N, "timeoutSec": N,
    }),
    make_tool("cdp_audit", "L1 WebView DOM health audit (overflow, truncation, small targets, overlap, offscreen, contrast, console errors).", {
        "target": S, "pid": I, "urlContains": S, "urlMatch": S, "evalJs": S,
    }),
    make_tool("logcat_cursor", "L2a incremental logcat cursor: --mark watermark or read new lines with crash highlights.", {
        "mark": B, "cursor": S, "priority": S, "tag": S, "pid": I, "limit": I, "noAdvance": B,
    }),
    make_tool("visual_check", "L3 raw screencap health check (black/blank/solid-color/blur/SSIM) with optional downsampled pull to PC.", {
        "roi": S, "out": S, "format": S, "quality": I, "maxWidth": I, "golden": S, "burstSheet": B,
        "pullToLocal": B, "localOut": S,
    }),
    make_tool("gesture_replay", "L2b /dev/uinput multitouch Protocol B virtual touchscreen gesture replay.", {
        "kind": {"type": "string", "enum": ["edge-swipe", "tap", "long-press", "swipe"]},
        "side": {"type": "string", "enum": ["left", "right"]}, "yPercent": N, "distanceDp": N,
        "durationMs": I, "cancelBack": B, "x": I, "y": I, "x0": I, "y0": I, "x1": I, "y1": I,
    }),
    make_tool("kernel_diag", "L5 read-only kernel diagnostics: pstore ramoops, dmesg ring buffer, or tracefs status.", {
        "mode": {"type": "string", "enum": ["pstore", "dmesg", "tracing"]}, "limit": I, "filter": S, "dir": S,
    }),
    make_tool("kernel_probe", "L5 read-only bpftrace/tracefs kernel probe preset (guard-audit, net-uid, sched-lat).", {
        "preset": {"type": "string", "enum": ["guard-audit", "net-uid", "sched-lat"]}, "durationSec": I,
    }, req=["preset"]),
    make_tool("pstore_dump", "L5 read-only /sys/fs/pstore ramoops dump and OEM watchdog/panic parser.", {}),
    make_tool("device_health_snapshot", "Strata daemon status, container list, and Android window/gfxinfo probe.", {
        "package": S,
    }),
    make_tool("reload_and_verify", "SSH hot-reload (reload.sh --ssh) + non-destructive live verification.", {
        "skipBuild": B, "uiOnly": B,
    }),
    make_tool("forward_cdp", "Forward a device WebView DevTools abstract socket to PC localhost via strata-debug + SSH.", {
        "localPort": I, "target": S, "pid": I,
    }),
]

CORE_TOOL_NAMES = {t["name"] for t in CORE_TOOLS}


def _handle_visual_check(
    args: dict[str, Any],
    run_argv: Callable[[list[str], float], dict[str, Any]],
    pull_fn: Callable[..., dict[str, Any]],
) -> dict[str, Any]:
    fmt = str(args.get("format", "webp"))
    ext = "png" if fmt.lower() == "png" else ("jpg" if fmt.lower() in {"jpg", "jpeg"} else "webp")
    want_pull = bool(args.get("pullToLocal")) or bool(args.get("localOut"))
    remote_out = args.get("out")
    auto_remote = False
    if want_pull and not remote_out:
        remote_out = f"/tmp/strata-mcp-cap-{os.getpid()}-{int(time.time() * 1000)}.{ext}"
        auto_remote = True

    cmd = ["strata-debug", "visual", "--format", fmt, "--quality", str(int(args.get("quality", 75)))]
    opt_flag(cmd, "--max-width", int(args["maxWidth"]) if args.get("maxWidth") is not None else None)
    opt_flag(cmd, "--roi", args.get("roi"))
    opt_flag(cmd, "--out", remote_out)
    opt_flag(cmd, "--golden", args.get("golden"))
    if args.get("burstSheet"):
        cmd.append("--burst-sheet")

    res = run_argv(cmd, 30.0)
    if want_pull and res.get("rc") == 0 and remote_out:
        saved = res["json"].get("saved") if isinstance(res.get("json"), dict) else None
        actual_remote = str(saved.get("path")) if isinstance(saved, dict) and saved.get("path") else str(remote_out)
        actual_fmt = str(saved.get("format") if isinstance(saved, dict) and saved.get("format") else fmt).lower()
        actual_ext = "png" if actual_fmt == "png" else ("jpg" if actual_fmt in {"jpg", "jpeg"} else "webp")
        local_out = str(args.get("localOut") or "").strip()
        if not local_out:
            cap_dir = os.path.join(tempfile.gettempdir(), "strata-mcp-captures")
            local_out = os.path.join(cap_dir, f"cap-{int(time.time() * 1000)}.{actual_ext}")
        elif actual_ext == "png" and os.path.splitext(local_out)[1].lower() in {".webp", ".jpg", ".jpeg"}:
            local_out = f"{os.path.splitext(local_out)[0]}.png"
        ssh_kw = extract_ssh_overrides(args)
        try:
            pull_res = pull_fn(actual_remote, local_out, **ssh_kw) if ssh_kw else pull_fn(actual_remote, local_out)
        finally:
            if auto_remote:
                run_argv(["rm", "-f", actual_remote], 15.0)
        if pull_res.get("ok"):
            res["localPath"] = local_out
            res["pulled"] = True
        else:
            res["pulled"] = False
            res["pullError"] = pull_res.get("stderr", "pull failed")
    return res


def dispatch_core_tool(
    name: str,
    args: dict[str, Any],
    *,
    exec_fn: Callable[..., dict[str, Any]] = ssh_exec,
    pull_fn: Callable[..., dict[str, Any]] = scp_pull,
    script_dir: str = "",
    repo_root: str = "",
) -> Any:
    def run_argv(argv: list[str], timeout: float = 30.0) -> dict[str, Any]:
        return invoke_exec(exec_fn, argv_to_cmd(argv), timeout=timeout, args=args)

    if name == "observe_step":
        pkg = str(args.get("package") or "auto")
        cmd = [
            "strata-debug", "observe",
            "--settle-ms", str(int(args.get("settleMs", 350))),
            "--package", pkg,
            "--priority", str(args.get("priority", "W")),
        ]
        opt_flag(cmd, "--cursor", args.get("cursor"))
        opt_flag(cmd, "--tag", args.get("tag"))
        opt_flag(cmd, "--roi", args.get("roi"))
        if not args.get("includeCdp", True):
            cmd.append("--no-cdp")
        if args.get("noVisual"):
            cmd.append("--no-visual")
        elif args.get("includeVisual"):
            cmd.append("--visual")
        if args.get("actionRpc"):
            cmd.extend(["--action-rpc", str(args["actionRpc"][0]), str(args["actionRpc"][1])])
        opt_flag(cmd, "--action-shell", args.get("actionShell"))
        opt_flag(cmd, "--action-cdp-eval", args.get("actionCdpEval"))
        if args.get("actionGesture"):
            cmd.extend(["--action-gesture", json.dumps(args["actionGesture"])])
        return run_argv(cmd)

    if name == "cdp_list_targets":
        cmd = ["strata-debug", "cdp", "list-targets"]
        opt_flag(cmd, "--target", args.get("target"))
        opt_flag(cmd, "--pid", int(args["pid"]) if args.get("pid") is not None else None)
        return run_argv(cmd)

    if name in {"cdp_eval", "cdp_console", "cdp_audit"}:
        sub = name.split("_", 1)[1]
        cmd = ["strata-debug", "cdp", sub]
        if sub == "eval":
            cmd.append(str(args["expression"]))
        opt_flag(cmd, "--target", args.get("target"))
        opt_flag(cmd, "--pid", int(args["pid"]) if args.get("pid") is not None else None)
        if sub == "audit":
            opt_flag(cmd, "--url-contains", args.get("urlContains") or args.get("urlMatch"))
            opt_flag(cmd, "--eval-js", args.get("evalJs"))
            return run_argv(cmd)
        t_sec = float(args.get("timeoutSec", 5.0))
        opt_flag(cmd, "--url-match", args.get("urlMatch"))
        cmd.extend(["--timeout", str(t_sec)])
        if sub == "console":
            dur = float(args.get("durationSec", 2.0))
            cmd.extend(["--duration", str(dur)])
            return run_argv(cmd, timeout=dur + 12.0)
        return run_argv(cmd, timeout=t_sec + 10.0)

    if name == "logcat_cursor":
        cmd = ["strata-debug", "logcat"]
        if args.get("mark"):
            cmd.append("--mark")
        opt_flag(cmd, "--cursor", args.get("cursor"))
        if not args.get("mark"):
            cmd.extend(["--priority", str(args.get("priority", "W")), "--limit", str(int(args.get("limit", 100)))])
            opt_flag(cmd, "--tag", args.get("tag"))
            opt_flag(cmd, "--pid", int(args["pid"]) if args.get("pid") is not None else None)
            if args.get("noAdvance"):
                cmd.append("--no-advance")
        return run_argv(cmd)

    if name == "visual_check":
        return _handle_visual_check(args, run_argv, pull_fn)

    if name == "gesture_replay":
        kind = str(args.get("kind", "edge-swipe"))
        cmd = ["strata-debug", "gesture", kind]
        if kind == "edge-swipe":
            cmd.extend([
                "--side", str(args.get("side", "right")),
                "--y-percent", str(float(args.get("yPercent", 50.0))),
                "--distance-dp", str(float(args.get("distanceDp", 96.0))),
                "--duration-ms", str(int(args.get("durationMs", 240))),
            ])
            if args.get("cancelBack"):
                cmd.append("--cancel-back")
        elif kind in {"tap", "long-press"}:
            cmd.extend([str(int(args["x"])), str(int(args["y"]))])
            if kind == "long-press":
                cmd.extend(["--duration-ms", str(int(args.get("durationMs", 600)))])
        elif kind == "swipe":
            cmd.extend([
                str(int(args["x0"])), str(int(args["y0"])),
                str(int(args["x1"])), str(int(args["y1"])),
                "--duration-ms", str(int(args.get("durationMs", 240))),
            ])
        return run_argv(cmd)

    if name == "kernel_diag":
        mode = str(args.get("mode", "pstore"))
        cmd = ["strata-debug", "kernel", mode]
        if mode in {"pstore", "tracing"}:
            opt_flag(cmd, "--dir", args.get("dir"))
        if mode in {"dmesg", "tracing"} and args.get("limit") is not None:
            cmd.extend(["--limit", str(int(args["limit"]))])
        if mode == "dmesg":
            opt_flag(cmd, "--filter", args.get("filter"))
        return run_argv(cmd)

    if name == "kernel_probe":
        dur = max(1, min(30, int(args.get("durationSec", 5))))
        return run_argv(["strata-debug", "kernel", "trace", str(args["preset"]), "--duration", str(dur)], timeout=dur + 15.0)

    if name == "pstore_dump":
        return run_argv(["strata-debug", "kernel", "pstore"])

    if name == "device_health_snapshot":
        pkg = str(args.get("package") or "auto")
        return {
            "daemon": run_argv(["host-exec", "/data/adb/ksu/bin/strata", "--json", "daemon", "status"]),
            "containers": run_argv(["host-exec", "/data/adb/ksu/bin/strata", "--json", "list"]),
            "sysProbe": run_argv(["strata-debug", "sys-probe", "--package", pkg]),
        }

    if name == "reload_and_verify":
        ssh_kw = extract_ssh_overrides(args)
        host, port, user = resolve_ssh_target(**ssh_kw)
        target = f"{user}@{host}" if user else host
        port_args = ["--ssh-port", port] if port else []
        r_cmd = ["bash", os.path.join(repo_root, "reload.sh"), "--ssh", target, *port_args] + (["--skip-build"] if args.get("skipBuild", True) else [])
        r1 = subprocess.run(r_cmd, capture_output=True, text=True, timeout=120.0, check=False)
        if r1.returncode != 0:
            return {"ok": False, "stage": "reload", "rc": r1.returncode, "stdout": r1.stdout[-1500:], "stderr": r1.stderr[-1500:]}
        v_cmd = [sys.executable, os.path.join(script_dir, "verify-live.py"), "--mode", "ssh", "--ssh", target, *port_args, "--non-destructive"] + (["--ui-only"] if args.get("uiOnly") else [])
        r2 = subprocess.run(v_cmd, capture_output=True, text=True, timeout=120.0, check=False)
        return {"ok": r2.returncode == 0, "stage": "verify", "rc": r2.returncode, "stdout": r2.stdout[-2000:], "stderr": r2.stderr[-1000:]}

    if name == "forward_cdp":
        port_num = int(args.get("localPort", 9222))
        sockets = (run_argv(["strata-debug", "cdp", "list-targets"]).get("json") or {}).get("sockets", [])
        pid, target = args.get("pid"), args.get("target")
        chosen = next((s for s in sockets if pid is not None and s.get("pid") == pid), None)
        if chosen is None and target:
            chosen = next((s for s in sockets if str(target) in s.get("process", "")), None)
        chosen = chosen or (sockets[0] if sockets else None)
        if not chosen:
            return {"ok": False, "error": "no devtools socket found on device"}
        fwd = run_argv(["strata-debug", "forward", f"tcp:{port_num}", f"localabstract:{chosen['socket']}"])
        tunnel = open_ssh_port_forward(port_num, port_num, **extract_ssh_overrides(args))
        return {
            "ok": fwd["rc"] == 0 and bool(tunnel.get("ok")),
            "localPort": port_num,
            "socket": chosen["socket"],
            "route": tunnel.get("route"),
            "stderr": tunnel.get("stderr", ""),
        }

    raise KeyError(f"unknown core tool '{name}'")
