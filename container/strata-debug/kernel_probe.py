"""L5 read-only kernel observability: pstore ramoops, dmesg, tracefs status, and bpftrace presets.

Zero out-of-tree kernel modules, zero GKI patches, zero panic risk.
"""

from __future__ import annotations

import glob
import os
import re
import shlex
import shutil
import subprocess
from typing import Any

from host_runner import HOST_EXEC, run_host_text

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
    local_exists = os.path.isdir(pstore_dir)
    files: list[str] = []
    if local_exists:
        for pat in patterns:
            files.extend(sorted(glob.glob(os.path.join(pstore_dir, pat))))

    if not files and os.path.exists(HOST_EXEC):
        quoted = shlex.quote(pstore_dir)
        res = run_host_text(
            ["/system/bin/sh", "-c", f"ls {quoted}/console-ramoops* {quoted}/dmesg-ramoops* 2>/dev/null"],
            timeout=6.0,
        )
        remote_files = [ln.strip() for ln in res.stdout.splitlines() if ln.strip()]
        if remote_files:
            parsed: dict[str, Any] = {}
            for rf in remote_files[:4]:
                cat = run_host_text(["/system/bin/cat", rf], timeout=6.0)
                parsed[os.path.basename(rf)] = parse_pstore_text(cat.stdout)
            return {"ok": True, "available": True, "source": "host-exec", "files": parsed}
        return {
            "ok": True,
            "available": False,
            "reason": f"no ramoops files in {pstore_dir}",
            "files": {},
        }

    if not local_exists:
        return {
            "ok": True,
            "available": False,
            "reason": f"pstore directory not mounted or missing: {pstore_dir}",
            "files": {},
        }

    if not files:
        return {
            "ok": True,
            "available": False,
            "reason": f"no ramoops files in {pstore_dir}",
            "files": {},
        }

    parsed = {}
    for path in files[:4]:
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                parsed[os.path.basename(path)] = parse_pstore_text(f.read())
        except OSError as exc:
            parsed[os.path.basename(path)] = {"error": str(exc)}
    return {"ok": True, "available": bool(parsed), "source": "local", "files": parsed}


def collect_dmesg(
    *,
    limit: int = 80,
    filter_pattern: str | None = None,
    timeout: float = 6.0,
) -> dict[str, Any]:
    """Read-only kernel ring buffer (dmesg) collector with watchdog hit extraction."""
    if os.path.exists(HOST_EXEC):
        cmd = [HOST_EXEC, "/system/bin/dmesg"]
        source = "host-exec"
    elif shutil.which("dmesg"):
        cmd = ["dmesg"]
        source = "local"
    else:
        return {"ok": True, "available": False, "reason": "dmesg binary unavailable", "lines": []}

    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"ok": True, "available": False, "reason": f"dmesg execution failed: {exc}", "lines": []}

    if res.returncode != 0:
        reason = res.stderr.strip()[:240] or f"dmesg exited with rc={res.returncode}"
        return {"ok": True, "available": False, "source": source, "reason": reason, "lines": []}

    lines = [ln for ln in res.stdout.splitlines() if ln.strip()]
    if filter_pattern:
        try:
            rx = re.compile(filter_pattern, re.IGNORECASE)
            lines = [ln for ln in lines if rx.search(ln)]
        except re.error as exc:
            return {"ok": False, "error": f"invalid filter regex: {exc}"}

    hits = [ln.strip() for ln in lines if WATCHDOG_RE.search(ln)][-20:]
    tail = lines[-max(1, limit) :]
    return {
        "ok": True,
        "available": True,
        "source": source,
        "totalLines": len(lines),
        "watchdogHits": hits,
        "lines": tail,
    }


L5_DIAG_HINT = "Enable L5 tracefs/bpffs mounts with: strata config set-l5-diag on"


def _read_small_text(path: str, max_bytes: int = 4096) -> str | None:
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read(max_bytes).strip()
    except OSError:
        return None


def is_bpffs_mounted(bpffs_dir: str = "/sys/fs/bpf", *, mounts_path: str = "/proc/mounts") -> bool:
    """Checks whether `bpffs_dir` exists, is readable, and is mounted as a `bpf` filesystem."""
    if not os.path.isdir(bpffs_dir) or not os.access(bpffs_dir, os.R_OK):
        return False
    norm = os.path.normpath(bpffs_dir)
    try:
        with open(mounts_path, encoding="utf-8", errors="replace") as f:
            for line in f:
                parts = line.split()
                if len(parts) >= 3 and os.path.normpath(parts[1]) == norm and parts[2] == "bpf":
                    return True
        return False
    except OSError:
        return os.access(bpffs_dir, os.R_OK)


def probe_trace_capabilities(
    tracing_dir: str = "/sys/kernel/tracing",
    bpffs_dir: str = "/sys/fs/bpf",
    *,
    mounts_path: str = "/proc/mounts",
) -> dict[str, Any]:
    return {
        "btfVmlinux": os.path.exists("/sys/kernel/btf/vmlinux"),
        "tracefs": os.path.isdir(tracing_dir) and os.path.exists(os.path.join(tracing_dir, "current_tracer")),
        "bpffsMounted": is_bpffs_mounted(bpffs_dir, mounts_path=mounts_path),
        "bpftraceBin": shutil.which("bpftrace") or "",
    }


def inspect_tracing(
    tracing_dir: str = "/sys/kernel/tracing", *, limit: int = 40,
    bpffs_dir: str = "/sys/fs/bpf", mounts_path: str = "/proc/mounts",
) -> dict[str, Any]:
    """Read-only tracefs and bpffs inspection; degrades gracefully when not mounted."""
    caps = probe_trace_capabilities(tracing_dir, bpffs_dir, mounts_path=mounts_path)
    if not os.path.isdir(tracing_dir):
        return {
            "ok": True, "available": False, "bpffsMounted": caps["bpffsMounted"],
            "reason": f"tracefs not mounted at {tracing_dir}; {L5_DIAG_HINT}",
            "hint": L5_DIAG_HINT, "capabilities": caps,
        }
    current_tracer = _read_small_text(os.path.join(tracing_dir, "current_tracer"))
    tracing_on = _read_small_text(os.path.join(tracing_dir, "tracing_on"))
    if current_tracer is None and tracing_on is None:
        return {
            "ok": True, "available": False, "bpffsMounted": caps["bpffsMounted"],
            "reason": f"tracefs at {tracing_dir} is unreadable or not mounted; {L5_DIAG_HINT}",
            "hint": L5_DIAG_HINT, "capabilities": caps,
        }
    avail_raw = _read_small_text(os.path.join(tracing_dir, "available_tracers")) or ""
    trace_raw = _read_small_text(os.path.join(tracing_dir, "trace"), max_bytes=32768) or ""
    trace_lines = [ln for ln in trace_raw.splitlines() if ln and not ln.startswith("#")][-max(1, limit) :]
    result: dict[str, Any] = {
        "ok": True,
        "available": True,
        "tracingDir": tracing_dir,
        "bpffsMounted": caps["bpffsMounted"],
        "tracingOn": tracing_on == "1",
        "currentTracer": current_tracer or "",
        "availableTracers": avail_raw.split(),
        "traceTail": trace_lines,
        "capabilities": caps,
    }
    if not caps["bpffsMounted"]:
        result["hint"] = f"bpffs not mounted at {bpffs_dir}; {L5_DIAG_HINT}"
    return result


def run_trace_preset(
    preset: str,
    duration_s: int = 5,
    *,
    tracing_dir: str = "/sys/kernel/tracing",
    bpffs_dir: str = "/sys/fs/bpf",
    mounts_path: str = "/proc/mounts",
) -> dict[str, Any]:
    if preset not in BPFTRACE_PRESETS:
        return {"ok": False, "error": f"unknown preset '{preset}', expected {sorted(BPFTRACE_PRESETS)}"}
    caps = probe_trace_capabilities(tracing_dir, bpffs_dir, mounts_path=mounts_path)
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
        out: dict[str, Any] = {
            "ok": res.returncode == 0,
            "available": True,
            "backend": "bpftrace",
            "preset": preset,
            "durationSec": duration,
            "bpffsMounted": caps["bpffsMounted"],
            "capabilities": caps,
            "output": res.stdout.strip()[-4000:],
            "stderr": res.stderr.strip()[-1000:],
        }
        if not caps["bpffsMounted"]:
            out["hint"] = f"bpffs not mounted at {bpffs_dir}; {L5_DIAG_HINT}"
        return out

    hints: list[str] = []
    if not caps["tracefs"] or not caps["bpffsMounted"]:
        hints.append(L5_DIAG_HINT)
    if not caps["bpftraceBin"]:
        hints.append("Install bpftrace with: setup-container.sh --debug-profile")
    hint_msg = "; ".join(hints)
    return {
        "ok": True,
        "available": False,
        "backend": "none",
        "preset": preset,
        "bpffsMounted": caps["bpffsMounted"],
        "capabilities": caps,
        "hint": hint_msg,
        "reason": f"bpftrace or /sys/kernel/tracing unavailable; {hint_msg}",
    }
