"""Multi-transport device runner supporting ssh, adb-forwarded ssh, host-exec, and local modes."""

from __future__ import annotations

import atexit
import json
import os
import re
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


def default_adb_serial(explicit: str | None = None, *, allow_env: bool = True) -> str | None:
    if explicit and explicit.strip():
        return explicit.strip()
    if allow_env:
        val = os.environ.get("STRATA_ADB_SERIAL", "").strip()
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


def _sanitize_control_token(value: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]+", "_", value)[:40] or "dev"


def _setup_adb_forward(serial: str, remote_port: int) -> int:
    res = subprocess.run(
        ["adb", "-s", serial, "forward", "tcp:0", f"tcp:{remote_port}"],
        text=True,
        capture_output=True,
        check=False,
    )
    out = res.stdout.strip()
    if res.returncode != 0 or not out.isdigit():
        detail = (res.stderr or res.stdout).strip()
        raise SystemExit(f"Failed to create ADB forward for '{serial}' -> tcp:{remote_port}: {detail}")
    return int(out)


def resolve_mode(mode: str, ssh_target: str | None, adb_serial: str | None = None) -> str:
    if mode not in {"auto", "ssh", "adb", "host-exec", "local"}:
        raise SystemExit(
            f"Unsupported transport mode '{mode}'. Use auto, ssh, adb, host-exec, or local."
        )
    if mode == "adb":
        return "ssh"
    if mode != "auto":
        return mode
    if adb_serial or ssh_target:
        return "ssh"
    if os.path.isfile(HOST_EXEC) and os.access(HOST_EXEC, os.X_OK):
        return "host-exec"
    if hasattr(os, "geteuid") and os.geteuid() == 0 and os.path.exists("/data/adb/ksu/bin/strata"):
        return "local"
    raise SystemExit(
        "Unable to resolve on-device runtime. Pass --ssh <host|user@host> or --adb <serial> "
        "(or set STRATA_SSH_HOST / STRATA_ADB_SERIAL), or run on-device."
    )


class DeviceRunner:
    def __init__(
        self,
        *,
        mode: str = "auto",
        ssh_target: str | None = None,
        ssh_port: int | None = None,
        adb_serial: str | None = None,
    ) -> None:
        self.adb_serial: str | None = None
        self._adb_local_port: int | None = None
        self._adb_control_path: str | None = None

        explicit_adb = default_adb_serial(adb_serial, allow_env=False)
        resolved_ssh = None if explicit_adb else default_ssh_target(ssh_target)
        resolved_adb = explicit_adb or (
            default_adb_serial(None, allow_env=True)
            if not resolved_ssh and mode in {"auto", "adb"}
            else None
        )

        self.mode = resolve_mode(mode, resolved_ssh, resolved_adb)
        remote_port = default_ssh_port(ssh_port)

        if mode == "adb" and not resolved_adb:
            raise SystemExit("mode=adb requires --adb <serial> or STRATA_ADB_SERIAL")

        if resolved_adb and self.mode == "ssh":
            self.adb_serial = resolved_adb
            self._adb_local_port = _setup_adb_forward(resolved_adb, remote_port)
            self.ssh_target: str | None = "root@127.0.0.1"
            self.ssh_port = self._adb_local_port
            token = _sanitize_control_token(resolved_adb)
            self._adb_control_path = os.path.join(
                _ensure_ssh_control_dir(), f"adb-{token}-{self.ssh_port}.sock"
            )
            atexit.register(self.close)
        else:
            self.ssh_port = remote_port
            if resolved_ssh and "@" not in resolved_ssh:
                resolved_ssh = f"root@{resolved_ssh}"
            self.ssh_target = resolved_ssh
            if self.mode == "ssh" and not self.ssh_target:
                raise SystemExit(
                    "mode=ssh requires --ssh <host|user@host>, --adb <serial>, or STRATA_SSH_HOST"
                )

    def __enter__(self) -> DeviceRunner:
        return self

    def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
        self.close()

    def close(self) -> None:
        if self._adb_control_path and self.ssh_target:
            subprocess.run(
                ["ssh", "-S", self._adb_control_path, "-O", "exit", self.ssh_target],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            )
            self._adb_control_path = None
        if self.adb_serial and self._adb_local_port is not None:
            subprocess.run(
                ["adb", "-s", self.adb_serial, "forward", "--remove", f"tcp:{self._adb_local_port}"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            )
            self._adb_local_port = None

    def _ssh_prefix(self) -> list[str]:
        assert self.ssh_target is not None
        if self.adb_serial and self._adb_control_path:
            return [
                "ssh",
                "-p",
                str(self.ssh_port),
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=8",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-o",
                "LogLevel=ERROR",
                "-o",
                "ControlMaster=auto",
                "-o",
                "ControlPersist=120",
                "-o",
                f"ControlPath={self._adb_control_path}",
                self.ssh_target,
            ]
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
