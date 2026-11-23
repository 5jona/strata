"""Multi-device session selection, fail-fast resolution, and per-device SSH/ADB isolation."""

from __future__ import annotations

import hashlib
import os
import re
import socket
import tempfile

from .device_discovery import (
    AdbDeviceEntry,
    DeviceRecord,
    SshHostEntry,
    discover_adb_devices,
    discover_devices,
    load_devices_config,
    load_ssh_config_entries,
    parse_adb_devices_output,
    parse_ssh_config_text,
)

FORWARD_PORT_BASE = 12222
FORWARD_PORT_COUNT = 78  # 12222..12299

_SELECTED_DEVICE_ID: str | None = None
_ALLOCATED_PORTS: dict[str, int] = {}


def reset_device_state() -> None:
    """Reset in-memory selection and forward port cache (for tests)."""
    global _SELECTED_DEVICE_ID
    _SELECTED_DEVICE_ID = None
    _ALLOCATED_PORTS.clear()


def _match_devices(records: list[DeviceRecord], ref: str) -> list[DeviceRecord]:
    needle = ref.strip()
    low = needle.lower()
    exact = [
        r for r in records
        if needle in {r.id, r.alias, r.adb_serial, r.adb_usb_serial, r.adb_tcpip_serial, r.ssh_host}
    ]
    if exact:
        return exact
    return [
        r for r in records
        if low in {
            v.lower()
            for v in (r.id, r.alias, r.adb_serial, r.adb_usb_serial, r.adb_tcpip_serial, r.ssh_host, r.model)
            if v
        }
    ]


def _format_available(records: list[DeviceRecord]) -> str:
    return ", ".join(f"{r.id} (model={r.model or 'unknown'}, transport={r.transport_summary})" for r in records)


def get_selected_device() -> str | None:
    return _SELECTED_DEVICE_ID


def select_device(device_ref: str | None, *, records: list[DeviceRecord] | None = None) -> DeviceRecord | None:
    global _SELECTED_DEVICE_ID
    if device_ref is None or not str(device_ref).strip():
        _SELECTED_DEVICE_ID = None
        return None
    all_devs = discover_devices() if records is None else records
    matches = _match_devices(all_devs, str(device_ref))
    if not matches:
        raise ValueError(f"device '{device_ref}' not found. Available: {_format_available(all_devs) or 'none'}")
    if len(matches) > 1:
        raise ValueError(f"ambiguous device '{device_ref}' matches: {_format_available(matches)}")
    _SELECTED_DEVICE_ID = matches[0].id
    return matches[0]


def resolve_device(
    device: str | None = None,
    *,
    host: str | None = None,
    records: list[DeviceRecord] | None = None,
) -> DeviceRecord | None:
    if host is not None and str(host).strip() and not (device and str(device).strip()):
        return None
    all_devs = discover_devices() if records is None else records
    if device and str(device).strip():
        matches = _match_devices(all_devs, str(device))
        if not matches:
            raise ValueError(f"device '{device}' not found. Available: {_format_available(all_devs) or 'none'}")
        if len(matches) > 1:
            raise ValueError(f"ambiguous device '{device}' matches: {_format_available(matches)}")
        return matches[0]
    if _SELECTED_DEVICE_ID:
        matches = _match_devices(all_devs, _SELECTED_DEVICE_ID)
        if not matches:
            raise RuntimeError(
                f"selected device '{_SELECTED_DEVICE_ID}' is not currently available; "
                f"call device_select or pass device explicitly. Available: {_format_available(all_devs) or 'none'}"
            )
        return matches[0]
    available = [r for r in all_devs if r.available]
    if len(available) > 1:
        raise RuntimeError(
            f"multiple devices available ({len(available)}): {_format_available(available)}. "
            "Specify 'device' explicitly or call 'device_select' first."
        )
    return available[0] if len(available) == 1 else None


def safe_device_key(device_key: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9._-]", "_", device_key.strip() or "default")[:40]
    digest = hashlib.sha256(device_key.encode("utf-8")).hexdigest()[:8]
    return f"{cleaned}-{digest}"


def runtime_ssh_dir() -> str:
    base = os.environ.get("XDG_RUNTIME_DIR", "").strip() or tempfile.gettempdir()
    path = os.path.join(base, f"strata-mcp-ssh-{os.getuid()}")
    os.makedirs(path, mode=0o700, exist_ok=True)
    os.chmod(path, 0o700)
    return path


def device_control_path(device_key: str) -> str:
    return os.path.join(runtime_ssh_dir(), f"ctrl-{safe_device_key(device_key)}")


def device_known_hosts_path(device_key: str) -> str:
    return os.path.join(runtime_ssh_dir(), f"known_hosts-{safe_device_key(device_key)}")


def _port_in_use(port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.05)
        return sock.connect_ex(("127.0.0.1", port)) == 0


def allocate_forward_port(
    device_key: str,
    *,
    used_ports: set[int] | None = None,
    probe_bind: bool = True,
) -> int:
    if device_key in _ALLOCATED_PORTS:
        cached = _ALLOCATED_PORTS[device_key]
        if not used_ports or cached not in used_ports:
            return cached
    blocked = set(_ALLOCATED_PORTS.values()) | (used_ports or set())
    h = int(hashlib.sha256(device_key.encode("utf-8")).hexdigest()[:8], 16)
    start = h % FORWARD_PORT_COUNT
    for offset in range(FORWARD_PORT_COUNT):
        cand = FORWARD_PORT_BASE + ((start + offset) % FORWARD_PORT_COUNT)
        if cand in blocked:
            continue
        if probe_bind and _port_in_use(cand):
            continue
        _ALLOCATED_PORTS[device_key] = cand
        return cand
    raise RuntimeError("no free local ADB forward port in range 12222..12299")


__all__ = [
    "AdbDeviceEntry",
    "DeviceRecord",
    "FORWARD_PORT_BASE",
    "FORWARD_PORT_COUNT",
    "SshHostEntry",
    "allocate_forward_port",
    "device_control_path",
    "device_known_hosts_path",
    "discover_adb_devices",
    "discover_devices",
    "get_selected_device",
    "load_devices_config",
    "load_ssh_config_entries",
    "parse_adb_devices_output",
    "parse_ssh_config_text",
    "reset_device_state",
    "resolve_device",
    "runtime_ssh_dir",
    "safe_device_key",
    "select_device",
]
