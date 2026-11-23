"""L2b pure-Python /dev/uinput multitouch Protocol B virtual touchscreen driver."""

from __future__ import annotations

import fcntl
import os
import re
import struct
import time
from typing import Any

from host_runner import HOST_EXEC, run_host_stdout as _run_host_stdout

# Linux input / uinput constants (linux/input.h, linux/uinput.h, 64-bit aarch64/x86_64)
EV_SYN, EV_KEY, EV_ABS = 0x00, 0x01, 0x03
SYN_REPORT, BTN_TOUCH, INPUT_PROP_DIRECT, BUS_VIRTUAL = 0x00, 0x14A, 0x01, 0x06
ABS_MT_SLOT, ABS_MT_TOUCH_MAJOR = 0x2F, 0x30
ABS_MT_POSITION_X, ABS_MT_POSITION_Y = 0x35, 0x36
ABS_MT_TRACKING_ID, ABS_MT_PRESSURE = 0x39, 0x3A

UI_DEV_CREATE, UI_DEV_DESTROY = 0x5501, 0x5502
UI_DEV_SETUP, UI_ABS_SETUP = 0x405C5503, 0x401C5504
UI_SET_EVBIT, UI_SET_KEYBIT = 0x40045564, 0x40045565
UI_SET_ABSBIT, UI_SET_PROPBIT = 0x40045567, 0x4004556E


def pack_input_event(ev_type: int, code: int, value: int, sec: int = 0, usec: int = 0) -> bytes:
    """Packs `struct input_event` (24 bytes on 64-bit Linux)."""
    return struct.pack("llHHi", sec, usec, ev_type, code, value)


def pack_uinput_setup(name: str = "strata-virtual-touch") -> bytes:
    """Packs `struct uinput_setup` (92 bytes)."""
    return struct.pack("=HHHH80sI", BUS_VIRTUAL, 0x18D1, 0x4EE7, 1, name.encode("ascii", "ignore")[:79], 0)


def pack_uinput_abs_setup(code: int, minimum: int, maximum: int, resolution: int = 0) -> bytes:
    """Packs `struct uinput_abs_setup` (28 bytes)."""
    return struct.pack("=Hxxiiiiii", code, 0, minimum, maximum, 0, 0, resolution)


def parse_wm_size(output: str) -> tuple[int, int] | None:
    m = re.findall(r"(\d+)x(\d+)", output)
    return (int(m[-1][0]), int(m[-1][1])) if m else None


def parse_wm_physical_size(output: str) -> tuple[int, int] | None:
    m = re.search(r"Physical\s+size:\s*(\d+)x(\d+)", output)
    return (int(m.group(1)), int(m.group(2))) if m else parse_wm_size(output)


def parse_wm_density(output: str) -> int | None:
    m = re.findall(r"(\d+)", output)
    return int(m[-1]) if m else None


def parse_dumpsys_input_viewport(output: str) -> dict[str, int] | None:
    """Parses `Viewport INTERNAL` from `dumpsys input` for orientation and logical/physical sizes."""
    candidates: list[tuple[int, dict[str, int]]] = []
    for line in output.splitlines():
        if "Viewport INTERNAL" not in line:
            continue
        m_log = re.search(r"logicalFrame=\[(\d+),\s*(\d+),\s*(\d+),\s*(\d+)\]", line)
        if not m_log:
            continue
        m_rot = re.search(r"orientation=(\d+)", line)
        rot = int(m_rot.group(1)) % 4 if m_rot else 0
        lw, lh = int(m_log.group(3)) - int(m_log.group(1)), int(m_log.group(4)) - int(m_log.group(2))
        if lw <= 0 or lh <= 0:
            continue
        m_dev = re.search(r"deviceSize=\[(\d+),\s*(\d+)\]", line)
        m_phys = re.search(r"physicalFrame=\[(\d+),\s*(\d+),\s*(\d+),\s*(\d+)\]", line)
        if m_dev:
            pw, ph = int(m_dev.group(1)), int(m_dev.group(2))
        elif m_phys:
            pw, ph = int(m_phys.group(3)) - int(m_phys.group(1)), int(m_phys.group(4)) - int(m_phys.group(2))
        else:
            pw, ph = (lh, lw) if rot in (1, 3) else (lw, lh)
        if pw <= 0 or ph <= 0:
            continue
        score = (2 if "displayId=0" in line else 0) + (1 if "isActive=[1]" in line or "isActive=true" in line else 0)
        candidates.append((score, {"logical_w": lw, "logical_h": lh, "phys_w": pw, "phys_h": ph, "rotation": rot}))
    if not candidates:
        return None
    candidates.sort(key=lambda item: item[0], reverse=True)
    return candidates[0][1]


def parse_dumpsys_display_info(output: str) -> dict[str, int] | None:
    """Parses `mOverrideDisplayInfo` / `mBaseDisplayInfo` from `dumpsys display`."""
    m_ov = re.search(r"mOverrideDisplayInfo=DisplayInfo\{[^\n]*?real\s+(\d+)\s*x\s*(\d+)[^\n]*?rotation\s+(\d+)", output)
    if not m_ov:
        return None
    lw, lh, rot = int(m_ov.group(1)), int(m_ov.group(2)), int(m_ov.group(3)) % 4
    if lw <= 0 or lh <= 0:
        return None
    m_base = re.search(r"mBaseDisplayInfo=DisplayInfo\{[^\n]*?real\s+(\d+)\s*x\s*(\d+)", output)
    pw, ph = (int(m_base.group(1)), int(m_base.group(2))) if m_base else ((lh, lw) if rot in (1, 3) else (lw, lh))
    return {"logical_w": lw, "logical_h": lh, "phys_w": pw, "phys_h": ph, "rotation": rot}


def logical_to_physical(
    x: int, y: int, logical_w: int, logical_h: int, phys_w: int, phys_h: int, rotation: int
) -> tuple[int, int]:
    """Maps logical display coordinates `(x, y)` to physical touch panel coordinates `(px, py)`.

    Inverse of Android `TouchInputMapper` CCW display rotation (`Surface.ROTATION_*`):
    - rotation 0 (0 deg):   `px = x`,                `py = y`
    - rotation 1 (90 CCW):  `px = phys_w - 1 - y`,   `py = x`
    - rotation 2 (180 deg): `px = phys_w - 1 - x`,   `py = phys_h - 1 - y`
    - rotation 3 (270 CCW): `px = y`,                `py = phys_h - 1 - x`
    Out-of-range coordinates are clamped to `[0, logical - 1]` and `[0, phys - 1]`.
    """
    lw, lh = max(1, int(logical_w)), max(1, int(logical_h))
    pw, ph = max(1, int(phys_w)), max(1, int(phys_h))
    rot = int(rotation) % 4
    cx, cy = max(0, min(lw - 1, int(x))), max(0, min(lh - 1, int(y)))

    def _scale(val: int, src: int, dst: int) -> int:
        if src == dst or src <= 1 or dst <= 1:
            return min(val, dst - 1)
        return int(round(val * (dst - 1) / (src - 1)))

    if rot == 0:
        px, py = _scale(cx, lw, pw), _scale(cy, lh, ph)
    elif rot == 1:
        px, py = (pw - 1) - _scale(cy, lh, pw), _scale(cx, lw, ph)
    elif rot == 2:
        px, py = (pw - 1) - _scale(cx, lw, pw), (ph - 1) - _scale(cy, lh, ph)
    else:
        px, py = _scale(cy, lh, pw), (ph - 1) - _scale(cx, lw, ph)
    return max(0, min(pw - 1, px)), max(0, min(ph - 1, py))


def get_display_geometry() -> dict[str, int]:
    """Detects logical/physical display dimensions, rotation (0..3), and DPI."""
    geom = {"logical_w": 1080, "logical_h": 2400, "phys_w": 1080, "phys_h": 2400, "rotation": 0, "dpi": 420}
    parsed = parse_dumpsys_input_viewport(_run_host_stdout(["/system/bin/dumpsys", "input"]))
    if not parsed:
        parsed = parse_dumpsys_display_info(_run_host_stdout(["/system/bin/dumpsys", "display"]))
    wm_out = _run_host_stdout(["/system/bin/wm", "size"])
    sz, psz = parse_wm_size(wm_out), parse_wm_physical_size(wm_out)
    if parsed:
        geom.update(parsed)
    elif sz:
        geom["logical_w"], geom["logical_h"] = sz
    if psz or (not parsed and sz):
        geom["phys_w"], geom["phys_h"] = psz or sz  # type: ignore[assignment]
    dp = parse_wm_density(_run_host_stdout(["/system/bin/wm", "density"]))
    if dp:
        geom["dpi"] = dp
    return geom


def get_screen_metrics() -> tuple[int, int, int]:
    geom = get_display_geometry()
    return geom["logical_w"], geom["logical_h"], geom["dpi"]


def compute_edge_swipe_points(
    width: int, height: int, density_dpi: int, side: str = "right",
    y_percent: float = 50.0, distance_dp: float = 96.0, duration_ms: int = 240, cancel_back: bool = False,
) -> list[tuple[int, int, float]]:
    """Generates smooth interpolated logical `(x, y, dt_sec)` waypoints for an edge swipe."""
    dist_px = max(24, int(round(distance_dp * (density_dpi / 160.0))))
    y = max(1, min(height - 2, int(round(height * (y_percent / 100.0)))))
    x0, x1 = (2, min(width - 2, 2 + dist_px)) if side == "left" else (width - 2, max(2, width - 2 - dist_px))

    steps = max(8, min(60, duration_ms // 12))
    dt = (duration_ms / 1000.0) / steps
    pts: list[tuple[int, int, float]] = []
    for i in range(steps + 1):
        t = i / steps
        pts.append((int(round(x0 + (x1 - x0) * (1.0 - (1.0 - t) ** 3))), y, dt))
    if cancel_back:
        for i in range(1, steps // 2 + 1):
            t = i / max(1, steps // 2)
            pts.append((int(round(x1 + (x0 - x1) * t)), y, dt))
    return pts


class UInputTouchDevice:
    """Context-managed `/dev/uinput` multitouch Protocol B virtual touchscreen."""

    def __init__(
        self, width: int, height: int, dev_path: str = "/dev/uinput", *,
        logical_width: int | None = None, logical_height: int | None = None, rotation: int = 0,
    ) -> None:
        self.width = self.phys_width = width
        self.height = self.phys_height = height
        self.rotation = int(rotation) % 4
        self.logical_width = logical_width or (height if self.rotation in (1, 3) else width)
        self.logical_height = logical_height or (width if self.rotation in (1, 3) else height)
        self.dev_path = dev_path
        self.fd: int | None = None
        self._tracking_id = 1

    def __enter__(self) -> UInputTouchDevice:
        if not os.path.exists(self.dev_path):
            raise FileNotFoundError(
                f"{self.dev_path} not present; enable extraCharDevs=['uinput'] in Strata config"
            )
        fd = os.open(self.dev_path, os.O_WRONLY | os.O_NONBLOCK)
        self.fd = fd
        for req, val in (
            (UI_SET_EVBIT, EV_KEY), (UI_SET_EVBIT, EV_ABS), (UI_SET_EVBIT, EV_SYN),
            (UI_SET_KEYBIT, BTN_TOUCH), (UI_SET_PROPBIT, INPUT_PROP_DIRECT),
        ):
            fcntl.ioctl(fd, req, val)
        axes = [
            (ABS_MT_SLOT, 0, 9), (ABS_MT_TRACKING_ID, 0, 65535),
            (ABS_MT_POSITION_X, 0, self.phys_width - 1), (ABS_MT_POSITION_Y, 0, self.phys_height - 1),
            (ABS_MT_TOUCH_MAJOR, 0, 255), (ABS_MT_PRESSURE, 0, 255),
        ]
        for code, lo, hi in axes:
            fcntl.ioctl(fd, UI_SET_ABSBIT, code)
            fcntl.ioctl(fd, UI_ABS_SETUP, pack_uinput_abs_setup(code, lo, hi))
        fcntl.ioctl(fd, UI_DEV_SETUP, pack_uinput_setup())
        fcntl.ioctl(fd, UI_DEV_CREATE)
        time.sleep(0.08)
        return self

    def __exit__(self, *_exc: Any) -> None:
        if self.fd is not None:
            try:
                fcntl.ioctl(self.fd, UI_DEV_DESTROY)
            except OSError:
                pass
            os.close(self.fd)
            self.fd = None

    def _map(self, x: int, y: int) -> tuple[int, int]:
        return logical_to_physical(
            x, y, self.logical_width, self.logical_height, self.phys_width, self.phys_height, self.rotation
        )

    def _emit(self, events: list[tuple[int, int, int]]) -> None:
        assert self.fd is not None
        now = time.time()
        sec, usec = int(now), int((now - int(now)) * 1_000_000)
        os.write(self.fd, b"".join(pack_input_event(t, c, v, sec, usec) for t, c, v in events))

    def _down(self, x: int, y: int) -> None:
        px, py = self._map(x, y)
        tid = self._tracking_id
        self._tracking_id += 1
        self._emit([
            (EV_ABS, ABS_MT_SLOT, 0), (EV_ABS, ABS_MT_TRACKING_ID, tid),
            (EV_ABS, ABS_MT_POSITION_X, px), (EV_ABS, ABS_MT_POSITION_Y, py),
            (EV_ABS, ABS_MT_TOUCH_MAJOR, 24), (EV_ABS, ABS_MT_PRESSURE, 80),
            (EV_KEY, BTN_TOUCH, 1), (EV_SYN, SYN_REPORT, 0),
        ])

    def _move(self, x: int, y: int) -> None:
        px, py = self._map(x, y)
        self._emit([
            (EV_ABS, ABS_MT_SLOT, 0), (EV_ABS, ABS_MT_POSITION_X, px),
            (EV_ABS, ABS_MT_POSITION_Y, py), (EV_SYN, SYN_REPORT, 0),
        ])

    def _up(self) -> None:
        self._emit([
            (EV_ABS, ABS_MT_SLOT, 0), (EV_ABS, ABS_MT_TRACKING_ID, -1),
            (EV_KEY, BTN_TOUCH, 0), (EV_SYN, SYN_REPORT, 0),
        ])

    def tap(self, x: int, y: int, hold_ms: int = 35) -> None:
        self._down(x, y)
        time.sleep(hold_ms / 1000.0)
        self._up()

    def swipe(self, x0: int, y0: int, x1: int, y1: int, duration_ms: int = 240) -> None:
        steps = max(8, min(60, duration_ms // 12))
        dt = (duration_ms / 1000.0) / steps
        self._down(x0, y0)
        for i in range(1, steps + 1):
            t = i / steps
            self._move(int(round(x0 + (x1 - x0) * t)), int(round(y0 + (y1 - y0) * t)))
            time.sleep(dt)
        self._up()

    def replay_points(self, points: list[tuple[int, int, float]]) -> None:
        if not points:
            return
        self._down(points[0][0], points[0][1])
        time.sleep(points[0][2])
        for x, y, dt in points[1:]:
            self._move(x, y)
            time.sleep(dt)
        self._up()
