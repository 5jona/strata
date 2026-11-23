"""Bounded native Android Perfetto trace capture via host-exec."""

from __future__ import annotations

import posixpath
import re
import subprocess
import time
from typing import Any

import perfetto_analyze
from host_runner import HOST_EXEC, run_host_text

PERFETTO_BIN = "/system/bin/perfetto"

MIN_DURATION_SEC = 1
MAX_DURATION_SEC = 15
DEFAULT_DURATION_SEC = 3

MIN_BUFFER_KB = 1024
MAX_BUFFER_KB = 32768
DEFAULT_BUFFER_KB = 8192

ALLOWED_OUT_PREFIX = "/data/local/tmp/"
SAFE_PATH_RE = re.compile(r"^[A-Za-z0-9._/-]+$")

PRESETS: tuple[str, ...] = ("ui-jank", "sched", "binder", "memory")

_PRESET_SOURCES: dict[str, str] = {
    "ui-jank": """
data_sources: {
    config {
        name: "linux.ftrace"
        ftrace_config {
            atrace_categories: "gfx"
            atrace_categories: "view"
            atrace_categories: "wm"
            atrace_categories: "am"
            atrace_categories: "input"
            atrace_categories: "sched"
            ftrace_events: "sched/sched_switch"
            ftrace_events: "sched/sched_waking"
        }
    }
}
data_sources: {
    config {
        name: "android.surfaceflinger.frametimeline"
    }
}
""".strip(),
    "sched": """
data_sources: {
    config {
        name: "linux.ftrace"
        ftrace_config {
            ftrace_events: "sched/sched_switch"
            ftrace_events: "sched/sched_wakeup"
            ftrace_events: "sched/sched_waking"
            ftrace_events: "power/cpu_frequency"
            ftrace_events: "power/cpu_idle"
        }
    }
}
data_sources: {
    config {
        name: "linux.process_stats"
        process_stats_config {
            scan_all_processes_on_start: true
        }
    }
}
""".strip(),
    "binder": """
data_sources: {
    config {
        name: "linux.ftrace"
        ftrace_config {
            atrace_categories: "binder_driver"
            atrace_categories: "binder_lock"
            ftrace_events: "binder/binder_transaction"
            ftrace_events: "binder/binder_transaction_received"
            ftrace_events: "binder/binder_lock"
            ftrace_events: "binder/binder_locked"
            ftrace_events: "binder/binder_unlock"
        }
    }
}
""".strip(),
    "memory": """
data_sources: {
    config {
        name: "linux.ftrace"
        ftrace_config {
            atrace_categories: "memory"
            ftrace_events: "lowmemorykiller/lowmemory_kill"
            ftrace_events: "oom/oom_score_adj_update"
            ftrace_events: "kmem/rss_stat"
        }
    }
}
data_sources: {
    config {
        name: "linux.sys_stats"
        sys_stats_config {
            meminfo_period_ms: 500
            vmstat_period_ms: 500
        }
    }
}
data_sources: {
    config {
        name: "linux.process_stats"
        process_stats_config {
            proc_stats_poll_ms: 500
        }
    }
}
""".strip(),
}


def validate_preset(preset: str) -> str:
    if preset not in _PRESET_SOURCES:
        raise ValueError(f"unknown preset '{preset}', expected one of {list(PRESETS)}")
    return preset


def validate_duration(duration_sec: int) -> int:
    if isinstance(duration_sec, bool) or not isinstance(duration_sec, int):
        raise ValueError("duration_sec must be an integer")
    if not (MIN_DURATION_SEC <= duration_sec <= MAX_DURATION_SEC):
        raise ValueError(f"duration_sec must be between {MIN_DURATION_SEC} and {MAX_DURATION_SEC}")
    return duration_sec


def validate_buffer_kb(buffer_kb: int) -> int:
    if isinstance(buffer_kb, bool) or not isinstance(buffer_kb, int):
        raise ValueError("buffer_kb must be an integer")
    if not (MIN_BUFFER_KB <= buffer_kb <= MAX_BUFFER_KB):
        raise ValueError(f"buffer_kb must be between {MIN_BUFFER_KB} and {MAX_BUFFER_KB}")
    return buffer_kb


def validate_out_path(out_path: str | None, preset: str, *, ts: int | None = None) -> str:
    validate_preset(preset)
    if out_path is None:
        stamp = int(time.time()) if ts is None else int(ts)
        return f"{ALLOWED_OUT_PREFIX}strata-perfetto-{preset}-{stamp}.perfetto-trace"
    if not isinstance(out_path, str) or not out_path:
        raise ValueError("out_path must be a non-empty string")
    if not out_path.startswith(ALLOWED_OUT_PREFIX):
        raise ValueError(f"out_path must start with '{ALLOWED_OUT_PREFIX}'")
    if ".." in out_path or not SAFE_PATH_RE.match(out_path):
        raise ValueError("out_path contains illegal traversal or disallowed characters")
    if out_path.endswith("/"):
        raise ValueError("out_path must be a file path, not a directory")
    norm = posixpath.normpath(out_path)
    if not norm.startswith(ALLOWED_OUT_PREFIX) or norm == ALLOWED_OUT_PREFIX.rstrip("/"):
        raise ValueError(f"out_path resolves outside '{ALLOWED_OUT_PREFIX}'")
    return norm


def get_preset_sources(preset: str) -> str:
    return _PRESET_SOURCES[validate_preset(preset)]


def build_perfetto_config(
    preset: str = "ui-jank",
    duration_sec: int = DEFAULT_DURATION_SEC,
    buffer_kb: int = DEFAULT_BUFFER_KB,
) -> str:
    """Builds a bounded Perfetto textproto configuration for the given preset."""
    p = validate_preset(preset)
    dur = validate_duration(duration_sec)
    buf = validate_buffer_kb(buffer_kb)
    return (
        f"buffers: {{\n"
        f"    size_kb: {buf}\n"
        f"    fill_policy: RING_BUFFER\n"
        f"}}\n"
        f"{_PRESET_SOURCES[p]}\n"
        f"duration_ms: {dur * 1000}\n"
    )


def stat_remote_size(remote_path: str) -> int:
    try:
        res = run_host_text(["/system/bin/stat", "-c", "%s", remote_path], timeout=5.0)
        if res.returncode == 0 and res.stdout.strip().isdigit():
            return int(res.stdout.strip())
    except (OSError, subprocess.TimeoutExpired):
        pass
    return 0


def capture_perfetto(
    preset: str = "ui-jank",
    duration_sec: int = DEFAULT_DURATION_SEC,
    buffer_kb: int = DEFAULT_BUFFER_KB,
    out_path: str | None = None,
    *,
    analyze: bool = True,
) -> dict[str, Any]:
    """Captures a bounded Android Perfetto trace into `/data/local/tmp/` via host-exec."""
    try:
        p = validate_preset(preset)
        dur = validate_duration(duration_sec)
        buf = validate_buffer_kb(buffer_kb)
        remote_path = validate_out_path(out_path, p)
        config_txt = build_perfetto_config(p, dur, buf)
    except ValueError as exc:
        return {"ok": False, "error": str(exc)}

    try:
        res = run_host_text(
            [PERFETTO_BIN, "--txt", "-c", "-", "-o", remote_path],
            input_text=config_txt,
            timeout=dur + 15,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {
            "ok": False,
            "preset": p,
            "durationSec": dur,
            "bufferKb": buf,
            "remotePath": remote_path,
            "sizeBytes": 0,
            "stderr": str(exc)[-1000:],
        }

    size_bytes = stat_remote_size(remote_path) if res.returncode == 0 else 0
    ok = res.returncode == 0 and size_bytes > 0
    result: dict[str, Any] = {
        "ok": ok,
        "preset": p,
        "durationSec": dur,
        "bufferKb": buf,
        "remotePath": remote_path,
        "sizeBytes": size_bytes,
        "stderr": res.stderr.strip()[-1000:],
    }
    if ok and analyze:
        result["summary"] = perfetto_analyze.analyze_trace_file(remote_path)
    return result
