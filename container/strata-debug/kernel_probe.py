"""L5 read-only kernel observability: pstore ramoops parser and safe tracefs/bpftrace presets.

Zero out-of-tree kernel modules, zero GKI patches, zero panic risk.
"""

from __future__ import annotations

import glob
import os
import re
import shutil
import subprocess
from typing import Any

HOST_EXEC = "/usr/local/bin/host-exec"
WATCHDOG_RE = re.compile(
    r"(oplus_secure_guard|malicious_app|Kernel panic|BUG:|Unable to handle kernel|Call trace:|reboot,)",
    re.IGNORECASE,
)

BPFTRACE_PRESETS: dict[str, str] = {
    "guard-audit": (
        "tracepoint:syscalls:sys_enter_setgid,"
        "tracepoint:syscalls:sys_enter_setresgid,"
        "tracepoint:syscalls:sys_enter_execve "
        "{ @[probe, comm, uid] = count(); }"
    ),
    "net-uid": (
        "kprobe:tcp_v4_connect,kprobe:tcp_v6_connect,kprobe:tcp_retransmit_skb "
        "{ @[func, uid, comm] = count(); }"
    ),
    "sched-lat": (
        "tracepoint:sched:sched_wakeup { @q[args->pid] = nsecs; } "
        "tracepoint:sched:sched_switch /@q[args->next_pid]/ "
        "{ @us = hist((nsecs - @q[args->next_pid]) / 1000); delete(@q[args->next_pid]); }"
    ),
}


def parse_pstore_text(text: str, *, max_lines: int = 40) -> dict[str, Any]:
    lines = text.splitlines()
    hits = [ln.strip() for ln in lines if WATCHDOG_RE.search(ln)]
    tail = [ln.strip() for ln in lines[-max_lines:] if ln.strip()]
    return {
        "totalLines": len(lines),
        "watchdogHits": hits[-20:],
        "tailLines": tail[-20:],
    }


def collect_pstore(pstore_dir: str = "/sys/fs/pstore") -> dict[str, Any]:
    patterns = ("console-ramoops*", "dmesg-ramoops*")
    files: list[str] = []
    for pat in patterns:
        files.extend(sorted(glob.glob(os.path.join(pstore_dir, pat))))

    if not files and os.path.exists(HOST_EXEC):
        res = subprocess.run(
            [HOST_EXEC, "/system/bin/sh", "-c", f"ls {pstore_dir}/console-ramoops* {pstore_dir}/dmesg-ramoops* 2>/dev/null"],
            capture_output=True,
            text=True,
            check=False,
        )
        remote_files = [ln.strip() for ln in res.stdout.splitlines() if ln.strip()]
        if remote_files:
            parsed: dict[str, Any] = {}
            for rf in remote_files[:4]:
                cat = subprocess.run([HOST_EXEC, "/system/bin/cat", rf], capture_output=True, text=True, check=False)
                parsed[os.path.basename(rf)] = parse_pstore_text(cat.stdout)
            return {"ok": True, "available": True, "source": "host-exec", "files": parsed}
        return {"ok": True, "available": False, "reason": "no ramoops files in /sys/fs/pstore", "files": {}}

    parsed = {}
    for path in files[:4]:
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                parsed[os.path.basename(path)] = parse_pstore_text(f.read())
        except OSError as exc:
            parsed[os.path.basename(path)] = {"error": str(exc)}
    return {"ok": True, "available": bool(parsed), "source": "local", "files": parsed}


def probe_trace_capabilities() -> dict[str, Any]:
    return {
        "btfVmlinux": os.path.exists("/sys/kernel/btf/vmlinux"),
        "tracefs": os.path.isdir("/sys/kernel/tracing"),
        "bpftraceBin": shutil.which("bpftrace") or "",
    }


def run_trace_preset(preset: str, duration_s: int = 5) -> dict[str, Any]:
    if preset not in BPFTRACE_PRESETS:
        return {"ok": False, "error": f"unknown preset '{preset}', expected {sorted(BPFTRACE_PRESETS)}"}
    caps = probe_trace_capabilities()
    duration = max(1, min(30, duration_s))

    if caps["bpftraceBin"] and caps["tracefs"]:
        script = f"{BPFTRACE_PRESETS[preset]} interval:s:{duration} {{ exit(); }}"
        res = subprocess.run(
            [caps["bpftraceBin"], "-e", script],
            capture_output=True,
            text=True,
            timeout=duration + 10,
            check=False,
        )
        return {
            "ok": res.returncode == 0,
            "backend": "bpftrace",
            "preset": preset,
            "durationSec": duration,
            "capabilities": caps,
            "output": res.stdout.strip()[-4000:],
            "stderr": res.stderr.strip()[-1000:],
        }

    return {
        "ok": True,
        "available": False,
        "backend": "none",
        "preset": preset,
        "capabilities": caps,
        "reason": "bpftrace or /sys/kernel/tracing unavailable; install with setup-container.sh --debug-profile",
    }
