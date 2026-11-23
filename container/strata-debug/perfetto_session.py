"""Background Perfetto ring-buffer session manager (start/status/snapshot/stop)."""

from __future__ import annotations

import json
import os
import re
import subprocess
import time
from typing import Any

import perfetto_analyze
import perfetto_ops
from host_runner import HOST_EXEC, run_host_text as _run_host

PERFETTO_BIN = perfetto_ops.PERFETTO_BIN

DEFAULT_SESSION_KEY = "strata_ring"
SESSION_KEY_RE = re.compile(r"^[A-Za-z0-9_-]{1,32}$")

MIN_RING_DURATION_SEC, MAX_RING_DURATION_SEC = 60, 3600
DEFAULT_RING_DURATION_SEC = 1800
DEFAULT_RING_BUFFER_KB = 16384


def validate_session_key(session_key: str) -> str:
    if not isinstance(session_key, str) or not SESSION_KEY_RE.match(session_key):
        raise ValueError("session_key must match ^[A-Za-z0-9_-]{1,32}$")
    return session_key


def validate_max_duration(max_duration_sec: int) -> int:
    if isinstance(max_duration_sec, bool) or not isinstance(max_duration_sec, int):
        raise ValueError("max_duration_sec must be an integer")
    if not (MIN_RING_DURATION_SEC <= max_duration_sec <= MAX_RING_DURATION_SEC):
        raise ValueError(f"max_duration_sec must be between {MIN_RING_DURATION_SEC} and {MAX_RING_DURATION_SEC}")
    return max_duration_sec


def session_meta_path(session_key: str, meta_dir: str = "/data/local/tmp") -> str:
    key = validate_session_key(session_key)
    return f"{meta_dir.rstrip('/')}/strata-perfetto-session-{key}.json"


def build_ring_config(
    preset: str = "ui-jank",
    buffer_kb: int = DEFAULT_RING_BUFFER_KB,
    max_duration_sec: int = DEFAULT_RING_DURATION_SEC,
) -> str:
    """Builds an in-memory RING_BUFFER Perfetto config (no write_into_file until stop)."""
    p = perfetto_ops.validate_preset(preset)
    buf = perfetto_ops.validate_buffer_kb(buffer_kb)
    dur = validate_max_duration(max_duration_sec)
    sources = perfetto_ops.get_preset_sources(p)
    return (
        f"buffers: {{\n"
        f"    size_kb: {buf}\n"
        f"    fill_policy: RING_BUFFER\n"
        f"}}\n"
        f"{sources}\n"
        f"duration_ms: {dur * 1000}\n"
    )


def _read_meta(path: str) -> dict[str, Any] | None:
    try:
        if os.path.isfile(path):
            with open(path, encoding="utf-8") as f:
                data = json.load(f)
            return data if isinstance(data, dict) else None
        if path.startswith("/data/local/tmp/") and os.path.exists(HOST_EXEC):
            res = _run_host(["/system/bin/cat", path], timeout=5.0)
            if res.returncode == 0 and res.stdout.strip():
                data = json.loads(res.stdout)
                return data if isinstance(data, dict) else None
    except (OSError, ValueError, subprocess.TimeoutExpired):
        pass
    return None


def _write_meta(path: str, data: dict[str, Any]) -> None:
    payload = json.dumps(data, ensure_ascii=False)
    if path.startswith("/data/local/tmp/") and os.path.exists(HOST_EXEC) and not os.path.isdir("/data/local/tmp"):
        _run_host(["/system/bin/sh", "-c", 'cat > "$1"', "_", path], input_text=payload, timeout=5.0)
        return
    try:
        with open(path, "w", encoding="utf-8") as f:
            f.write(payload)
    except OSError:
        if path.startswith("/data/local/tmp/") and os.path.exists(HOST_EXEC):
            _run_host(["/system/bin/sh", "-c", 'cat > "$1"', "_", path], input_text=payload, timeout=5.0)


def _delete_meta(path: str) -> None:
    try:
        if os.path.isfile(path):
            os.unlink(path)
            return
    except OSError:
        pass
    if path.startswith("/data/local/tmp/") and os.path.exists(HOST_EXEC):
        try:
            _run_host(["/system/bin/rm", "-f", path], timeout=5.0)
        except (OSError, subprocess.TimeoutExpired):
            pass


def _parse_pid(stdout: str) -> int | None:
    for token in stdout.strip().splitlines():
        s = token.strip()
        if s.isdigit() and int(s) > 1:
            return int(s)
    return None


def _is_pid_active(pid: int | None) -> tuple[bool, int, str]:
    if not isinstance(pid, int) or pid <= 1:
        return False, 2, ""
    try:
        res = _run_host(["/system/bin/sh", "-c", 'kill -0 "$1" 2>/dev/null', "_", str(pid)], timeout=5.0)
        return res.returncode == 0, res.returncode, res.stderr.strip()[-400:]
    except (OSError, subprocess.TimeoutExpired) as exc:
        return False, -1, str(exc)[-400:]


def _stop_pid_gracefully(pid: int) -> tuple[bool, str]:
    script = (
        'pid="$1"; kill -INT "$pid" 2>/dev/null || exit 2; '
        'i=0; while [ "$i" -lt 40 ]; do kill -0 "$pid" 2>/dev/null || exit 0; sleep 0.15; i=$((i + 1)); done; '
        'kill -TERM "$pid" 2>/dev/null || true; sleep 0.3; kill -0 "$pid" 2>/dev/null && exit 1 || exit 0'
    )
    res = _run_host(["/system/bin/sh", "-c", script, "_", str(pid)], timeout=15.0)
    return res.returncode == 0, res.stderr.strip()[-1000:]


def start_ring_session(
    preset: str = "ui-jank",
    buffer_kb: int = DEFAULT_RING_BUFFER_KB,
    max_duration_sec: int = DEFAULT_RING_DURATION_SEC,
    session_key: str = DEFAULT_SESSION_KEY,
    out_path: str | None = None,
    *,
    meta_dir: str = "/data/local/tmp",
) -> dict[str, Any]:
    """Starts a background Perfetto ring-buffer session tracked by host PID."""
    try:
        key = validate_session_key(session_key)
        p = perfetto_ops.validate_preset(preset)
        buf = perfetto_ops.validate_buffer_kb(buffer_kb)
        max_dur = validate_max_duration(max_duration_sec)
        remote_path = perfetto_ops.validate_out_path(out_path, p)
        config_txt = build_ring_config(p, buf, max_dur)
    except ValueError as exc:
        return {"ok": False, "action": "start", "error": str(exc)}

    mpath = session_meta_path(key, meta_dir)
    existing = _read_meta(mpath) or {}
    existing_pid = existing.get("pid") if isinstance(existing.get("pid"), int) else None
    active, _, _ = _is_pid_active(existing_pid)
    if active:
        return {
            "ok": False, "action": "start", "sessionKey": key, "active": True, "pid": existing_pid,
            "error": f"perfetto ring session '{key}' is already running (pid={existing_pid})",
        }

    try:
        res = _run_host(
            [PERFETTO_BIN, "--background", "--txt", "-c", "-", "-o", remote_path],
            input_text=config_txt, timeout=15.0,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"ok": False, "action": "start", "sessionKey": key, "error": str(exc)[-400:]}

    pid = _parse_pid(res.stdout)
    base = {
        "action": "start", "sessionKey": key, "preset": p,
        "bufferKb": buf, "maxDurationSec": max_dur, "remotePath": remote_path,
    }
    if res.returncode != 0 or pid is None:
        return {
            "ok": False, **base,
            "stderr": (res.stderr.strip() or res.stdout.strip() or "perfetto --background returned no pid")[-1000:],
        }

    _write_meta(
        mpath,
        {"sessionKey": key, "pid": pid, "preset": p, "bufferKb": buf, "maxDurationSec": max_dur,
         "remotePath": remote_path, "startedAt": int(time.time())},
    )
    return {"ok": True, "pid": pid, **base}


def ring_session_status(
    session_key: str = DEFAULT_SESSION_KEY, *, meta_dir: str = "/data/local/tmp"
) -> dict[str, Any]:
    """Checks whether a background Perfetto ring-buffer session is currently active."""
    try:
        key = validate_session_key(session_key)
    except ValueError as exc:
        return {"ok": False, "action": "status", "error": str(exc)}

    meta = _read_meta(session_meta_path(key, meta_dir)) or {}
    pid = meta.get("pid") if isinstance(meta.get("pid"), int) else None
    active, rc, stderr = _is_pid_active(pid)
    out: dict[str, Any] = {"ok": True, "action": "status", "active": active, "sessionKey": key, "returncode": rc}
    for k in ("pid", "preset", "bufferKb", "maxDurationSec", "remotePath", "startedAt"):
        if k in meta:
            out[k] = meta[k]
    if stderr and not active:
        out["stderr"] = stderr
    return out


def stop_ring_session(
    session_key: str = DEFAULT_SESSION_KEY, analyze: bool = True, *, meta_dir: str = "/data/local/tmp"
) -> dict[str, Any]:
    """Stops a background ring session via SIGINT, flushes trace to disk, and optionally analyzes it."""
    try:
        key = validate_session_key(session_key)
    except ValueError as exc:
        return {"ok": False, "action": "stop", "error": str(exc)}

    mpath = session_meta_path(key, meta_dir)
    meta = _read_meta(mpath) or {}
    remote_path = str(meta.get("remotePath", ""))
    pid = meta.get("pid") if isinstance(meta.get("pid"), int) else None
    if pid is None:
        return {"ok": False, "action": "stop", "sessionKey": key, "remotePath": remote_path,
                "sizeBytes": 0, "error": f"no active session metadata for '{key}'"}

    try:
        stopped_ok, stderr = _stop_pid_gracefully(pid)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"ok": False, "action": "stop", "sessionKey": key, "remotePath": remote_path, "error": str(exc)[-400:]}

    size_bytes = 0
    if remote_path:
        size_bytes = os.path.getsize(remote_path) if os.path.isfile(remote_path) else perfetto_ops.stat_remote_size(remote_path)
    _delete_meta(mpath)

    ok = (stopped_ok or size_bytes > 0) and (not remote_path or size_bytes > 0)
    out: dict[str, Any] = {
        "ok": ok, "action": "stop", "sessionKey": key, "pid": pid, "remotePath": remote_path,
        "sizeBytes": size_bytes, "stderr": stderr,
    }
    if ok and analyze and remote_path and size_bytes > 0:
        out["summary"] = perfetto_analyze.analyze_trace_file(remote_path)
    return out


def snapshot_ring_session(
    session_key: str = DEFAULT_SESSION_KEY,
    keep_running: bool = True,
    analyze: bool = True,
    *,
    meta_dir: str = "/data/local/tmp",
) -> dict[str, Any]:
    """Flushes the current ring buffer as a snapshot, optionally analyzes it, and restarts recording."""
    try:
        key = validate_session_key(session_key)
    except ValueError as exc:
        return {"ok": False, "action": "snapshot", "error": str(exc)}

    meta = _read_meta(session_meta_path(key, meta_dir))
    if not meta:
        return {"ok": False, "action": "snapshot", "sessionKey": key, "error": f"no session metadata for '{key}'"}

    preset = str(meta.get("preset", "ui-jank"))
    buffer_kb = int(meta.get("bufferKb", DEFAULT_RING_BUFFER_KB))
    max_dur = int(meta.get("maxDurationSec", DEFAULT_RING_DURATION_SEC))

    stopped = stop_ring_session(session_key=key, analyze=analyze, meta_dir=meta_dir)
    out: dict[str, Any] = {
        "ok": bool(stopped.get("ok")), "action": "snapshot", "sessionKey": key, "preset": preset,
        "remotePath": stopped.get("remotePath", ""), "sizeBytes": stopped.get("sizeBytes", 0),
        "keepRunning": keep_running, "stderr": stopped.get("stderr", ""),
    }
    if "summary" in stopped:
        out["summary"] = stopped["summary"]

    if keep_running:
        restarted = start_ring_session(
            preset=preset, buffer_kb=buffer_kb, max_duration_sec=max_dur,
            session_key=key, out_path=None, meta_dir=meta_dir,
        )
        out["restarted"] = bool(restarted.get("ok"))
        out["nextRemotePath"] = restarted.get("remotePath", "")
        if not restarted.get("ok"):
            out["restartError"] = restarted.get("error") or restarted.get("stderr", "")
    return out
