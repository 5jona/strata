"""Android application lifecycle, foreground inspection, and package diagnostics."""

from __future__ import annotations

import re
import subprocess
from typing import Any

import sys_probe

HOST_EXEC = "/usr/local/bin/host-exec"


def _run_host(args: list[str], timeout: float = 15.0) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [HOST_EXEC, *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        check=False,
    )


def parse_dumpsys_package(text: str) -> dict[str, Any]:
    """Extracts version, SDK levels, UID, and APK paths from `dumpsys package` output."""
    out: dict[str, Any] = {}
    str_fields = {
        "versionName": r"versionName=(\S+)",
        "codePath": r"codePath=(\S+)",
    }
    int_fields = {
        "versionCode": r"versionCode=(\d+)",
        "targetSdk": r"targetSdk=(\d+)",
        "minSdk": r"minSdk=(\d+)",
        "uid": r"(?:userId|appId)=(\d+)",
    }
    for key, pat in str_fields.items():
        m = re.search(pat, text)
        if m:
            out[key] = m.group(1)
    for key, pat in int_fields.items():
        m = re.search(pat, text)
        if m:
            out[key] = int(m.group(1))
    if "uid" in out:
        out["userId"] = out["uid"]

    apk_paths: list[str] = []
    for pat in (r"codePath=(\S+)", r"resourcePath=(\S+)", r"path:\s*(\S+\.apk)"):
        for match in re.findall(pat, text):
            if match not in apk_paths:
                apk_paths.append(match)
    out["apkPaths"] = apk_paths
    return out


def parse_meminfo_summary(text: str) -> dict[str, Any]:
    """Extracts total PSS, Java heap, and native heap (in KB) from `dumpsys meminfo`."""
    out: dict[str, Any] = {}
    total_m = re.search(r"TOTAL PSS:\s*(\d+)", text) or re.search(r"^\s*TOTAL\s+(\d+)", text, re.MULTILINE)
    if total_m:
        out["totalPssKb"] = int(total_m.group(1))
    java_m = re.search(r"Java Heap:\s*(\d+)", text)
    if java_m:
        out["javaHeapKb"] = int(java_m.group(1))
    native_m = re.search(r"Native Heap:\s*(\d+)", text)
    if native_m:
        out["nativeHeapKb"] = int(native_m.group(1))
    return out


def _read_uid_for_pid(pid: int) -> int | None:
    try:
        with open(f"/proc/{pid}/status", encoding="utf-8", errors="replace") as f:
            for line in f:
                if line.startswith("Uid:"):
                    parts = line.split()
                    if len(parts) >= 2 and parts[1].isdigit():
                        return int(parts[1])
    except OSError:
        return None
    return None


def _get_pids(package: str) -> list[int]:
    res = _run_host(["/system/bin/pidof", package], timeout=5.0)
    if res.returncode != 0 or not res.stdout.strip():
        return []
    return [int(tok) for tok in res.stdout.split() if tok.isdigit()]


def get_top_app() -> dict[str, Any]:
    """Returns foreground package, activity, component, PID, and UID."""
    fg: dict[str, Any] = {"package": None, "activity": None, "component": None}
    for cmd in (
        ["/system/bin/dumpsys", "activity", "activities"],
        ["/system/bin/dumpsys", "window", "displays"],
        ["/system/bin/dumpsys", "window", "windows"],
    ):
        res = _run_host(cmd, timeout=8.0)
        if res.returncode == 0 and res.stdout:
            parsed = sys_probe.parse_foreground_activity(res.stdout)
            if parsed.get("package"):
                fg = parsed
                break

    pkg = fg.get("package")
    pid: int | None = None
    uid: int | None = None
    if pkg:
        pids = _get_pids(pkg)
        pid = pids[0] if pids else None
        if pid is not None:
            uid = _read_uid_for_pid(pid)
        if uid is None:
            pkg_res = _run_host(["/system/bin/dumpsys", "package", pkg], timeout=8.0)
            if pkg_res.returncode == 0:
                uid = parse_dumpsys_package(pkg_res.stdout).get("uid")

    return {
        "ok": bool(pkg),
        "package": pkg,
        "activity": fg.get("activity"),
        "component": fg.get("component"),
        "pid": pid,
        "uid": uid,
    }


def inspect_app(package: str = "auto") -> dict[str, Any]:
    """Collects package metadata, PIDs, memory summary, and gfxinfo for an app."""
    resolved = package
    if not resolved or resolved == "auto":
        top = get_top_app()
        resolved = top.get("package") or ""
        if not resolved:
            return {"ok": False, "error": "unable to detect foreground package"}

    pids = _get_pids(resolved)
    pkg_res = _run_host(["/system/bin/dumpsys", "package", resolved], timeout=10.0)
    mem_res = _run_host(["/system/bin/dumpsys", "meminfo", resolved], timeout=10.0)
    gfx_res = _run_host(["/system/bin/dumpsys", "gfxinfo", resolved], timeout=10.0)

    return {
        "ok": True,
        "package": resolved,
        "pid": pids[0] if pids else None,
        "pids": pids,
        "packageInfo": parse_dumpsys_package(pkg_res.stdout),
        "meminfo": parse_meminfo_summary(mem_res.stdout),
        "gfxinfo": sys_probe.parse_gfxinfo(gfx_res.stdout),
    }


def start_app(
    package: str,
    *,
    activity: str | None = None,
    action: str | None = None,
    data_uri: str | None = None,
) -> dict[str, Any]:
    """Starts an Android app by explicit component, intent action/data, or launcher activity."""
    cmd: list[str] = ["/system/bin/am", "start"]
    method = "am-start"
    component: str | None = None

    if activity:
        component = activity if "/" in activity else f"{package}/{activity}"
        cmd.extend(["-n", component])
        if action:
            cmd.extend(["-a", action])
        if data_uri:
            cmd.extend(["-d", data_uri])
    elif action or data_uri:
        cmd.extend(["-p", package])
        if action:
            cmd.extend(["-a", action])
        if data_uri:
            cmd.extend(["-d", data_uri])
    else:
        res_act = _run_host(
            ["/system/bin/cmd", "package", "resolve-activity", "--brief", package],
            timeout=8.0,
        )
        for line in reversed(res_act.stdout.splitlines()):
            cand = line.strip()
            if "/" in cand and not cand.startswith("priority="):
                component = cand
                break
        if component:
            cmd.extend(["-n", component])
        else:
            method = "monkey"
            cmd = [
                "/system/bin/monkey",
                "-p",
                package,
                "-c",
                "android.intent.category.LAUNCHER",
                "1",
            ]

    res = _run_host(cmd, timeout=15.0)
    return {
        "ok": res.returncode == 0,
        "package": package,
        "method": method,
        "component": component,
        "rc": res.returncode,
        "stdout": res.stdout.strip()[:400],
        "stderr": res.stderr.strip()[:240],
    }


def stop_app(package: str) -> dict[str, Any]:
    """Force-stops the target Android package."""
    res = _run_host(["/system/bin/am", "force-stop", package], timeout=10.0)
    return {
        "ok": res.returncode == 0,
        "package": package,
        "rc": res.returncode,
        "stderr": res.stderr.strip()[:240],
    }


def clear_app(package: str) -> dict[str, Any]:
    """Clears app data and cache for the target Android package."""
    res = _run_host(["/system/bin/pm", "clear", package], timeout=15.0)
    return {
        "ok": res.returncode == 0,
        "package": package,
        "rc": res.returncode,
        "stdout": res.stdout.strip()[:240],
        "stderr": res.stderr.strip()[:240],
    }


def install_apk(
    apk_path: str,
    *,
    replace: bool = True,
    grant: bool = True,
    downgrade: bool = False,
) -> dict[str, Any]:
    """Installs an APK on the Android host via `pm install`."""
    cmd = ["/system/bin/pm", "install"]
    if replace:
        cmd.append("-r")
    if grant:
        cmd.append("-g")
    if downgrade:
        cmd.append("-d")
    cmd.append(apk_path)
    res = _run_host(cmd, timeout=60.0)
    return {
        "ok": res.returncode == 0 and "Failure" not in res.stdout,
        "apkPath": apk_path,
        "rc": res.returncode,
        "stdout": res.stdout.strip()[:400],
        "stderr": res.stderr.strip()[:240],
    }
