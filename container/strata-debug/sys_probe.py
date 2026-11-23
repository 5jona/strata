"""L2a Android dumpsys (window, SurfaceFlinger, gfxinfo) and SideVeil UI inspector."""

from __future__ import annotations

import json
import re
from typing import Any

from host_runner import HOST_EXEC, run_host_text as _run_host

SIDEVEIL_CTL = "/data/adb/modules/sideveil/bin/sideveil-ctl"

WINDOW_HEADER_RE = re.compile(r"^\s*Window #\d+ Window\{[0-9a-fA-F]+\s+u\d+\s+([^}]+)\}:")
FRAME_RE = re.compile(r"mFrame=\[(\d+),(\d+)\]\[(\d+),(\d+)\]")
BOUNDS_RE = re.compile(r"\[(-?\d+(?:\.\d+)?),\s*(-?\d+(?:\.\d+)?),\s*(-?\d+(?:\.\d+)?),\s*(-?\d+(?:\.\d+)?)\]")
TYPE_RE = re.compile(r"ty=(\S+)")
LAYER_RE = re.compile(r"mBaseLayer=(\d+)")
FG_PATTERNS = (
    re.compile(r"topResumedActivity=ActivityRecord\{[^}]+\s+u\d+\s+([A-Za-z0-9._]+)/([A-Za-z0-9._/$]+)"),
    re.compile(r"(?:mResumedActivity|ResumedActivity):\s*ActivityRecord\{[^}]+\s+u\d+\s+([A-Za-z0-9._]+)/([A-Za-z0-9._/$]+)"),
    re.compile(r"mCurrentFocus=Window\{[^}]+\s+u\d+\s+([A-Za-z0-9._]+)/([A-Za-z0-9._/$]+)"),
    re.compile(r"mFocusedApp=ActivityRecord\{[^}]+\s+u\d+\s+([A-Za-z0-9._]+)/([A-Za-z0-9._/$]+)"),
)


def parse_foreground_activity(text: str) -> dict[str, Any]:
    for pat in FG_PATTERNS:
        m = pat.search(text)
        if m:
            pkg, act = m.group(1), m.group(2).rstrip("}")
            return {"package": pkg, "activity": act, "component": f"{pkg}/{act}"}
    return {"package": None, "activity": None, "component": None}


def detect_foreground_package() -> str | None:
    for cmd in (
        ["/system/bin/dumpsys", "activity", "activities"],
        ["/system/bin/dumpsys", "window", "displays"],
        ["/system/bin/dumpsys", "window", "windows"],
    ):
        res = _run_host(cmd)
        if res.returncode == 0 and res.stdout:
            pkg = parse_foreground_activity(res.stdout).get("package")
            if pkg:
                return str(pkg)
    return None


def parse_window_windows(text: str, *, visible_only: bool = True) -> list[dict[str, Any]]:
    windows: list[dict[str, Any]] = []
    cur: dict[str, Any] | None = None

    def _flush() -> None:
        if cur is None:
            return
        if not visible_only or cur.get("visible"):
            windows.append(cur)

    for line in text.splitlines():
        hdr = WINDOW_HEADER_RE.match(line)
        if hdr:
            _flush()
            name = hdr.group(1).strip()
            cur = {
                "name": name,
                "package": name.split("/")[0] if "/" in name else name,
                "visible": False,
                "frame": None,
                "layer": None,
                "type": None,
            }
            continue
        if cur is None:
            continue
        if "mViewVisibility=0x0" in line or "mHasSurface=true" in line:
            if "mHasSurface=true" in line:
                cur["hasSurface"] = True
            if "mViewVisibility=0x0" in line:
                cur["visible"] = True
        if "isOnScreen=true" in line:
            cur["visible"] = True
        if "isOnScreen=false" in line:
            cur["visible"] = False
        fm = FRAME_RE.search(line)
        if fm and cur["frame"] is None:
            x0, y0, x1, y1 = (int(g) for g in fm.groups())
            cur["frame"] = {"x": x0, "y": y0, "w": x1 - x0, "h": y1 - y0}
        tm = TYPE_RE.search(line)
        if tm and cur["type"] is None:
            cur["type"] = tm.group(1)
        lm = LAYER_RE.search(line)
        if lm and cur["layer"] is None:
            cur["layer"] = int(lm.group(1))
    _flush()
    return windows


def parse_surfaceflinger_layers(text: str, *, limit: int = 30) -> list[dict[str, Any]]:
    layers: list[dict[str, Any]] = []
    current_name: str | None = None
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("+ Layer ") or stripped.startswith("Layer "):
            m = re.search(r"\(([^)]+)\)", stripped)
            current_name = m.group(1) if m else stripped
            continue
        if current_name and ("z=" in stripped or "alpha=" in stripped) and "bounds=" in stripped:
            zm = re.search(r"z=\s*(-?\d+)", stripped)
            am = re.search(r"alpha=\s*([0-9.]+)", stripped)
            bm = BOUNDS_RE.search(stripped)
            bounds = [float(g) for g in bm.groups()] if bm else None
            layers.append({
                "name": current_name,
                "z": int(zm.group(1)) if zm else 0,
                "alpha": float(am.group(1)) if am else 1.0,
                "bounds": bounds,
            })
            current_name = None
            if len(layers) >= limit:
                break
    return layers


def parse_gfxinfo(text: str) -> dict[str, Any]:
    out: dict[str, Any] = {}
    patterns = {
        "totalFrames": r"Total frames rendered:\s*(\d+)",
        "jankyFrames": r"Janky frames:\s*(\d+)",
        "jankyPercent": r"Janky frames:\s*\d+\s*\(([0-9.]+)%\)",
        "p50Ms": r"50th percentile:\s*(\d+)ms",
        "p90Ms": r"90th percentile:\s*(\d+)ms",
        "p95Ms": r"95th percentile:\s*(\d+)ms",
        "p99Ms": r"99th percentile:\s*(\d+)ms",
    }
    for key, pat in patterns.items():
        m = re.search(pat, text)
        if m:
            raw = m.group(1)
            out[key] = float(raw) if "." in raw else int(raw)
    return out


def inspect_sideveil_ui() -> dict[str, Any] | None:
    chk = _run_host(["/system/bin/sh", "-c", f"test -x {SIDEVEIL_CTL} && echo ok"])
    if chk.stdout.strip() != "ok":
        return None
    res = _run_host([SIDEVEIL_CTL, "debug", "inspect-ui"])
    if res.returncode != 0:
        return {"ok": False, "stderr": res.stderr.strip()[:240]}
    try:
        return json.loads(res.stdout)
    except json.JSONDecodeError:
        return {"ok": True, "raw": res.stdout.strip()[:400]}


def collect_sys_probe(
    package: str = "auto",
    *,
    include_surfaceflinger: bool = True,
    include_sideveil: bool | None = None,
) -> dict[str, Any]:
    if not package or package == "auto":
        resolved_pkg = detect_foreground_package() or "unknown"
    else:
        resolved_pkg = package

    win_res = _run_host(["/system/bin/dumpsys", "window", "windows"])
    windows = parse_window_windows(win_res.stdout)
    pkg_windows = (
        [w for w in windows if resolved_pkg in w["name"] or resolved_pkg in w["package"]]
        if resolved_pkg != "unknown"
        else []
    )

    gfx: dict[str, Any] = {}
    if resolved_pkg != "unknown":
        gfx_res = _run_host(["/system/bin/dumpsys", "gfxinfo", resolved_pkg])
        gfx = parse_gfxinfo(gfx_res.stdout)

    sf_layers: list[dict[str, Any]] = []
    if include_surfaceflinger and resolved_pkg != "unknown":
        sf_res = _run_host(["/system/bin/dumpsys", "SurfaceFlinger", "--list"])
        if sf_res.returncode == 0 and sf_res.stdout:
            sf_layers = [{"name": ln.strip()} for ln in sf_res.stdout.splitlines() if resolved_pkg in ln][:20]

    probe_sideveil = (
        resolved_pkg == "pochita.sideveil.host" if include_sideveil is None else bool(include_sideveil)
    )

    return {
        "ok": True,
        "package": resolved_pkg,
        "visibleWindowCount": len(windows),
        "packageWindows": pkg_windows,
        "gfxinfo": gfx,
        "surfaceFlingerMatches": sf_layers,
        "sideveilInspect": inspect_sideveil_ui() if probe_sideveil else None,
    }
