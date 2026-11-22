"""L2a incremental logcat cursor manager with atomic state persistence."""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import tempfile
from typing import Any

HOST_EXEC = "/usr/local/bin/host-exec"
DEFAULT_CURSOR_PATH = "/tmp/strata-debug-logcat.cursor"

PRIORITY_ORDER = {"V": 0, "D": 1, "I": 2, "W": 3, "E": 4, "F": 5, "A": 5}
THREADTIME_RE = re.compile(
    r"^(\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2}\.\d{3})\s+(\d+)\s+(\d+)\s+([VDIWEFA])\s+([^:]+?)\s*:\s?(.*)$"
)


def parse_threadtime_line(line: str) -> dict[str, Any] | None:
    m = THREADTIME_RE.match(line.rstrip("\r\n"))
    if not m:
        return None
    return {
        "ts": m.group(1),
        "pid": int(m.group(2)),
        "tid": int(m.group(3)),
        "priority": m.group(4),
        "tag": m.group(5).strip(),
        "msg": m.group(6),
    }


def _atomic_write_json(path: str, data: dict[str, Any]) -> None:
    parent = os.path.dirname(os.path.abspath(path)) or "/tmp"
    os.makedirs(parent, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".strata-logcat-", dir=parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    except Exception:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise


def _load_cursor(cursor_path: str) -> dict[str, Any]:
    if not os.path.isfile(cursor_path):
        return {}
    try:
        with open(cursor_path, encoding="utf-8") as f:
            data = json.load(f)
            return data if isinstance(data, dict) else {}
    except (OSError, json.JSONDecodeError):
        return {}


def _line_digest(line: str) -> str:
    return hashlib.sha1(line.encode("utf-8", errors="replace")).hexdigest()[:12]


def _host_timestamp() -> str:
    res = subprocess.run(
        [HOST_EXEC, "/system/bin/date", "+%m-%d %H:%M:%S.000"],
        capture_output=True,
        text=True,
        check=False,
    )
    ts = res.stdout.strip()
    if ts:
        return ts
    tail = subprocess.run(
        [HOST_EXEC, "/system/bin/logcat", "-d", "-v", "threadtime", "-t", "1"],
        capture_output=True,
        text=True,
        check=False,
    )
    for line in reversed(tail.stdout.splitlines()):
        parsed = parse_threadtime_line(line)
        if parsed:
            return parsed["ts"]
    return "01-01 00:00:00.000"


def mark_cursor(cursor_path: str = DEFAULT_CURSOR_PATH) -> dict[str, Any]:
    ts = _host_timestamp()
    state = {"ts": ts, "lastDigest": ""}
    _atomic_write_json(cursor_path, state)
    return state


def filter_logcat_lines(
    lines: list[str],
    *,
    priority: str = "W",
    tag: str | None = None,
    pid: int | None = None,
    last_digest: str = "",
    limit: int = 100,
) -> tuple[list[dict[str, Any]], str, str]:
    min_prio = PRIORITY_ORDER.get(priority.upper(), 3)
    entries: list[dict[str, Any]] = []
    latest_ts = ""
    latest_digest = last_digest
    skip_until_digest = bool(last_digest)

    for raw in lines:
        parsed = parse_threadtime_line(raw)
        if not parsed:
            continue
        digest = _line_digest(raw)
        latest_ts = parsed["ts"]
        latest_digest = digest
        if skip_until_digest:
            if digest == last_digest:
                skip_until_digest = False
            continue
        if PRIORITY_ORDER.get(parsed["priority"], 0) < min_prio:
            continue
        if tag and tag.lower() not in parsed["tag"].lower():
            continue
        if pid is not None and parsed["pid"] != pid:
            continue
        entries.append(parsed)

    if skip_until_digest:
        # Cursor line rotated out of ring buffer; re-run without digest skipping.
        return filter_logcat_lines(
            lines, priority=priority, tag=tag, pid=pid, last_digest="", limit=limit
        )
    return entries[-limit:] if limit > 0 else entries, latest_ts, latest_digest


def read_incremental(
    cursor_path: str = DEFAULT_CURSOR_PATH,
    *,
    priority: str = "W",
    tag: str | None = None,
    pid: int | None = None,
    limit: int = 100,
    advance: bool = True,
) -> dict[str, Any]:
    state = _load_cursor(cursor_path)
    since_ts = state.get("ts")
    cmd = [HOST_EXEC, "/system/bin/logcat", "-d", "-v", "threadtime"]
    if since_ts:
        cmd.extend(["-T", since_ts])
    else:
        cmd.extend(["-t", str(max(limit * 4, 200))])

    res = subprocess.run(cmd, capture_output=True, text=True, check=False)
    lines = res.stdout.splitlines()
    entries, latest_ts, latest_digest = filter_logcat_lines(
        lines,
        priority=priority,
        tag=tag,
        pid=pid,
        last_digest=str(state.get("lastDigest", "")),
        limit=limit,
    )
    if advance and latest_ts:
        _atomic_write_json(cursor_path, {"ts": latest_ts, "lastDigest": latest_digest})
    return {
        "ok": res.returncode == 0,
        "cursor": latest_ts or since_ts or "",
        "count": len(entries),
        "entries": entries,
    }
