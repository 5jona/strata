"""Modular PC-side MCP server package for Strata generic Android debugging."""

from __future__ import annotations

from typing import Any, Callable

from . import devices, transport
from .tools_android import ANDROID_TOOL_NAMES, ANDROID_TOOLS, dispatch_android_tool
from .tools_core import CORE_TOOL_NAMES, CORE_TOOLS, dispatch_core_tool
from .tools_devices import DEVICE_TOOL_NAMES, DEVICE_TOOLS, dispatch_device_tool

TOOLS: list[dict[str, Any]] = CORE_TOOLS + ANDROID_TOOLS + DEVICE_TOOLS


def dispatch_tool(
    name: str,
    args: dict[str, Any],
    *,
    exec_fn: Callable[..., dict[str, Any]] = transport.ssh_exec,
    push_fn: Callable[..., dict[str, Any]] = transport.scp_push,
    pull_fn: Callable[..., dict[str, Any]] = transport.scp_pull,
    script_dir: str = "",
    repo_root: str = "",
) -> Any:
    if name in CORE_TOOL_NAMES:
        return dispatch_core_tool(
            name,
            args,
            exec_fn=exec_fn,
            pull_fn=pull_fn,
            script_dir=script_dir,
            repo_root=repo_root,
        )
    if name in ANDROID_TOOL_NAMES:
        return dispatch_android_tool(
            name,
            args,
            exec_fn=exec_fn,
            push_fn=push_fn,
            pull_fn=pull_fn,
        )
    if name in DEVICE_TOOL_NAMES:
        return dispatch_device_tool(
            name,
            args,
            exec_fn=exec_fn,
            pull_fn=pull_fn,
        )
    raise KeyError(f"unknown tool '{name}'")


__all__ = [
    "ANDROID_TOOLS",
    "CORE_TOOLS",
    "DEVICE_TOOLS",
    "TOOLS",
    "devices",
    "dispatch_tool",
    "transport",
]
