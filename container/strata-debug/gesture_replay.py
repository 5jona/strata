"""L2b pure-Python /dev/uinput multitouch Protocol B virtual touchscreen driver."""

from __future__ import annotations

import fcntl
import os
import re
import struct
import subprocess
import time
from typing import Any

# Linux input event constants (linux/input.h, linux/uinput.h)
EV_SYN = 0x00
EV_KEY = 0x01
EV_ABS = 0x03
SYN_REPORT = 0x00
BTN_TOUCH = 0x14A

ABS_MT_SLOT = 0x2F
ABS_MT_TOUCH_MAJOR = 0x30
ABS_MT_POSITION_X = 0x35
ABS_MT_POSITION_Y = 0x36
ABS_MT_TRACKING_ID = 0x39
ABS_MT_PRESSURE = 0x3A
INPUT_PROP_DIRECT = 0x01

BUS_VIRTUAL = 0x06

# ioctl numbers for aarch64 / x86_64 Linux uinput
UI_DEV_CREATE = 0x5501
UI_DEV_DESTROY = 0x5502
UI_DEV_SETUP = 0x405C5503   # _IOW('U', 3, struct uinput_setup[92])
UI_ABS_SETUP = 0x401C5504   # _IOW('U', 4, struct uinput_abs_setup[28])
UI_SET_EVBIT = 0x40045564   # _IOW('U', 100, int)
UI_SET_KEYBIT = 0x40045565  # _IOW('U', 101, int)
UI_SET_ABSBIT = 0x40045567  # _IOW('U', 103, int)
UI_SET_PROPBIT = 0x4004556E # _IOW('U', 110, int)

HOST_EXEC = "/usr/local/bin/host-exec"


def pack_input_event(ev_type: int, code: int, value: int, sec: int = 0, usec: int = 0) -> bytes:
    """Packs `struct input_event` (24 bytes on 64-bit Linux)."""
    return struct.pack("llHHi", sec, usec, ev_type, code, value)


def pack_uinput_setup(name: str = "strata-virtual-touch") -> bytes:
    """Packs `struct uinput_setup` (92 bytes)."""
    raw_name = name.encode("ascii", errors="ignore")[:79]
    return struct.pack("=HHHH80sI", BUS_VIRTUAL, 0x18D1, 0x4EE7, 1, raw_name, 0)


def pack_uinput_abs_setup(code: int, minimum: int, maximum: int, resolution: int = 0) -> bytes:
    """Packs `struct uinput_abs_setup` (28 bytes)."""
    return struct.pack("=Hxxiiiiii", code, 0, minimum, maximum, 0, 0, resolution)


def parse_wm_size(output: str) -> tuple[int, int] | None:
    matches = re.findall(r"(\d+)x(\d+)", output)
    if not matches:
        return None
    w, h = matches[-1]
    return int(w), int(h)


def parse_wm_density(output: str) -> int | None:
    matches = re.findall(r"(\d+)", output)
    return int(matches[-1]) if matches else None


def get_screen_metrics() -> tuple[int, int, int]:
    width, height, density = 1080, 2400, 420
    try:
        res = subprocess.run([HOST_EXEC, "/system/bin/wm", "size"], capture_output=True, text=True, check=False)
        parsed = parse_wm_size(res.stdout)
        if parsed:
            width, height = parsed
        dres = subprocess.run([HOST_EXEC, "/system/bin/wm", "density"], capture_output=True, text=True, check=False)
        dp = parse_wm_density(dres.stdout)
        if dp:
            density = dp
    except OSError:
        pass
    return width, height, density


def compute_edge_swipe_points(
    width: int,
    height: int,
    density_dpi: int,
    side: str = "right",
    y_percent: float = 50.0,
    distance_dp: float = 96.0,
    duration_ms: int = 240,
    cancel_back: bool = False,
) -> list[tuple[int, int, float]]:
    """Generates smooth interpolated `(x, y, dt_sec)` waypoints for an edge swipe."""
    dist_px = max(24, int(round(distance_dp * (density_dpi / 160.0))))
    y = max(1, min(height - 2, int(round(height * (y_percent / 100.0)))))
    if side == "left":
        x0, x1 = 2, min(width - 2, 2 + dist_px)
    else:
        x0, x1 = width - 2, max(2, width - 2 - dist_px)

    steps = max(8, min(60, duration_ms // 12))
    dt = (duration_ms / 1000.0) / steps
    pts: list[tuple[int, int, float]] = []
    for i in range(steps + 1):
        t = i / steps
        eased = 1.0 - (1.0 - t) ** 3
        x = int(round(x0 + (x1 - x0) * eased))
        pts.append((x, y, dt))
    if cancel_back:
        for i in range(1, steps // 2 + 1):
            t = i / max(1, steps // 2)
            x = int(round(x1 + (x0 - x1) * t))
            pts.append((x, y, dt))
    return pts


class UInputTouchDevice:
    """Context-managed `/dev/uinput` multitouch Protocol B virtual touchscreen."""

    def __init__(self, width: int, height: int, dev_path: str = "/dev/uinput") -> None:
        self.width = width
        self.height = height
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
        fcntl.ioctl(fd, UI_SET_EVBIT, EV_KEY)
        fcntl.ioctl(fd, UI_SET_EVBIT, EV_ABS)
        fcntl.ioctl(fd, UI_SET_EVBIT, EV_SYN)
        fcntl.ioctl(fd, UI_SET_KEYBIT, BTN_TOUCH)
        fcntl.ioctl(fd, UI_SET_PROPBIT, INPUT_PROP_DIRECT)

        axes = [
            (ABS_MT_SLOT, 0, 9),
            (ABS_MT_TRACKING_ID, 0, 65535),
            (ABS_MT_POSITION_X, 0, self.width - 1),
            (ABS_MT_POSITION_Y, 0, self.height - 1),
            (ABS_MT_TOUCH_MAJOR, 0, 255),
            (ABS_MT_PRESSURE, 0, 255),
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

    def _emit(self, events: list[tuple[int, int, int]]) -> None:
        assert self.fd is not None
        now = time.time()
        sec = int(now)
        usec = int((now - sec) * 1_000_000)
        payload = b"".join(pack_input_event(t, c, v, sec, usec) for t, c, v in events)
        os.write(self.fd, payload)

    def _down(self, x: int, y: int) -> None:
        tid = self._tracking_id
        self._tracking_id += 1
        self._emit([
            (EV_ABS, ABS_MT_SLOT, 0),
            (EV_ABS, ABS_MT_TRACKING_ID, tid),
            (EV_ABS, ABS_MT_POSITION_X, x),
            (EV_ABS, ABS_MT_POSITION_Y, y),
            (EV_ABS, ABS_MT_TOUCH_MAJOR, 24),
            (EV_ABS, ABS_MT_PRESSURE, 80),
            (EV_KEY, BTN_TOUCH, 1),
            (EV_SYN, SYN_REPORT, 0),
        ])

    def _move(self, x: int, y: int) -> None:
        self._emit([
            (EV_ABS, ABS_MT_SLOT, 0),
            (EV_ABS, ABS_MT_POSITION_X, x),
            (EV_ABS, ABS_MT_POSITION_Y, y),
            (EV_SYN, SYN_REPORT, 0),
        ])

    def _up(self) -> None:
        self._emit([
            (EV_ABS, ABS_MT_SLOT, 0),
            (EV_ABS, ABS_MT_TRACKING_ID, -1),
            (EV_KEY, BTN_TOUCH, 0),
            (EV_SYN, SYN_REPORT, 0),
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
        x0, y0, dt0 = points[0]
        self._down(x0, y0)
        time.sleep(dt0)
        for x, y, dt in points[1:]:
            self._move(x, y)
            time.sleep(dt)
        self._up()
