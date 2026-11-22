"""Multi-transport device runner supporting ssh, host-exec, and local modes."""

from __future__ import annotations

import json
import os
import shlex
import shutil
import subprocess
import tempfile

HOST_EXEC = "/usr/local/bin/host-exec"


def _ensure_ssh_control_dir() -> str:
    xdg = os.environ.get("XDG_RUNTIME_DIR", "").strip()
    if xdg and os.path.isdir(xdg):
        control_dir = os.path.join(xdg, "strata-verify")
    else:
        uid = os.getuid() if hasattr(os, "getuid") else 0
        control_dir = os.path.join(tempfile.gettempdir(), f"strata-verify-{uid}")
    os.makedirs(control_dir, mode=0o700, exist_ok=True)
    os.chmod(control_dir, 0o700)
    return control_dir


def default_ssh_target(explicit: str | None = None) -> str | None:
    if explicit and explicit.strip():
        return explicit.strip()
    for key in ("STRATA_SSH_HOST", "SSH_TARGET"):
        val = os.environ.get(key, "").strip()
        if val:
            return val
    return None


def default_ssh_port(explicit: int | None = None) -> int:
    if explicit is not None:
        return explicit
    for key in ("STRATA_SSH_PORT", "SSH_PORT"):
        raw = os.environ.get(key, "").strip()
        if raw.isdigit():
            return int(raw)
    return 2222


def resolve_mode(mode: str, ssh_target: str | None) -> str:
    if mode not in {"auto", "ssh", "host-exec", "local"}:
        raise SystemExit(f"Unsupported transport mode '{mode}'. Use auto, ssh, host-exec, or local.")
    if mode != "auto":
        return mode
    if ssh_target:
        return "ssh"
    if os.path.isfile(HOST_EXEC) and os.access(HOST_EXEC, os.X_OK):
        return "host-exec"
    if hasattr(os, "geteuid") and os.geteuid() == 0 and os.path.exists("/data/adb/ksu/bin/strata"):
        return "local"
    raise SystemExit(
        "Unable to resolve on-device runtime. Pass --ssh <host|user@host> "
        "(or set STRATA_SSH_HOST / SSH_TARGET), or run on-device."
    )


class DeviceRunner:
    def __init__(
        self,
        *,
        mode: str = "auto",
        ssh_target: str | None = None,
        ssh_port: int | None = None,
    ) -> None:
        resolved_ssh = default_ssh_target(ssh_target)
        self.mode = resolve_mode(mode, resolved_ssh)
        self.ssh_port = default_ssh_port(ssh_port)
        if resolved_ssh and "@" not in resolved_ssh:
            resolved_ssh = f"root@{resolved_ssh}"
        self.ssh_target = resolved_ssh
        if self.mode == "ssh" and not self.ssh_target:
            raise SystemExit("mode=ssh requires --ssh <host|user@host> or STRATA_SSH_HOST / SSH_TARGET")

    def _ssh_prefix(self) -> list[str]:
        assert self.ssh_target is not None
        control_path = os.path.join(_ensure_ssh_control_dir(), "%r@%h:%p")
        return [
            "ssh",
            "-p",
            str(self.ssh_port),
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=8",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            "ControlMaster=auto",
            "-o",
            "ControlPersist=120",
            "-o",
            f"ControlPath={control_path}",
            self.ssh_target,
        ]

    def build_host_argv(self, cmd: str) -> list[str]:
        if self.mode == "ssh":
            return [*self._ssh_prefix(), f"{HOST_EXEC} /system/bin/sh -c {shlex.quote(cmd)}"]
        if self.mode == "host-exec":
            return [HOST_EXEC, "/system/bin/sh", "-c", cmd]
        return ["/system/bin/sh", "-c", cmd]

    def su(self, cmd: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        res = subprocess.run(self.build_host_argv(cmd), text=True, capture_output=True, check=False)
        if check and res.returncode != 0:
            raise RuntimeError(
                f"Command failed ({self.mode}, rc={res.returncode}): {cmd}\n"
                f"stdout: {res.stdout}\n"
                f"stderr: {res.stderr}"
            )
        return res

    def container_cmd(self, cmd: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        if self.mode == "ssh":
            argv = [*self._ssh_prefix(), cmd]
        elif self.mode == "host-exec":
            argv = ["/bin/sh", "-c", cmd]
        else:
            return self.su(f"/data/adb/ksu/bin/strata exec /bin/sh -c {shlex.quote(cmd)}", check=check)
        res = subprocess.run(argv, text=True, capture_output=True, check=False)
        if check and res.returncode != 0:
            raise RuntimeError(
                f"Container command failed ({self.mode}, rc={res.returncode}): {cmd}\n"
                f"stdout: {res.stdout}\n"
                f"stderr: {res.stderr}"
            )
        return res

    def strata_json(self, args: str) -> dict | list:
        res = self.su(f"/data/adb/ksu/bin/strata --json {args}")
        return json.loads(res.stdout)

    def pull_host_file(self, remote_path: str, local_path: str) -> None:
        if self.mode == "local":
            shutil.copyfile(remote_path, local_path)
            return
        argv = self.build_host_argv(f"/system/bin/cat {shlex.quote(remote_path)}")
        res = subprocess.run(argv, capture_output=True, check=True)
        with open(local_path, "wb") as f:
            f.write(res.stdout)
