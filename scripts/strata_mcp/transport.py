"""SSH transport, ControlMaster pooling, and binary pipe file transfer for Strata MCP."""

from __future__ import annotations

import ipaddress
import json
import os
import shlex
import subprocess
import tempfile
from typing import Any, Callable

MAX_TIMEOUT_SEC = 120.0

S: dict[str, Any] = {"type": "string"}
I: dict[str, Any] = {"type": "integer"}
N: dict[str, Any] = {"type": "number"}
B: dict[str, Any] = {"type": "boolean"}


def make_tool(name: str, desc: str, props: dict[str, Any], req: list[str] | None = None) -> dict[str, Any]:
    merged = {**props, "host": S, "port": I}
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
    if kw:
        return exec_fn(remote_cmd, timeout=timeout, **kw)
    return exec_fn(remote_cmd, timeout=timeout)


def _is_ip_literal(host: str) -> bool:
    try:
        ipaddress.ip_address(host.strip("[]"))
        return True
    except ValueError:
        return False


def ssh_config(
    host: str | None = None,
    port: str | int | None = None,
    user: str | None = None,
) -> tuple[str, str, str]:
    eff_host = str(host).strip() if host is not None and str(host).strip() else os.environ.get("STRATA_SSH_HOST", "").strip()
    if not eff_host:
        raise RuntimeError("STRATA_SSH_HOST environment variable is not set")

    raw_port = str(port).strip() if port is not None and str(port).strip() else os.environ.get("STRATA_SSH_PORT", "").strip()
    raw_user = str(user).strip() if user is not None and str(user).strip() else os.environ.get("STRATA_SSH_USER", "").strip()

    if _is_ip_literal(eff_host):
        return eff_host, raw_port or "2222", raw_user or "root"
    return eff_host, raw_port, raw_user


def control_path() -> str:
    xdg = os.environ.get("XDG_RUNTIME_DIR", "").strip()
    ctl_dir = os.path.join(xdg, "strata-mcp") if xdg else os.path.join(tempfile.gettempdir(), f"strata-mcp-{os.getuid()}")
    os.makedirs(ctl_dir, mode=0o700, exist_ok=True)
    os.chmod(ctl_dir, 0o700)
    return os.path.join(ctl_dir, "cm-%r@%h:%p")


def build_ssh_base_args(host: str, port: str | None = None, user: str | None = None) -> list[str]:
    cmd = ["ssh"]
    if port:
        cmd.extend(["-p", str(port)])
    cmd.extend([
        "-o", "BatchMode=yes",
        "-o", "ConnectTimeout=8",
        "-o", "StrictHostKeyChecking=accept-new",
        "-o", "ControlMaster=auto",
        "-o", "ControlPersist=300",
        "-o", f"ControlPath={control_path()}",
        f"{user}@{host}" if user else host,
    ])
    return cmd


def ssh_exec(
    remote_cmd: str,
    timeout: float = 30.0,
    *,
    host: str | None = None,
    port: str | int | None = None,
    user: str | None = None,
) -> dict[str, Any]:
    eff_host, eff_port, eff_user = ssh_config(host=host, port=port, user=user)
    eff = max(1.0, min(MAX_TIMEOUT_SEC, float(timeout)))
    try:
        res = subprocess.run(
            [*build_ssh_base_args(eff_host, eff_port, eff_user), remote_cmd],
            capture_output=True,
            text=True,
            timeout=eff,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return {"ok": False, "rc": 124, "json": None, "stdout": "", "stderr": f"SSH command timed out after {eff}s"}
    stdout = res.stdout.strip()
    try:
        parsed = json.loads(stdout) if stdout else None
    except json.JSONDecodeError:
        parsed = None
    return {"rc": res.returncode, "json": parsed, "stdout": stdout, "stderr": res.stderr.strip()}


def scp_push(
    local_path: str,
    remote_path: str,
    timeout: float = 60.0,
    *,
    host: str | None = None,
    port: str | int | None = None,
    user: str | None = None,
    host_scope: bool = False,
) -> dict[str, Any]:
    if not os.path.isfile(local_path):
        raise FileNotFoundError(f"local file not found: {local_path}")
    eff_host, eff_port, eff_user = ssh_config(host=host, port=port, user=user)
    eff = max(1.0, min(MAX_TIMEOUT_SEC, float(timeout)))
    quoted_remote = shlex.quote(remote_path)
    if host_scope:
        remote_cmd = f"host-exec /system/bin/sh -c {shlex.quote(f'cat > {quoted_remote}')}"
    else:
        remote_cmd = f"cat > {quoted_remote}"
    try:
        with open(local_path, "rb") as src:
            res = subprocess.run(
                [*build_ssh_base_args(eff_host, eff_port, eff_user), remote_cmd],
                stdin=src,
                capture_output=True,
                timeout=eff,
                check=False,
            )
    except subprocess.TimeoutExpired:
        return {"ok": False, "rc": 124, "localPath": local_path, "remotePath": remote_path, "stderr": f"push timed out after {eff}s"}
    return {
        "ok": res.returncode == 0,
        "rc": res.returncode,
        "localPath": local_path,
        "remotePath": remote_path,
        "bytes": os.path.getsize(local_path),
        "stderr": res.stderr.decode("utf-8", errors="replace").strip(),
    }


def scp_pull(
    remote_path: str,
    local_path: str,
    timeout: float = 60.0,
    *,
    host: str | None = None,
    port: str | int | None = None,
    user: str | None = None,
    host_scope: bool = False,
) -> dict[str, Any]:
    eff_host, eff_port, eff_user = ssh_config(host=host, port=port, user=user)
    eff = max(1.0, min(MAX_TIMEOUT_SEC, float(timeout)))
    quoted_remote = shlex.quote(remote_path)
    if host_scope:
        remote_cmd = f"host-exec /system/bin/cat {quoted_remote}"
    else:
        remote_cmd = f"cat {quoted_remote}"
    try:
        res = subprocess.run(
            [*build_ssh_base_args(eff_host, eff_port, eff_user), remote_cmd],
            capture_output=True,
            timeout=eff,
            check=False,
        )
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

    return {
        "ok": True,
        "rc": 0,
        "remotePath": remote_path,
        "localPath": local_path,
        "bytes": len(res.stdout),
        "stderr": stderr,
    }
