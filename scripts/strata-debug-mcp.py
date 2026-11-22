#!/usr/bin/env python3
"""Thin PC-side MCP server (JSON-RPC 2.0 over stdio) bridging AI agents to Strata over SSH.

Uses persistent OpenSSH ControlMaster connections to Strata's Dropbear SSH daemon so every
debug tool call executes with sub-50ms overhead over SSH + host-exec.

Environment variables:
  STRATA_SSH_HOST  Target device IP or hostname (required for remote execution)
  STRATA_SSH_PORT  Dropbear SSH port (default: 2222)
  STRATA_SSH_USER  SSH username (default: root)
"""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
from typing import Any

PROTOCOL_VERSION = "2024-11-05"
SERVER_INFO = {"name": "strata-debug-mcp", "version": "0.1.0"}
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(SCRIPT_DIR)

def _tool(name: str, desc: str, props: dict[str, Any], required: list[str] | None = None) -> dict[str, Any]:
    schema: dict[str, Any] = {"type": "object", "properties": props}
    if required:
        schema["required"] = required
    return {"name": name, "description": desc, "inputSchema": schema}


TOOLS: list[dict[str, Any]] = [
    _tool(
        "observe_step",
        "Atomic L4 observe: mark logcat -> action -> settle -> collect logcat + sys-probe + CDP + optional visual ROI.",
        {
            "actionRpc": {"type": "array", "items": {"type": "string"}, "minItems": 2, "maxItems": 2},
            "actionShell": {"type": "string"},
            "actionCdpEval": {"type": "string"},
            "actionGesture": {"type": "object"},
            "settleMs": {"type": "integer", "default": 350},
            "package": {"type": "string", "default": "pochita.sideveil.host"},
            "priority": {"type": "string", "default": "W"},
            "tag": {"type": "string"},
            "includeCdp": {"type": "boolean", "default": True},
            "includeVisual": {"type": "boolean", "default": False},
            "roi": {"type": "string"},
        },
    ),
    _tool(
        "cdp_audit",
        "L1 WebView DOM health audit (overflow, overlap, offscreen, CSS tokens, contrast, console errors) via abstract UDS.",
        {"target": {"type": "string"}, "pid": {"type": "integer"}, "urlContains": {"type": "string"}, "evalJs": {"type": "string"}},
    ),
    _tool(
        "gesture_replay",
        "L2b /dev/uinput multitouch Protocol B virtual touchscreen gesture replay.",
        {
            "kind": {"type": "string", "enum": ["edge-swipe", "tap", "long-press", "swipe"], "default": "edge-swipe"},
            "side": {"type": "string", "enum": ["left", "right"], "default": "right"},
            "yPercent": {"type": "number", "default": 50.0},
            "distanceDp": {"type": "number", "default": 96.0},
            "durationMs": {"type": "integer", "default": 240},
            "cancelBack": {"type": "boolean", "default": False},
            "x": {"type": "integer"}, "y": {"type": "integer"},
            "x0": {"type": "integer"}, "y0": {"type": "integer"},
            "x1": {"type": "integer"}, "y1": {"type": "integer"},
        },
    ),
    _tool(
        "kernel_probe",
        "L5 read-only bpftrace/tracefs kernel probe preset (guard-audit, net-uid, sched-lat).",
        {"preset": {"type": "string", "enum": ["guard-audit", "net-uid", "sched-lat"]}, "durationSec": {"type": "integer", "default": 5}},
        required=["preset"],
    ),
    _tool("pstore_dump", "L5 read-only /sys/fs/pstore ramoops dump and OEM watchdog/panic parser.", {}),
    _tool("device_health_snapshot", "Strata daemon status, container list, and Android window/gfxinfo/SideVeil probe.", {"package": {"type": "string"}}),
    _tool("reload_and_verify", "SSH hot-reload (reload.sh --ssh) + non-destructive live verification.", {"skipBuild": {"type": "boolean"}, "uiOnly": {"type": "boolean"}}),
    _tool("forward_cdp", "Forward a device WebView DevTools abstract socket to PC localhost via strata-debug forward + SSH tunnel.", {"localPort": {"type": "integer"}, "target": {"type": "string"}, "pid": {"type": "integer"}}),
]


def _ssh_config() -> tuple[str, str, str]:
    host = os.environ.get("STRATA_SSH_HOST", "").strip()
    port = os.environ.get("STRATA_SSH_PORT", "2222").strip() or "2222"
    user = os.environ.get("STRATA_SSH_USER", "root").strip() or "root"
    if not host:
        raise RuntimeError("STRATA_SSH_HOST environment variable is not set")
    return host, port, user


def _ssh_base_args(host: str, port: str, user: str) -> list[str]:
    return [
        "ssh", "-p", port, "-o", "BatchMode=yes", "-o", "ConnectTimeout=8",
        "-o", "StrictHostKeyChecking=accept-new", "-o", "ControlMaster=auto",
        "-o", "ControlPersist=300", "-o", "ControlPath=/tmp/strata-mcp-%r@%h:%p",
        f"{user}@{host}",
    ]


def ssh_exec(remote_cmd: str, timeout: float = 30.0) -> dict[str, Any]:
    host, port, user = _ssh_config()
    res = subprocess.run([*_ssh_base_args(host, port, user), remote_cmd], capture_output=True, text=True, timeout=timeout, check=False)
    stdout = res.stdout.strip()
    try:
        parsed = json.loads(stdout) if stdout else None
    except json.JSONDecodeError:
        parsed = None
    return {"rc": res.returncode, "json": parsed, "stdout": stdout, "stderr": res.stderr.strip()}


def _call_tool(name: str, args: dict[str, Any]) -> Any:
    if name == "observe_step":
        cmd = ["strata-debug", "observe", "--settle-ms", str(args.get("settleMs", 350))]
        cmd += ["--package", str(args.get("package", "pochita.sideveil.host"))]
        cmd += ["--priority", str(args.get("priority", "W"))]
        if args.get("tag"):
            cmd += ["--tag", str(args["tag"])]
        if not args.get("includeCdp", True):
            cmd.append("--no-cdp")
        if args.get("includeVisual"):
            cmd.append("--visual")
        if args.get("roi"):
            cmd += ["--roi", str(args["roi"])]
        if args.get("actionRpc"):
            cmd += ["--action-rpc", str(args["actionRpc"][0]), str(args["actionRpc"][1])]
        if args.get("actionShell"):
            cmd += ["--action-shell", str(args["actionShell"])]
        if args.get("actionCdpEval"):
            cmd += ["--action-cdp-eval", str(args["actionCdpEval"])]
        if args.get("actionGesture"):
            cmd += ["--action-gesture", json.dumps(args["actionGesture"])]
        return ssh_exec(" ".join(shlex.quote(c) for c in cmd))

    if name == "cdp_audit":
        cmd = ["strata-debug", "cdp", "audit", "--target", str(args.get("target", "pochita.sideveil.host"))]
        if args.get("pid") is not None:
            cmd += ["--pid", str(args["pid"])]
        if args.get("urlContains"):
            cmd += ["--url-contains", str(args["urlContains"])]
        if args.get("evalJs"):
            cmd += ["--eval-js", str(args["evalJs"])]
        return ssh_exec(" ".join(shlex.quote(c) for c in cmd))

    if name == "gesture_replay":
        kind = str(args.get("kind", "edge-swipe"))
        cmd = ["strata-debug", "gesture", kind]
        if kind == "edge-swipe":
            cmd += [
                "--side", str(args.get("side", "right")),
                "--y-percent", str(args.get("yPercent", 50.0)),
                "--distance-dp", str(args.get("distanceDp", 96.0)),
                "--duration-ms", str(args.get("durationMs", 240)),
            ]
            if args.get("cancelBack"):
                cmd.append("--cancel-back")
        elif kind in {"tap", "long-press"}:
            cmd += [str(args["x"]), str(args["y"])]
            if kind == "long-press":
                cmd += ["--duration-ms", str(args.get("durationMs", 600))]
        elif kind == "swipe":
            cmd += [
                str(args["x0"]), str(args["y0"]), str(args["x1"]), str(args["y1"]),
                "--duration-ms", str(args.get("durationMs", 240)),
            ]
        return ssh_exec(" ".join(shlex.quote(c) for c in cmd))

    if name == "kernel_probe":
        preset = str(args["preset"])
        dur = int(args.get("durationSec", 5))
        return ssh_exec(f"strata-debug kernel trace {shlex.quote(preset)} --duration {dur}", timeout=dur + 15)

    if name == "pstore_dump":
        return ssh_exec("strata-debug kernel pstore")

    if name == "device_health_snapshot":
        pkg = shlex.quote(str(args.get("package", "pochita.sideveil.host")))
        return {
            "daemon": ssh_exec("host-exec /data/adb/ksu/bin/strata --json daemon status"),
            "containers": ssh_exec("host-exec /data/adb/ksu/bin/strata --json list"),
            "sysProbe": ssh_exec(f"strata-debug sys-probe --package {pkg}"),
        }

    if name == "reload_and_verify":
        host, port, user = _ssh_config()
        target = f"{user}@{host}"
        reload_cmd = ["bash", os.path.join(REPO_ROOT, "reload.sh"), "--ssh", target, "--ssh-port", port]
        if args.get("skipBuild", True):
            reload_cmd.append("--skip-build")
        r1 = subprocess.run(reload_cmd, capture_output=True, text=True, check=False)
        if r1.returncode != 0:
            return {"ok": False, "stage": "reload", "rc": r1.returncode, "stdout": r1.stdout[-1500:], "stderr": r1.stderr[-1500:]}
        ver_cmd = [
            sys.executable, os.path.join(SCRIPT_DIR, "verify-live.py"),
            "--mode", "ssh", "--ssh", target, "--ssh-port", port, "--non-destructive",
        ]
        if args.get("uiOnly"):
            ver_cmd.append("--ui-only")
        r2 = subprocess.run(ver_cmd, capture_output=True, text=True, check=False)
        return {"ok": r2.returncode == 0, "stage": "verify", "rc": r2.returncode, "stdout": r2.stdout[-2000:], "stderr": r2.stderr[-1000:]}

    if name == "forward_cdp":
        port_num = int(args.get("localPort", 9222))
        targets = ssh_exec("strata-debug cdp list-targets")
        sockets = (targets.get("json") or {}).get("sockets", [])
        pid = args.get("pid")
        target = str(args.get("target", "pochita.sideveil.host"))
        chosen = next((s for s in sockets if pid is not None and s.get("pid") == pid), None)
        if not chosen:
            chosen = next((s for s in sockets if target in s.get("process", "")), None) or (sockets[0] if sockets else None)
        if not chosen:
            return {"ok": False, "error": "no devtools socket found on device"}
        fwd = ssh_exec(f"strata-debug forward tcp:{port_num} localabstract:{shlex.quote(chosen['socket'])}")
        host, ssh_port, user = _ssh_config()
        tunnel = subprocess.run(
            ["ssh", "-p", ssh_port, "-f", "-N", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes",
             "-L", f"127.0.0.1:{port_num}:127.0.0.1:{port_num}", f"{user}@{host}"],
            capture_output=True, text=True, check=False,
        )
        return {"ok": fwd["rc"] == 0 and tunnel.returncode == 0, "localPort": port_num, "socket": chosen["socket"], "stderr": tunnel.stderr.strip()}

    raise KeyError(f"unknown tool '{name}'")


def handle_rpc_request(req: dict[str, Any]) -> dict[str, Any] | None:
    method = req.get("method")
    req_id = req.get("id")
    params = req.get("params") or {}

    if method == "notifications/initialized":
        return None
    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": SERVER_INFO,
            },
        }
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": req_id, "result": {"tools": TOOLS}}
    if method == "tools/call":
        try:
            payload = _call_tool(str(params.get("name", "")), params.get("arguments") or {})
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {"content": [{"type": "text", "text": json.dumps(payload, ensure_ascii=False)}]},
            }
        except Exception as exc:
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {"isError": True, "content": [{"type": "text", "text": str(exc)}]},
            }
    if req_id is None:
        return None
    return {"jsonrpc": "2.0", "id": req_id, "error": {"code": -32601, "message": f"Method not found: {method}"}}


def main() -> int:
    for line in sys.stdin:
        raw = line.strip()
        if not raw:
            continue
        try:
            req = json.loads(raw)
        except json.JSONDecodeError as exc:
            resp = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": str(exc)}}
        else:
            resp = handle_rpc_request(req)
        if resp is not None:
            sys.stdout.write(json.dumps(resp, ensure_ascii=False) + "\n")
            sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
