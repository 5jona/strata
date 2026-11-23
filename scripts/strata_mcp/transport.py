"""SSH/ADB-SSH transport, ControlMaster pooling, port forwarding, and binary pipe file transfer."""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import tempfile
from typing import Any, Callable

from .transport_routing import (
    TransportRoute,
    build_candidate_routes,
    cleanup_control_socket,
    ensure_adb_forward,
    resolve_ssh_target,
    run_with_routes,
    ssh_config,
)

MAX_TIMEOUT_SEC = 120.0

S: dict[str, Any] = {"type": "string"}
I: dict[str, Any] = {"type": "integer"}
N: dict[str, Any] = {"type": "number"}
B: dict[str, Any] = {"type": "boolean"}
DEVICE_PROP: dict[str, Any] = {
    "type": "string",
    "description": "Target device alias, ADB serial, or model; specify explicitly when multiple devices are online.",
}


def make_tool(
    name: str,
    desc: str,
    props: dict[str, Any],
    req: list[str] | None = None,
    *,
    include_target: bool = True,
) -> dict[str, Any]:
    merged = {**props, "device": DEVICE_PROP, "host": S, "port": I} if include_target else dict(props)
    schema: dict[str, Any] = {"type": "object", "properties": merged, "additionalProperties": False}
    if req:
        schema["required"] = req
    return {"name": name, "description": desc, "inputSchema": schema}


def opt_flag(cmd: list[str], flag: str, val: Any) -> None:
    if val is not None and val != "":
        cmd.extend([flag, str(val)])


def argv_to_cmd(argv: list[str]) -> str:
    return " ".join(shlex.quote(c) for c in argv)


def extract_ssh_overrides(args: dict[str, Any] | None) -> dict[str, Any]:
    if not args:
        return {}
    out: dict[str, Any] = {}
    if args.get("device") is not None and str(args["device"]).strip():
        out["device"] = str(args["device"]).strip()
    if args.get("host") is not None and str(args["host"]).strip():
        out["host"] = str(args["host"]).strip()
    if args.get("port") is not None:
        out["port"] = int(args["port"])
    return out


def invoke_exec(
    exec_fn: Callable[..., dict[str, Any]],
    remote_cmd: str,
    timeout: float = 30.0,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    kw = extract_ssh_overrides(args)
    return exec_fn(remote_cmd, timeout=timeout, **kw) if kw else exec_fn(remote_cmd, timeout=timeout)


def control_path() -> str:
    xdg = os.environ.get("XDG_RUNTIME_DIR", "").strip()
    ctl_dir = os.path.join(xdg, "strata-mcp") if xdg else os.path.join(tempfile.gettempdir(), f"strata-mcp-{os.getuid()}")
    os.makedirs(ctl_dir, mode=0o700, exist_ok=True)
    os.chmod(ctl_dir, 0o700)
    return os.path.join(ctl_dir, "cm-%r@%h:%p")


def build_ssh_base_args(
    host: str,
    port: str | None = None,
    user: str | None = None,
    *,
    control_path_override: str | None = None,
    known_hosts_path: str | None = None,
) -> list[str]:
    cmd = ["ssh"]
    if port:
        cmd.extend(["-p", str(port)])
    cmd.extend([
        "-o", "BatchMode=yes",
        "-o", "ConnectTimeout=8",
        "-o", "StrictHostKeyChecking=accept-new",
        "-o", "ControlMaster=auto",
        "-o", "ControlPersist=300",
        "-o", f"ControlPath={control_path_override or control_path()}",
    ])
    if known_hosts_path:
        cmd.extend(["-o", f"UserKnownHostsFile={known_hosts_path}"])
    cmd.append(f"{user}@{host}" if user else host)
    return cmd


def ssh_exec(
    remote_cmd: str,
    timeout: float = 30.0,
    *,
    device: str | None = None,
    host: str | None = None,
    port: str | int | None = None,
    user: str | None = None,
) -> dict[str, Any]:
    routes = build_candidate_routes(device=device, host=host, port=port, user=user)
    eff = max(1.0, min(MAX_TIMEOUT_SEC, float(timeout)))

    def _once(route: TransportRoute) -> dict[str, Any]:
        args = build_ssh_base_args(
            route.host, route.port, route.user,
            control_path_override=route.control_path, known_hosts_path=route.known_hosts_path,
        )
        try:
            res = subprocess.run([*args, remote_cmd], capture_output=True, text=True, timeout=eff, check=False)
        except subprocess.TimeoutExpired:
            return {"ok": False, "rc": 124, "json": None, "stdout": "", "stderr": f"SSH command timed out after {eff}s"}
        stdout = res.stdout.strip()
        try:
            parsed = json.loads(stdout) if stdout else None
        except json.JSONDecodeError:
            parsed = None
        return {"rc": res.returncode, "json": parsed, "stdout": stdout, "stderr": res.stderr.strip()}

    return run_with_routes(routes, _once)


def scp_push(
    local_path: str,
    remote_path: str,
    timeout: float = 60.0,
    *,
    device: str | None = None,
    host: str | None = None,
    port: str | int | None = None,
    user: str | None = None,
    host_scope: bool = False,
) -> dict[str, Any]:
    if not os.path.isfile(local_path):
        raise FileNotFoundError(f"local file not found: {local_path}")
    routes = build_candidate_routes(device=device, host=host, port=port, user=user)
    eff = max(1.0, min(MAX_TIMEOUT_SEC, float(timeout)))
    q = shlex.quote(remote_path)
    remote_cmd = f"host-exec /system/bin/sh -c {shlex.quote(f'cat > {q}')}" if host_scope else f"cat > {q}"

    def _once(route: TransportRoute) -> dict[str, Any]:
        args = build_ssh_base_args(
            route.host, route.port, route.user,
            control_path_override=route.control_path, known_hosts_path=route.known_hosts_path,
        )
        try:
            with open(local_path, "rb") as src:
                res = subprocess.run([*args, remote_cmd], stdin=src, capture_output=True, timeout=eff, check=False)
        except subprocess.TimeoutExpired:
            return {"ok": False, "rc": 124, "localPath": local_path, "remotePath": remote_path, "stderr": f"push timed out after {eff}s"}
        return {
            "ok": res.returncode == 0, "rc": res.returncode, "localPath": local_path, "remotePath": remote_path,
            "bytes": os.path.getsize(local_path), "stderr": res.stderr.decode("utf-8", errors="replace").strip(),
        }

    return run_with_routes(routes, _once)


def scp_pull(
    remote_path: str,
    local_path: str,
    timeout: float = 60.0,
    *,
    device: str | None = None,
    host: str | None = None,
    port: str | int | None = None,
    user: str | None = None,
    host_scope: bool = False,
) -> dict[str, Any]:
    routes = build_candidate_routes(device=device, host=host, port=port, user=user)
    eff = max(1.0, min(MAX_TIMEOUT_SEC, float(timeout)))
    q = shlex.quote(remote_path)
    remote_cmd = f"host-exec /system/bin/cat {q}" if host_scope else f"cat {q}"

    def _once(route: TransportRoute) -> dict[str, Any]:
        args = build_ssh_base_args(
            route.host, route.port, route.user,
            control_path_override=route.control_path, known_hosts_path=route.known_hosts_path,
        )
        try:
            res = subprocess.run([*args, remote_cmd], capture_output=True, timeout=eff, check=False)
        except subprocess.TimeoutExpired:
            return {"ok": False, "rc": 124, "remotePath": remote_path, "localPath": local_path, "stderr": f"pull timed out after {eff}s"}
        stderr = res.stderr.decode("utf-8", errors="replace").strip()
        if res.returncode != 0:
            return {"ok": False, "rc": res.returncode, "remotePath": remote_path, "localPath": local_path, "stderr": stderr}
        abs_local = os.path.abspath(local_path)
        parent = os.path.dirname(abs_local)
        os.makedirs(parent, exist_ok=True)
        tmp_path = ""
        try:
            with tempfile.NamedTemporaryFile(dir=parent, prefix=".strata-pull-", delete=False) as tmp:
                tmp_path = tmp.name
                tmp.write(res.stdout)
                tmp.flush()
                os.fsync(tmp.fileno())
            os.replace(tmp_path, abs_local)
        except BaseException:
            if tmp_path and os.path.exists(tmp_path):
                os.unlink(tmp_path)
            raise
        return {"ok": True, "rc": 0, "remotePath": remote_path, "localPath": local_path, "bytes": len(res.stdout), "stderr": stderr}

    return run_with_routes(routes, _once)


def open_ssh_port_forward(
    local_port: int,
    remote_port: int,
    *,
    device: str | None = None,
    host: str | None = None,
    port: str | int | None = None,
    user: str | None = None,
) -> dict[str, Any]:
    routes = build_candidate_routes(device=device, host=host, port=port, user=user)

    def _once(route: TransportRoute) -> dict[str, Any]:
        cmd = ["ssh"] + (["-p", route.port] if route.port else []) + [
            "-f", "-N", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes",
            "-o", "StrictHostKeyChecking=accept-new", "-o", f"ControlPath={route.control_path}",
        ]
        if route.known_hosts_path:
            cmd.extend(["-o", f"UserKnownHostsFile={route.known_hosts_path}"])
        cmd.extend(["-L", f"127.0.0.1:{local_port}:127.0.0.1:{remote_port}", f"{route.user}@{route.host}" if route.user else route.host])
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=15.0, check=False)
        return {"ok": res.returncode == 0, "rc": res.returncode, "stderr": res.stderr.strip(), "route": route.kind}

    return run_with_routes(routes, _once)


__all__ = [
    "B", "DEVICE_PROP", "I", "MAX_TIMEOUT_SEC", "N", "S", "TransportRoute",
    "argv_to_cmd", "build_candidate_routes", "build_ssh_base_args", "cleanup_control_socket",
    "control_path", "ensure_adb_forward", "extract_ssh_overrides", "invoke_exec",
    "make_tool", "open_ssh_port_forward", "opt_flag", "resolve_ssh_target",
    "run_with_routes", "scp_pull", "scp_push", "ssh_config", "ssh_exec",
]
