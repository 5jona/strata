#!/usr/bin/env python3
"""Thin PC-side MCP server (JSON-RPC 2.0 over stdio) bridging AI agents to Strata over SSH.

Env: STRATA_SSH_HOST (required), STRATA_SSH_PORT (default 2222), STRATA_SSH_USER (default root).
"""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
import tempfile
from typing import Any

PROTOCOL_VERSION, MAX_TIMEOUT_SEC = "2024-11-05", 120.0
SERVER_INFO = {"name": "strata-debug-mcp", "version": "0.2.0"}
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(SCRIPT_DIR)
_S, _I, _N, _B = {"type": "string"}, {"type": "integer"}, {"type": "number"}, {"type": "boolean"}


def _tool(name: str, desc: str, props: dict[str, Any], req: list[str] | None = None) -> dict[str, Any]:
    schema: dict[str, Any] = {"type": "object", "properties": props, "additionalProperties": False}
    if req:
        schema["required"] = req
    return {"name": name, "description": desc, "inputSchema": schema}


TOOLS: list[dict[str, Any]] = [
    _tool("observe_step", "Atomic L4 observe: mark/cursor -> action -> settle -> logcat + sys-probe + CDP + visual.", {
        "actionRpc": {"type": "array", "items": _S, "minItems": 2, "maxItems": 2},
        "actionShell": _S, "actionCdpEval": _S, "actionGesture": {"type": "object"}, "cursor": _S,
        "settleMs": _I, "package": _S, "priority": _S, "tag": _S,
        "includeCdp": _B, "includeVisual": _B, "noVisual": _B, "roi": _S,
    }),
    _tool("cdp_list_targets", "List discovered WebView/Chrome DevTools abstract sockets and /json/list targets.", {"target": _S, "pid": _I}),
    _tool("cdp_eval", "Evaluate JavaScript in a WebView target over abstract UDS CDP WebSocket.", {
        "expression": _S, "target": _S, "pid": _I, "urlMatch": _S, "timeoutSec": _N,
    }, req=["expression"]),
    _tool("cdp_console", "Capture WebView console events and uncaught exceptions over CDP.", {
        "target": _S, "pid": _I, "urlMatch": _S, "durationSec": _N, "timeoutSec": _N,
    }),
    _tool("cdp_audit", "L1 WebView DOM health audit (overflow, overlap, offscreen, contrast, console errors).", {
        "target": _S, "pid": _I, "urlContains": _S, "urlMatch": _S, "evalJs": _S,
    }),
    _tool("logcat_cursor", "L2a incremental logcat cursor: --mark watermark or read new lines with crash highlights.", {
        "mark": _B, "cursor": _S, "priority": _S, "tag": _S, "pid": _I, "limit": _I, "noAdvance": _B,
    }),
    _tool("visual_check", "L3 raw screencap health check (black/blank/solid-color/blur/SSIM) with optional save.", {
        "roi": _S, "out": _S, "format": _S, "quality": _I, "golden": _S, "burstSheet": _B,
    }),
    _tool("gesture_replay", "L2b /dev/uinput multitouch Protocol B virtual touchscreen gesture replay.", {
        "kind": {"type": "string", "enum": ["edge-swipe", "tap", "long-press", "swipe"]},
        "side": {"type": "string", "enum": ["left", "right"]}, "yPercent": _N, "distanceDp": _N,
        "durationMs": _I, "cancelBack": _B, "x": _I, "y": _I, "x0": _I, "y0": _I, "x1": _I, "y1": _I,
    }),
    _tool("kernel_diag", "L5 read-only kernel diagnostics: pstore ramoops, dmesg ring buffer, or tracefs status.", {
        "mode": {"type": "string", "enum": ["pstore", "dmesg", "tracing"]}, "limit": _I, "filter": _S, "dir": _S,
    }),
    _tool("kernel_probe", "L5 read-only bpftrace/tracefs kernel probe preset (guard-audit, net-uid, sched-lat).", {
        "preset": {"type": "string", "enum": ["guard-audit", "net-uid", "sched-lat"]}, "durationSec": _I,
    }, req=["preset"]),
    _tool("pstore_dump", "L5 read-only /sys/fs/pstore ramoops dump and OEM watchdog/panic parser.", {}),
    _tool("device_health_snapshot", "Strata daemon status, container list, and Android window/gfxinfo/SideVeil probe.", {"package": _S}),
    _tool("reload_and_verify", "SSH hot-reload (reload.sh --ssh) + non-destructive live verification.", {"skipBuild": _B, "uiOnly": _B}),
    _tool("forward_cdp", "Forward a device WebView DevTools abstract socket to PC localhost via strata-debug + SSH.", {
        "localPort": _I, "target": _S, "pid": _I,
    }),
]


def _ssh_config() -> tuple[str, str, str]:
    host = os.environ.get("STRATA_SSH_HOST", "").strip()
    port = os.environ.get("STRATA_SSH_PORT", "2222").strip() or "2222"
    user = os.environ.get("STRATA_SSH_USER", "root").strip() or "root"
    if not host:
        raise RuntimeError("STRATA_SSH_HOST environment variable is not set")
    return host, port, user


def _control_path() -> str:
    xdg = os.environ.get("XDG_RUNTIME_DIR", "").strip()
    ctl_dir = os.path.join(xdg, "strata-mcp") if xdg else os.path.join(tempfile.gettempdir(), f"strata-mcp-{os.getuid()}")
    os.makedirs(ctl_dir, mode=0o700, exist_ok=True)
    os.chmod(ctl_dir, 0o700)
    return os.path.join(ctl_dir, "cm-%r@%h:%p")


def _ssh_base_args(host: str, port: str, user: str) -> list[str]:
    return [
        "ssh", "-p", port, "-o", "BatchMode=yes", "-o", "ConnectTimeout=8",
        "-o", "StrictHostKeyChecking=accept-new", "-o", "ControlMaster=auto",
        "-o", "ControlPersist=300", "-o", f"ControlPath={_control_path()}", f"{user}@{host}",
    ]


def ssh_exec(remote_cmd: str, timeout: float = 30.0) -> dict[str, Any]:
    host, port, user = _ssh_config()
    eff = max(1.0, min(MAX_TIMEOUT_SEC, float(timeout)))
    try:
        res = subprocess.run([*_ssh_base_args(host, port, user), remote_cmd], capture_output=True, text=True, timeout=eff, check=False)
    except subprocess.TimeoutExpired:
        return {"ok": False, "rc": 124, "json": None, "stdout": "", "stderr": f"SSH command timed out after {eff}s"}
    stdout = res.stdout.strip()
    try:
        parsed = json.loads(stdout) if stdout else None
    except json.JSONDecodeError:
        parsed = None
    return {"rc": res.returncode, "json": parsed, "stdout": stdout, "stderr": res.stderr.strip()}


def _ssh_argv(argv: list[str], timeout: float = 30.0) -> dict[str, Any]:
    return ssh_exec(" ".join(shlex.quote(c) for c in argv), timeout=timeout)


def _opt(cmd: list[str], flag: str, val: Any) -> None:
    if val is not None and val != "":
        cmd.extend([flag, str(val)])


def _call_tool(name: str, args: dict[str, Any]) -> Any:
    if name == "observe_step":
        cmd = ["strata-debug", "observe", "--settle-ms", str(int(args.get("settleMs", 350))),
               "--package", str(args.get("package", "pochita.sideveil.host")), "--priority", str(args.get("priority", "W"))]
        _opt(cmd, "--cursor", args.get("cursor"))
        _opt(cmd, "--tag", args.get("tag"))
        _opt(cmd, "--roi", args.get("roi"))
        if not args.get("includeCdp", True):
            cmd.append("--no-cdp")
        if args.get("noVisual"):
            cmd.append("--no-visual")
        elif args.get("includeVisual"):
            cmd.append("--visual")
        if args.get("actionRpc"):
            cmd += ["--action-rpc", str(args["actionRpc"][0]), str(args["actionRpc"][1])]
        _opt(cmd, "--action-shell", args.get("actionShell"))
        _opt(cmd, "--action-cdp-eval", args.get("actionCdpEval"))
        if args.get("actionGesture"):
            cmd += ["--action-gesture", json.dumps(args["actionGesture"])]
        return _ssh_argv(cmd)

    if name == "cdp_list_targets":
        cmd = ["strata-debug", "cdp", "list-targets"]
        _opt(cmd, "--target", args.get("target"))
        _opt(cmd, "--pid", int(args["pid"]) if args.get("pid") is not None else None)
        return _ssh_argv(cmd)

    if name in {"cdp_eval", "cdp_console", "cdp_audit"}:
        sub = name.split("_", 1)[1]
        cmd = ["strata-debug", "cdp", sub]
        if sub == "eval":
            cmd.append(str(args["expression"]))
        cmd += ["--target", str(args.get("target", "pochita.sideveil.host"))]
        _opt(cmd, "--pid", int(args["pid"]) if args.get("pid") is not None else None)
        if sub == "audit":
            _opt(cmd, "--url-contains", args.get("urlContains") or args.get("urlMatch"))
            _opt(cmd, "--eval-js", args.get("evalJs"))
            return _ssh_argv(cmd)
        t_sec = float(args.get("timeoutSec", 5.0))
        _opt(cmd, "--url-match", args.get("urlMatch"))
        cmd += ["--timeout", str(t_sec)]
        if sub == "console":
            dur = float(args.get("durationSec", 2.0))
            cmd += ["--duration", str(dur)]
            return _ssh_argv(cmd, timeout=dur + 12.0)
        return _ssh_argv(cmd, timeout=t_sec + 10.0)

    if name == "logcat_cursor":
        cmd = ["strata-debug", "logcat"]
        if args.get("mark"):
            cmd.append("--mark")
        _opt(cmd, "--cursor", args.get("cursor"))
        if not args.get("mark"):
            cmd += ["--priority", str(args.get("priority", "W")), "--limit", str(int(args.get("limit", 100)))]
            _opt(cmd, "--tag", args.get("tag"))
            _opt(cmd, "--pid", int(args["pid"]) if args.get("pid") is not None else None)
            if args.get("noAdvance"):
                cmd.append("--no-advance")
        return _ssh_argv(cmd)

    if name == "visual_check":
        cmd = ["strata-debug", "visual", "--format", str(args.get("format", "webp")), "--quality", str(int(args.get("quality", 75)))]
        _opt(cmd, "--roi", args.get("roi"))
        _opt(cmd, "--out", args.get("out"))
        _opt(cmd, "--golden", args.get("golden"))
        if args.get("burstSheet"):
            cmd.append("--burst-sheet")
        return _ssh_argv(cmd)

    if name == "gesture_replay":
        kind = str(args.get("kind", "edge-swipe"))
        cmd = ["strata-debug", "gesture", kind]
        if kind == "edge-swipe":
            cmd += ["--side", str(args.get("side", "right")), "--y-percent", str(float(args.get("yPercent", 50.0))),
                    "--distance-dp", str(float(args.get("distanceDp", 96.0))), "--duration-ms", str(int(args.get("durationMs", 240)))]
            if args.get("cancelBack"):
                cmd.append("--cancel-back")
        elif kind in {"tap", "long-press"}:
            cmd += [str(int(args["x"])), str(int(args["y"]))]
            if kind == "long-press":
                cmd += ["--duration-ms", str(int(args.get("durationMs", 600)))]
        elif kind == "swipe":
            cmd += [str(int(args["x0"])), str(int(args["y0"])), str(int(args["x1"])), str(int(args["y1"])), "--duration-ms", str(int(args.get("durationMs", 240)))]
        return _ssh_argv(cmd)

    if name == "kernel_diag":
        mode = str(args.get("mode", "pstore"))
        cmd = ["strata-debug", "kernel", mode]
        if mode in {"pstore", "tracing"}:
            _opt(cmd, "--dir", args.get("dir"))
        if mode in {"dmesg", "tracing"} and args.get("limit") is not None:
            cmd += ["--limit", str(int(args["limit"]))]
        if mode == "dmesg":
            _opt(cmd, "--filter", args.get("filter"))
        return _ssh_argv(cmd)
    if name == "kernel_probe":
        dur = max(1, min(30, int(args.get("durationSec", 5))))
        return _ssh_argv(["strata-debug", "kernel", "trace", str(args["preset"]), "--duration", str(dur)], timeout=dur + 15.0)
    if name == "pstore_dump":
        return _ssh_argv(["strata-debug", "kernel", "pstore"])
    if name == "device_health_snapshot":
        pkg = str(args.get("package", "pochita.sideveil.host"))
        return {
            "daemon": _ssh_argv(["host-exec", "/data/adb/ksu/bin/strata", "--json", "daemon", "status"]),
            "containers": _ssh_argv(["host-exec", "/data/adb/ksu/bin/strata", "--json", "list"]),
            "sysProbe": _ssh_argv(["strata-debug", "sys-probe", "--package", pkg]),
        }
    if name == "reload_and_verify":
        host, port, user = _ssh_config()
        target = f"{user}@{host}"
        r_cmd = ["bash", os.path.join(REPO_ROOT, "reload.sh"), "--ssh", target, "--ssh-port", port] + (["--skip-build"] if args.get("skipBuild", True) else [])
        r1 = subprocess.run(r_cmd, capture_output=True, text=True, timeout=120.0, check=False)
        if r1.returncode != 0:
            return {"ok": False, "stage": "reload", "rc": r1.returncode, "stdout": r1.stdout[-1500:], "stderr": r1.stderr[-1500:]}
        v_cmd = [sys.executable, os.path.join(SCRIPT_DIR, "verify-live.py"), "--mode", "ssh", "--ssh", target, "--ssh-port", port, "--non-destructive"] + (["--ui-only"] if args.get("uiOnly") else [])
        r2 = subprocess.run(v_cmd, capture_output=True, text=True, timeout=120.0, check=False)
        return {"ok": r2.returncode == 0, "stage": "verify", "rc": r2.returncode, "stdout": r2.stdout[-2000:], "stderr": r2.stderr[-1000:]}
    if name == "forward_cdp":
        port_num = int(args.get("localPort", 9222))
        sockets = (_ssh_argv(["strata-debug", "cdp", "list-targets"]).get("json") or {}).get("sockets", [])
        pid, target = args.get("pid"), str(args.get("target", "pochita.sideveil.host"))
        chosen = next((s for s in sockets if pid is not None and s.get("pid") == pid), None)
        chosen = chosen or next((s for s in sockets if target in s.get("process", "")), None) or (sockets[0] if sockets else None)
        if not chosen:
            return {"ok": False, "error": "no devtools socket found on device"}
        fwd = _ssh_argv(["strata-debug", "forward", f"tcp:{port_num}", f"localabstract:{chosen['socket']}"])
        host, ssh_port, user = _ssh_config()
        tunnel = subprocess.run(
            ["ssh", "-p", ssh_port, "-f", "-N", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes",
             "-o", f"ControlPath={_control_path()}", "-L", f"127.0.0.1:{port_num}:127.0.0.1:{port_num}", f"{user}@{host}"],
            capture_output=True, text=True, timeout=15.0, check=False,
        )
        return {"ok": fwd["rc"] == 0 and tunnel.returncode == 0, "localPort": port_num, "socket": chosen["socket"], "stderr": tunnel.stderr.strip()}
    raise KeyError(f"unknown tool '{name}'")


def handle_rpc_request(req: Any) -> dict[str, Any] | None:
    if not isinstance(req, dict):
        return {"jsonrpc": "2.0", "id": None, "error": {"code": -32600, "message": "Invalid Request"}}
    method, req_id, params = req.get("method"), req.get("id"), req.get("params") or {}
    if method == "notifications/initialized":
        return None
    if method == "ping":
        return {"jsonrpc": "2.0", "id": req_id, "result": {}}
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": req_id, "result": {"protocolVersion": PROTOCOL_VERSION, "capabilities": {"tools": {}}, "serverInfo": SERVER_INFO}}
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": req_id, "result": {"tools": TOOLS}}
    if method == "tools/call":
        try:
            payload = _call_tool(str(params.get("name", "")), params.get("arguments") or {})
            return {"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": json.dumps(payload, ensure_ascii=False)}]}}
        except Exception as exc:
            sys.stderr.write(f"[strata-debug-mcp] tool error ({params.get('name')}): {exc}\n")
            return {"jsonrpc": "2.0", "id": req_id, "result": {"isError": True, "content": [{"type": "text", "text": str(exc)}]}}
    return None if req_id is None else {"jsonrpc": "2.0", "id": req_id, "error": {"code": -32601, "message": f"Method not found: {method}"}}


def main() -> int:
    for line in sys.stdin:
        raw = line.strip()
        if not raw:
            continue
        try:
            resp = handle_rpc_request(json.loads(raw))
        except json.JSONDecodeError as exc:
            resp = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": str(exc)}}
        if resp is not None:
            sys.stdout.write(json.dumps(resp, ensure_ascii=False) + "\n")
            sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
