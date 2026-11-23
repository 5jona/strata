"""Candidate route construction, ADB forward management, and self-healing retry loop."""

from __future__ import annotations

from dataclasses import dataclass
import ipaddress
import os
import subprocess
from typing import Any, Callable

from . import devices


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


def cleanup_control_socket(ctl_path: str) -> None:
    try:
        if ctl_path and os.path.exists(ctl_path):
            os.unlink(ctl_path)
    except OSError:
        pass


def ensure_adb_forward(serial: str, local_port: int, remote_port: int) -> None:
    res = subprocess.run(
        ["adb", "-s", serial, "forward", f"tcp:{local_port}", f"tcp:{remote_port}"],
        capture_output=True, text=True, timeout=10.0, check=False,
    )
    if res.returncode != 0:
        raise RuntimeError(f"adb forward failed for {serial}: {(res.stderr or res.stdout).strip()}")


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
        res = run_once(route)
        if route.kind == "adb-ssh" and res.get("rc") == 255 and route.adb_serial and route.local_port:
            cleanup_control_socket(route.control_path)
            try:
                ensure_adb_forward(route.adb_serial, route.local_port, route.remote_ssh_port)
                res = run_once(route)
            except Exception as exc:
                res = {"ok": False, "rc": 255, "json": None, "stdout": "", "stderr": str(exc)}
        last_res = res
        if res.get("rc") not in (124, 255):
            return res
        if idx + 1 < len(routes):
            cleanup_control_socket(route.control_path)
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
