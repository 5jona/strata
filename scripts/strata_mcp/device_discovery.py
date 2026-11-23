"""Multi-source device discovery (adb, ~/.ssh/config, devices.json) with strict identity correlation."""

from __future__ import annotations

from dataclasses import dataclass, field
import ipaddress
import json
import os
import subprocess
from typing import Any


@dataclass
class AdbDeviceEntry:
    serial: str
    state: str
    transport: str  # "usb" | "tcpip"
    product: str | None = None
    model: str | None = None
    device: str | None = None
    transport_id: str | None = None


@dataclass
class SshHostEntry:
    host: str
    hostname: str | None = None
    port: int | None = None
    user: str | None = None


@dataclass
class DeviceRecord:
    id: str
    alias: str | None = None
    model: str | None = None
    product: str | None = None
    adb_serial: str | None = None
    adb_state: str | None = None
    adb_transports: list[str] = field(default_factory=list)
    adb_usb_serial: str | None = None
    adb_tcpip_serial: str | None = None
    ssh_host: str | None = None
    ssh_hostname: str | None = None
    ssh_port: int | None = None
    ssh_user: str | None = None
    prefer_transport: str = "ssh"
    sources: list[str] = field(default_factory=list)

    @property
    def available(self) -> bool:
        return self.adb_state == "device" or bool(self.ssh_host)

    @property
    def transport_summary(self) -> str:
        parts: list[str] = []
        if self.adb_usb_serial and self.adb_state == "device":
            parts.append("usb")
        if self.adb_tcpip_serial and self.adb_state == "device":
            parts.append("adb-tcpip")
        if self.ssh_host:
            parts.append("ssh")
        return "+".join(parts) if parts else (self.adb_state or "offline")

    def to_dict(self, selected_id: str | None = None) -> dict[str, Any]:
        return {
            "id": self.id,
            "alias": self.alias,
            "model": self.model,
            "serial": self.adb_serial,
            "usb": bool(self.adb_usb_serial and self.adb_state == "device"),
            "wireless": bool(self.ssh_host or (self.adb_tcpip_serial and self.adb_state == "device")),
            "adbState": self.adb_state,
            "adbTransports": list(self.adb_transports),
            "sshHost": self.ssh_host,
            "sshPort": self.ssh_port,
            "sshUser": self.ssh_user,
            "preferTransport": self.prefer_transport,
            "selected": self.id == selected_id,
            "sources": list(self.sources),
        }


def _is_ip(value: str | None) -> bool:
    if not value:
        return False
    try:
        ipaddress.ip_address(value.strip("[]"))
        return True
    except ValueError:
        return False


def _tcpip_ip(serial: str | None) -> str | None:
    if not serial or ":" not in serial:
        return None
    host = serial.rsplit(":", 1)[0].strip("[]")
    return host if _is_ip(host) else None


def parse_adb_devices_output(text: str) -> list[AdbDeviceEntry]:
    entries: list[AdbDeviceEntry] = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("List of devices") or line.startswith("*"):
            continue
        parts = line.split()
        if len(parts) < 2:
            continue
        serial, state = parts[0], parts[1]
        kv = dict(tok.split(":", 1) for tok in parts[2:] if ":" in tok)
        transport = "tcpip" if (":" in serial or "._adb-tls-" in serial) else "usb"
        entries.append(AdbDeviceEntry(
            serial=serial, state=state, transport=transport,
            product=kv.get("product"), model=kv.get("model"),
            device=kv.get("device"), transport_id=kv.get("transport_id"),
        ))
    return entries


def discover_adb_devices() -> list[AdbDeviceEntry]:
    try:
        res = subprocess.run(["adb", "devices", "-l"], capture_output=True, text=True, timeout=5.0, check=False)
    except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
        return []
    return parse_adb_devices_output(res.stdout) if res.returncode == 0 else []


def parse_ssh_config_text(text: str, *, extra_hosts: set[str] | None = None) -> list[SshHostEntry]:
    extra = {h.lower() for h in (extra_hosts or set()) if h}
    env_host = os.environ.get("STRATA_SSH_HOST", "").strip().lower()
    if env_host:
        extra.add(env_host)
    blocks: list[tuple[list[str], dict[str, str]]] = []
    cur_hosts: list[str] = []
    cur_opts: dict[str, str] = {}
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split(None, 1)
        if len(parts) < 2:
            continue
        key, val = parts[0].lower(), parts[1].strip()
        if key == "host":
            if cur_hosts:
                blocks.append((cur_hosts, cur_opts))
            cur_hosts = [h for h in val.split() if not any(ch in h for ch in "*?!")]
            cur_opts = {}
        elif cur_hosts and key not in cur_opts:
            cur_opts[key] = val
    if cur_hosts:
        blocks.append((cur_hosts, cur_opts))

    entries: list[SshHostEntry] = []
    seen: set[str] = set()
    for hosts, opts in blocks:
        port_raw = opts.get("port")
        port = int(port_raw) if port_raw and port_raw.isdigit() else None
        hostname, user = opts.get("hostname"), opts.get("user")
        for host in hosts:
            low, hname_low = host.lower(), (hostname or "").lower()
            relevant = (
                port == 2222 or "strata" in low or "android" in low
                or "strata" in hname_low or "android" in hname_low or low in extra
            )
            if relevant and host not in seen:
                seen.add(host)
                entries.append(SshHostEntry(host=host, hostname=hostname, port=port, user=user))
    return entries


def load_ssh_config_entries(path: str | None = None, *, extra_hosts: set[str] | None = None) -> list[SshHostEntry]:
    cfg_path = path or os.environ.get("STRATA_SSH_CONFIG_PATH") or os.path.expanduser("~/.ssh/config")
    if not os.path.isfile(cfg_path):
        return []
    try:
        with open(cfg_path, encoding="utf-8") as f:
            return parse_ssh_config_text(f.read(), extra_hosts=extra_hosts)
    except OSError:
        return []


def load_devices_config(path: str | None = None) -> dict[str, dict[str, Any]]:
    cfg_path = path or os.environ.get("STRATA_DEVICES_CONFIG") or os.path.expanduser("~/.config/strata/devices.json")
    if not os.path.isfile(cfg_path):
        return {}
    try:
        with open(cfg_path, encoding="utf-8") as f:
            raw = json.load(f)
    except (OSError, json.JSONDecodeError):
        return {}
    if isinstance(raw, dict) and isinstance(raw.get("devices"), dict):
        raw = raw["devices"]
    if not isinstance(raw, dict):
        return {}
    return {str(k): v for k, v in raw.items() if isinstance(v, dict)}


def _add_unique(items: list[str], value: str) -> None:
    if value not in items:
        items.append(value)


def discover_devices(
    *,
    adb_entries: list[AdbDeviceEntry] | None = None,
    ssh_entries: list[SshHostEntry] | None = None,
    config_map: dict[str, dict[str, Any]] | None = None,
) -> list[DeviceRecord]:
    cfg = load_devices_config() if config_map is None else config_map
    configured_ssh = {str(v.get("sshHost", "")).strip() for v in cfg.values() if v.get("sshHost")}
    adb_list = discover_adb_devices() if adb_entries is None else adb_entries
    ssh_list = load_ssh_config_entries(extra_hosts=configured_ssh) if ssh_entries is None else ssh_entries

    records: list[DeviceRecord] = []
    by_serial: dict[str, DeviceRecord] = {}
    by_ssh_host: dict[str, DeviceRecord] = {}

    for alias, item in cfg.items():
        serial = str(item.get("adbSerial") or "").strip() or None
        ssh_host = str(item.get("sshHost") or "").strip() or None
        ssh_port = int(item["sshPort"]) if item.get("sshPort") is not None else None
        ssh_user = str(item.get("sshUser") or "").strip() or None
        prefer = str(item.get("preferTransport") or "ssh").strip().lower()
        rec = DeviceRecord(
            id=alias, alias=alias, model=str(item.get("model") or "").strip() or None,
            adb_serial=serial, ssh_host=ssh_host, ssh_port=ssh_port, ssh_user=ssh_user,
            prefer_transport=prefer if prefer in {"ssh", "adb"} else "ssh", sources=["devices_json"],
        )
        records.append(rec)
        if serial:
            by_serial[serial] = rec
        if ssh_host:
            by_ssh_host[ssh_host] = rec

    for adb in adb_list:
        rec = by_serial.get(adb.serial)
        if rec is None:
            rec = DeviceRecord(id=adb.serial, adb_serial=adb.serial, model=adb.model, product=adb.product)
            records.append(rec)
            by_serial[adb.serial] = rec
        else:
            rec.model = rec.model or adb.model
            rec.product = rec.product or adb.product
        _add_unique(rec.sources, "adb")
        _add_unique(rec.adb_transports, adb.transport)
        if adb.transport == "usb":
            rec.adb_usb_serial = adb.serial
        else:
            rec.adb_tcpip_serial = adb.serial
        if rec.adb_state != "device" or adb.state == "device":
            rec.adb_state = adb.state

    for ssh in ssh_list:
        rec = by_ssh_host.get(ssh.host)
        if rec is None and _is_ip(ssh.hostname):
            rec = next((c for c in records if c.adb_tcpip_serial and _tcpip_ip(c.adb_tcpip_serial) == ssh.hostname), None)
        if rec is None:
            rec = DeviceRecord(
                id=ssh.host, alias=ssh.host, ssh_host=ssh.host,
                ssh_hostname=ssh.hostname, ssh_port=ssh.port, ssh_user=ssh.user,
            )
            records.append(rec)
            by_ssh_host[ssh.host] = rec
        else:
            rec.ssh_host = rec.ssh_host or ssh.host
            rec.ssh_hostname = rec.ssh_hostname or ssh.hostname
            rec.ssh_port = rec.ssh_port or ssh.port
            rec.ssh_user = rec.ssh_user or ssh.user
            by_ssh_host[ssh.host] = rec
        _add_unique(rec.sources, "ssh_config")

    return records
