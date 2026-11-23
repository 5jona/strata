#!/usr/bin/env python3
"""Thin PC-side MCP server (JSON-RPC 2.0 over stdio) bridging AI agents to Strata over SSH.

Env: STRATA_SSH_HOST (required, alias or IP), STRATA_SSH_PORT (optional), STRATA_SSH_USER (optional).
"""

from __future__ import annotations

import json
import os
import sys
from typing import Any

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(SCRIPT_DIR)
if SCRIPT_DIR not in sys.path:
    sys.path.insert(0, SCRIPT_DIR)

from strata_mcp import TOOLS, devices, dispatch_tool, transport

PROTOCOL_VERSION = "2024-11-05"
SERVER_INFO = {"name": "strata-debug-mcp", "version": "0.4.0"}
MAX_TIMEOUT_SEC = transport.MAX_TIMEOUT_SEC

_control_path = transport.control_path
_ssh_config = transport.ssh_config
build_ssh_base_args = transport.build_ssh_base_args
ssh_exec = transport.ssh_exec
scp_push = transport.scp_push
scp_pull = transport.scp_pull


def _call_tool(name: str, args: dict[str, Any]) -> Any:
    return dispatch_tool(
        name,
        args,
        exec_fn=lambda cmd, timeout=30.0, **kw: ssh_exec(cmd, timeout=timeout, **kw),
        push_fn=lambda *a, **kw: scp_push(*a, **kw),
        pull_fn=lambda *a, **kw: scp_pull(*a, **kw),
        script_dir=SCRIPT_DIR,
        repo_root=REPO_ROOT,
    )


def handle_rpc_request(req: Any) -> dict[str, Any] | None:
    if not isinstance(req, dict):
        return {"jsonrpc": "2.0", "id": None, "error": {"code": -32600, "message": "Invalid Request"}}
    method, req_id, params = req.get("method"), req.get("id"), req.get("params") or {}
    if method == "notifications/initialized":
        return None
    if method == "ping":
        return {"jsonrpc": "2.0", "id": req_id, "result": {}}
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
            sys.stderr.write(f"[strata-debug-mcp] tool error ({params.get('name')}): {exc}\n")
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
            resp = handle_rpc_request(json.loads(raw))
        except json.JSONDecodeError as exc:
            resp = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": str(exc)}}
        if resp is not None:
            sys.stdout.write(json.dumps(resp, ensure_ascii=False) + "\n")
            sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
