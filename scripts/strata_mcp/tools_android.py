"""Generic Android debugging MCP tools (10 tools: UI, App, APK, LSPosed, SQLite, KernelSU, exec, transfer)."""

from __future__ import annotations

import os
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
    opt_flag,
    scp_pull,
    scp_push,
    ssh_exec,
)

ANDROID_TOOLS: list[dict[str, Any]] = [
    make_tool("ui_dump", "Dump Android UI hierarchy via uiautomator with optional text/id/desc filters.", {
        "compressed": B, "query": S, "resourceId": S, "desc": S, "clickableOnly": B, "limit": I,
    }),
    make_tool("ui_tap", "Find a UI element by text, resourceId, or content-desc and tap its center.", {
        "text": S, "resourceId": S, "desc": S, "index": I,
        "method": {"type": "string", "enum": ["input", "uinput"]},
    }),
    make_tool("app_inspect", "Inspect top resumed activity or detailed package metadata (version, UID, permissions).", {
        "package": S, "topOnly": B,
    }),
    make_tool("app_control", "Start, force-stop, or clear data of an Android application package.", {
        "action": {"type": "string", "enum": ["start", "stop", "clear"]},
        "package": S, "activity": S, "intentAction": S, "dataUri": S,
    }, req=["action", "package"]),
    make_tool("apk_install", "Install an APK from PC (localPath) or device (remotePath) via pm install.", {
        "localPath": S, "remotePath": S, "grantPermissions": B, "allowDowngrade": B, "replace": B,
    }),
    make_tool("lsposed_inspect", "Inspect LSPosed enabled modules or read recent LSPosed log entries.", {
        "mode": {"type": "string", "enum": ["modules", "logs"]},
        "module": S, "grep": S, "limit": I,
    }),
    make_tool("sqlite_query", "List tables or run read-only SQL queries against a device SQLite database.", {
        "dbPath": S, "mode": {"type": "string", "enum": ["tables", "query"]}, "sql": S, "limit": I,
    }, req=["dbPath"]),
    make_tool("ksu_inspect", "Inspect KernelSU modules, input devices, or read-only sysfs/proc paths.", {
        "mode": {"type": "string", "enum": ["modules", "inputs", "sysfs"]}, "path": S,
    }),
    make_tool("device_exec", "Execute an arbitrary shell command on Android host (host-exec) or inside container.", {
        "command": S, "scope": {"type": "string", "enum": ["host", "container"]}, "timeoutSec": N,
    }, req=["command"]),
    make_tool("file_transfer", "Push or pull files over SSH ControlMaster (container or Android host filesystem).", {
        "direction": {"type": "string", "enum": ["push", "pull"]},
        "localPath": S, "remotePath": S, "hostScope": B,
    }, req=["direction", "localPath", "remotePath"]),
]

ANDROID_TOOL_NAMES = {t["name"] for t in ANDROID_TOOLS}


def dispatch_android_tool(
    name: str,
    args: dict[str, Any],
    *,
    exec_fn: Callable[..., dict[str, Any]] = ssh_exec,
    push_fn: Callable[..., dict[str, Any]] = scp_push,
    pull_fn: Callable[..., dict[str, Any]] = scp_pull,
) -> Any:
    def run_argv(argv: list[str], timeout: float = 30.0) -> dict[str, Any]:
        return invoke_exec(exec_fn, argv_to_cmd(argv), timeout=timeout, args=args)

    if name == "ui_dump":
        cmd = ["strata-debug", "ui", "dump"]
        if args.get("compressed"):
            cmd.append("--compressed")
        opt_flag(cmd, "--query", args.get("query"))
        opt_flag(cmd, "--resource-id", args.get("resourceId"))
        opt_flag(cmd, "--desc", args.get("desc"))
        if args.get("clickableOnly"):
            cmd.append("--clickable-only")
        opt_flag(cmd, "--limit", int(args["limit"]) if args.get("limit") is not None else None)
        return run_argv(cmd)

    if name == "ui_tap":
        cmd = ["strata-debug", "ui", "tap"]
        opt_flag(cmd, "--text", args.get("text"))
        opt_flag(cmd, "--resource-id", args.get("resourceId"))
        opt_flag(cmd, "--desc", args.get("desc"))
        opt_flag(cmd, "--index", int(args["index"]) if args.get("index") is not None else None)
        opt_flag(cmd, "--method", args.get("method"))
        return run_argv(cmd)

    if name == "app_inspect":
        pkg = str(args.get("package") or "").strip()
        if args.get("topOnly") or pkg == "top":
            return run_argv(["strata-debug", "app", "top"])
        cmd = ["strata-debug", "app", "inspect"]
        if pkg:
            cmd.append(pkg)
        return run_argv(cmd)

    if name == "app_control":
        action = str(args["action"])
        cmd = ["strata-debug", "app", action, str(args["package"])]
        opt_flag(cmd, "--activity", args.get("activity"))
        opt_flag(cmd, "--action", args.get("intentAction"))
        opt_flag(cmd, "--data", args.get("dataUri"))
        return run_argv(cmd)

    if name == "apk_install":
        local_path = str(args.get("localPath") or "").strip()
        remote_path = str(args.get("remotePath") or "").strip()
        if not local_path and not remote_path:
            raise ValueError("either localPath or remotePath is required")
        cleanup_remote = False
        if local_path:
            if not os.path.isfile(local_path):
                raise FileNotFoundError(f"APK file not found: {local_path}")
            remote_path = f"/data/local/tmp/strata_apk_{os.path.basename(local_path)}"
            ssh_kw = extract_ssh_overrides(args)
            ssh_kw["host_scope"] = True
            push_res = push_fn(local_path, remote_path, **ssh_kw)
            if not push_res.get("ok"):
                return {"ok": False, "stage": "push", **push_res}
            cleanup_remote = True

        cmd = ["strata-debug", "app", "install", remote_path]
        if args.get("grantPermissions") is False:
            cmd.append("--no-grant")
        if args.get("allowDowngrade"):
            cmd.append("--downgrade")
        if args.get("replace") is False:
            cmd.append("--no-replace")
        try:
            return run_argv(cmd, timeout=90.0)
        finally:
            if cleanup_remote:
                run_argv(["host-exec", "/system/bin/rm", "-f", remote_path], timeout=15.0)

    if name == "lsposed_inspect":
        mode = str(args.get("mode", "modules"))
        cmd = ["strata-debug", "lsposed", mode]
        opt_flag(cmd, "--module", args.get("module"))
        opt_flag(cmd, "--grep", args.get("grep"))
        opt_flag(cmd, "--limit", int(args["limit"]) if args.get("limit") is not None else None)
        return run_argv(cmd)

    if name == "sqlite_query":
        mode = str(args.get("mode", "query"))
        db_path = str(args["dbPath"])
        if mode == "tables":
            return run_argv(["strata-debug", "db", "tables", db_path])
        sql = str(args.get("sql") or "").strip()
        if not sql:
            raise ValueError("sql is required when mode is 'query'")
        cmd = ["strata-debug", "db", "query", db_path, "--sql", sql]
        opt_flag(cmd, "--limit", int(args["limit"]) if args.get("limit") is not None else None)
        return run_argv(cmd)

    if name == "ksu_inspect":
        mode = str(args.get("mode", "modules"))
        cmd = ["strata-debug", "ksu", mode]
        if mode == "sysfs":
            path = str(args.get("path") or "").strip()
            if not path:
                raise ValueError("path is required when mode is 'sysfs'")
            cmd.append(path)
        return run_argv(cmd)

    if name == "device_exec":
        scope = str(args.get("scope", "host"))
        timeout = max(1.0, min(60.0, float(args.get("timeoutSec", 30.0))))
        if scope == "container":
            cmd = ["/bin/sh", "-c", str(args["command"])]
        else:
            cmd = ["host-exec", "/system/bin/sh", "-c", str(args["command"])]
        return run_argv(cmd, timeout=timeout)

    if name == "file_transfer":
        direction = str(args["direction"])
        local_path = str(args["localPath"])
        remote_path = str(args["remotePath"])
        host_scope = bool(args.get("hostScope", False))
        kw = extract_ssh_overrides(args)
        if host_scope:
            kw["host_scope"] = True
        if direction == "push":
            return push_fn(local_path, remote_path, **kw) if kw else push_fn(local_path, remote_path)
        if direction == "pull":
            return pull_fn(remote_path, local_path, **kw) if kw else pull_fn(remote_path, local_path)
        raise ValueError(f"unsupported transfer direction: {direction}")

    raise KeyError(f"unknown android tool '{name}'")
