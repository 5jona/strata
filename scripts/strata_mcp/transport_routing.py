"""Candidate route construction, ADB forward management, and self-healing retry loop."""

from __future__ import annotations

import atexit
from dataclasses import dataclass
import ipaddress
import os
import subprocess
from typing import Any, Callable

from . import devices

_OWNED_CONTROL_ROUTES: dict[str, tuple[str, str, str]] = {}
_OWNED_SSH_FORWARDS: set[tuple[str, str, str, str, int, int]] = set()
_OWNED_ADB_FORWARDS: set[tuple[str, int]] = set()


@dataclass
class TransportRoute:
    kind: str  # "ssh" | "adb-ssh"
    host: str
    port: str
    user: str
    device_key: str
    control_path: str
    known_hosts_path: str | None = None
    adb_serial: str | None = None
    local_port: int | None = None
    remote_ssh_port: int = 2222


def is_ip_literal(host: str) -> bool:
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
        raise RuntimeError("STRATA_SSH_HOST environment variable is not set and no device was discovered")
    raw_port = str(port).strip() if port is not None and str(port).strip() else os.environ.get("STRATA_SSH_PORT", "").strip()
    raw_user = str(user).strip() if user is not None and str(user).strip() else os.environ.get("STRATA_SSH_USER", "").strip()
    if is_ip_literal(eff_host):
        return eff_host, raw_port or "2222", raw_user or "root"
    return eff_host, raw_port, raw_user


def _routes_for_record(
    rec: devices.DeviceRecord,
    *,
    port: str | int | None = None,
    user: str | None = None,
) -> list[TransportRoute]:
    ctl = devices.device_control_path(rec.id)
    kh = devices.device_known_hosts_path(rec.id)
    ssh_route: TransportRoute | None = None
    adb_route: TransportRoute | None = None

    if rec.ssh_host:
        p = str(port).strip() if port is not None else (str(rec.ssh_port) if rec.ssh_port else ("2222" if is_ip_literal(rec.ssh_host) else ""))
        u = str(user).strip() if user is not None else (rec.ssh_user or ("root" if is_ip_literal(rec.ssh_host) else ""))
        ssh_route = TransportRoute("ssh", rec.ssh_host, p, u, rec.id, ctl, kh)

    adb_serial = rec.adb_usb_serial or rec.adb_tcpip_serial or rec.adb_serial
    if adb_serial and rec.adb_state == "device":
        remote_p = int(port) if port is not None else (rec.ssh_port or 2222)
        local_p = devices.allocate_forward_port(rec.id)
        u = str(user).strip() if user is not None else (rec.ssh_user or "root")
        adb_route = TransportRoute(
            "adb-ssh", "127.0.0.1", str(local_p), u, rec.id, ctl, kh,
            adb_serial=adb_serial, local_port=local_p, remote_ssh_port=remote_p,
        )

    ordered = [adb_route, ssh_route] if rec.prefer_transport == "adb" else [ssh_route, adb_route]
    routes = [r for r in ordered if r is not None]
    if not routes:
        raise RuntimeError(f"device '{rec.id}' has no usable SSH or online ADB transport")
    return routes


def build_candidate_routes(
    *,
    device: str | None = None,
    host: str | None = None,
    port: str | int | None = None,
    user: str | None = None,
) -> list[TransportRoute]:
    rec = devices.resolve_device(device=device, host=host)
    if rec is not None:
        return _routes_for_record(rec, port=port, user=user)
    eff_host, eff_port, eff_user = ssh_config(host=host, port=port, user=user)
    key = f"host-{eff_host}-{eff_port or 'default'}"
    return [TransportRoute(
        "ssh", eff_host, eff_port, eff_user, key,
        devices.device_control_path(key), devices.device_known_hosts_path(key),
    )]


def register_control_route(route: TransportRoute) -> None:
    if route.control_path:
        _OWNED_CONTROL_ROUTES[route.control_path] = (route.host, route.port, route.user)


def register_ssh_forward(route: TransportRoute, local_port: int, remote_port: int) -> None:
    register_control_route(route)
    if route.control_path:
        _OWNED_SSH_FORWARDS.add((route.control_path, route.host, route.port, route.user, local_port, remote_port))


def reset_transport_state() -> None:
    """Clear in-memory transport cleanup registries (for unit tests)."""
    _OWNED_CONTROL_ROUTES.clear()
    _OWNED_SSH_FORWARDS.clear()
    _OWNED_ADB_FORWARDS.clear()


def _ssh_ctl_cmd(ctl_path: str, host: str, port: str, user: str, extra: list[str]) -> list[str]:
    dest = f"{user}@{host}" if (user and host) else (host or "_")
    cmd = ["ssh"]
    if port:
        cmd.extend(["-p", str(port)])
    cmd.extend(["-o", "BatchMode=yes", "-o", f"ControlPath={ctl_path}", *extra, dest])
    return cmd


def cleanup_control_socket(
    ctl_path: str,
    *,
    host: str | None = None,
    port: str | None = None,
    user: str | None = None,
) -> None:
    if not ctl_path:
        return
    saved = _OWNED_CONTROL_ROUTES.pop(ctl_path, None)
    eff_host = host if host is not None else (saved[0] if saved else "")
    eff_port = port if port is not None else (saved[1] if saved else "")
    eff_user = user if user is not None else (saved[2] if saved else "")
    if "%" in ctl_path or os.path.exists(ctl_path):
        try:
            subprocess.run(
                _ssh_ctl_cmd(ctl_path, eff_host, eff_port, eff_user, ["-O", "exit"]),
                capture_output=True, text=True, timeout=5.0, check=False,
            )
        except (OSError, subprocess.TimeoutExpired):
            pass
    try:
        if os.path.exists(ctl_path):
            os.unlink(ctl_path)
    except OSError:
        pass


def cleanup_all_transports() -> None:
    """Best-effort atexit cleanup for owned SSH port forwards, ControlMasters, and ADB forwards."""
    for ctl, host, port, user, lport, rport in list(_OWNED_SSH_FORWARDS):
        if "%" in ctl or os.path.exists(ctl):
            try:
                subprocess.run(
                    _ssh_ctl_cmd(ctl, host, port, user, ["-O", "cancel", "-L", f"127.0.0.1:{lport}:127.0.0.1:{rport}"]),
                    capture_output=True, text=True, timeout=5.0, check=False,
                )
            except (OSError, subprocess.TimeoutExpired):
                pass
    _OWNED_SSH_FORWARDS.clear()
    for ctl, (host, port, user) in list(_OWNED_CONTROL_ROUTES.items()):
        cleanup_control_socket(ctl, host=host, port=port, user=user)
    _OWNED_CONTROL_ROUTES.clear()
    for serial, lport in list(_OWNED_ADB_FORWARDS):
        try:
            subprocess.run(
                ["adb", "-s", serial, "forward", "--remove", f"tcp:{lport}"],
                capture_output=True, text=True, timeout=5.0, check=False,
            )
        except (OSError, subprocess.TimeoutExpired):
            pass
    _OWNED_ADB_FORWARDS.clear()


atexit.register(cleanup_all_transports)


def ensure_adb_forward(serial: str, local_port: int, remote_port: int) -> None:
    res = subprocess.run(
        ["adb", "-s", serial, "forward", f"tcp:{local_port}", f"tcp:{remote_port}"],
        capture_output=True, text=True, timeout=10.0, check=False,
    )
    if res.returncode != 0:
        raise RuntimeError(f"adb forward failed for {serial}: {(res.stderr or res.stdout).strip()}")
    _OWNED_ADB_FORWARDS.add((serial, local_port))


def run_with_routes(
    routes: list[TransportRoute],
    run_once: Callable[[TransportRoute], dict[str, Any]],
) -> dict[str, Any]:
    last_res: dict[str, Any] = {"ok": False, "rc": 255, "stderr": "no transport route available"}
    for idx, route in enumerate(routes):
        try:
            if route.kind == "adb-ssh" and route.adb_serial and route.local_port:
                ensure_adb_forward(route.adb_serial, route.local_port, route.remote_ssh_port)
        except Exception as exc:
            last_res = {"ok": False, "rc": 255, "json": None, "stdout": "", "stderr": str(exc)}
            continue
        register_control_route(route)
        res = run_once(route)
        if route.kind == "adb-ssh" and res.get("rc") == 255 and route.adb_serial and route.local_port:
            cleanup_control_socket(route.control_path, host=route.host, port=route.port, user=route.user)
            try:
                ensure_adb_forward(route.adb_serial, route.local_port, route.remote_ssh_port)
                register_control_route(route)
                res = run_once(route)
            except Exception as exc:
                res = {"ok": False, "rc": 255, "json": None, "stdout": "", "stderr": str(exc)}
        last_res = res
        if res.get("rc") not in (124, 255):
            return res
        if idx + 1 < len(routes):
            cleanup_control_socket(route.control_path, host=route.host, port=route.port, user=route.user)
    return last_res


def resolve_ssh_target(
    *,
    device: str | None = None,
    host: str | None = None,
    port: str | int | None = None,
    user: str | None = None,
) -> tuple[str, str, str]:
    route = build_candidate_routes(device=device, host=host, port=port, user=user)[0]
    if route.kind == "adb-ssh" and route.adb_serial and route.local_port:
        ensure_adb_forward(route.adb_serial, route.local_port, route.remote_ssh_port)
    return route.host, route.port, route.user
