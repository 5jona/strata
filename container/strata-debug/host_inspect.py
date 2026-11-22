"""Read-only SQLite inspection, KernelSU module metadata, input devices, and sysfs/proc reader."""

from __future__ import annotations

import os
import posixpath
import re
import shutil
import sqlite3
import subprocess
import tempfile
from typing import Any

HOST_EXEC = "/usr/local/bin/host-exec"
ALLOWED_SQL_KEYWORDS = {"SELECT", "PRAGMA", "EXPLAIN", "WITH"}
DEV_HEADER_RE = re.compile(r"^add device \d+:\s+(\S+)")
DEV_ID_RE = re.compile(
    r"bus:\s*(\S+)\s+vendor\s+(\S+)\s+product\s+(\S+)\s+version\s+(\S+)"
)


def _run_host_text(args: list[str], timeout: float = 10.0) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [HOST_EXEC, *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        check=False,
    )


def _run_host_bytes(args: list[str], timeout: float = 10.0) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [HOST_EXEC, *args], capture_output=True, timeout=timeout, check=False
    )


def validate_readonly_sql(sql: str) -> str:
    """Validates that a SQL statement is a single read-only query."""
    s = sql
    while True:
        s = s.lstrip()
        if s.startswith("--"):
            nl = s.find("\n")
            s = "" if nl == -1 else s[nl + 1 :]
        elif s.startswith("/*"):
            end = s.find("*/")
            if end == -1:
                raise ValueError("unterminated block comment in SQL")
            s = s[end + 2 :]
        else:
            break

    s = s.strip()
    if not s:
        raise ValueError("empty SQL query")
    if s.endswith(";"):
        s = s[:-1].rstrip()
    if ";" in s:
        raise ValueError("multi-statement SQL is not allowed")

    m = re.match(r"([A-Za-z]+)", s)
    if not m or m.group(1).upper() not in ALLOWED_SQL_KEYWORDS:
        raise ValueError("only read-only SELECT, PRAGMA, EXPLAIN, or WITH queries are allowed")
    return s


def _copy_sqlite_to_tmpdir(db_path: str, tmpdir: str) -> str:
    dst_main = os.path.join(tmpdir, os.path.basename(db_path) or "target.db")
    if os.path.isfile(db_path):
        for suffix in ("", "-wal", "-shm"):
            src = db_path + suffix
            if os.path.isfile(src):
                shutil.copy2(src, dst_main + suffix)
        return dst_main

    for suffix in ("", "-wal", "-shm"):
        src = db_path + suffix
        res = _run_host_bytes(["/system/bin/cat", src])
        if res.returncode == 0:
            with open(dst_main + suffix, "wb") as f:
                f.write(res.stdout)
        elif suffix == "":
            raise FileNotFoundError(f"database not found or unreadable: {db_path}")
    return dst_main


def _serialize_val(val: Any) -> Any:
    if isinstance(val, bytes):
        return val.hex()
    return val


def db_list_tables(db_path: str) -> dict[str, Any]:
    """Lists tables and views in a SQLite database using a temporary read-only copy."""
    with tempfile.TemporaryDirectory(prefix="strata-db-") as tmpdir:
        try:
            copy_path = _copy_sqlite_to_tmpdir(db_path, tmpdir)
        except FileNotFoundError as exc:
            return {"ok": False, "dbPath": db_path, "error": str(exc)}
        conn = sqlite3.connect(f"file:{copy_path}?mode=ro", uri=True)
        try:
            rows = conn.execute(
                "SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') ORDER BY name"
            ).fetchall()
            return {
                "ok": True,
                "dbPath": db_path,
                "tables": [{"name": str(r[0]), "type": str(r[1])} for r in rows],
            }
        finally:
            conn.close()


def db_query(db_path: str, sql: str, *, limit: int = 50) -> dict[str, Any]:
    """Executes a validated read-only SQL query against a temporary SQLite copy."""
    clean_sql = validate_readonly_sql(sql)
    with tempfile.TemporaryDirectory(prefix="strata-db-") as tmpdir:
        try:
            copy_path = _copy_sqlite_to_tmpdir(db_path, tmpdir)
        except FileNotFoundError as exc:
            return {"ok": False, "dbPath": db_path, "error": str(exc)}
        conn = sqlite3.connect(f"file:{copy_path}?mode=ro", uri=True)
        try:
            cur = conn.execute(clean_sql)
            cols = [d[0] for d in cur.description] if cur.description else []
            fetch_n = limit + 1 if limit > 0 else 1001
            raw_rows = cur.fetchmany(fetch_n)
            truncated = len(raw_rows) > (limit if limit > 0 else 1000)
            if truncated:
                raw_rows = raw_rows[:-1]
            rows = [[_serialize_val(v) for v in row] for row in raw_rows]
            return {
                "ok": True,
                "dbPath": db_path,
                "columns": cols,
                "rowCount": len(rows),
                "truncated": truncated,
                "rows": rows,
            }
        except sqlite3.Error as exc:
            return {"ok": False, "dbPath": db_path, "error": str(exc)}
        finally:
            conn.close()


def parse_module_prop(text: str) -> dict[str, str]:
    """Parses key=value entries from a KernelSU/Magisk module.prop file."""
    props: dict[str, str] = {}
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        k, v = stripped.split("=", 1)
        props[k.strip()] = v.strip()
    return props


def inspect_ksu_modules(modules_dir: str = "/data/adb/modules") -> dict[str, Any]:
    """Lists installed KernelSU modules with module.prop metadata and flag states."""
    ls_res = _run_host_text(["/system/bin/ls", "-1", modules_dir], timeout=8.0)
    if ls_res.returncode != 0:
        return {
            "ok": False,
            "modulesDir": modules_dir,
            "error": ls_res.stderr.strip()[:240] or "failed to list modules directory",
        }

    modules: list[dict[str, Any]] = []
    for name in sorted(ln.strip() for ln in ls_res.stdout.splitlines() if ln.strip()):
        mod_path = f"{modules_dir.rstrip('/')}/{name}"
        prop_res = _run_host_text(["/system/bin/cat", f"{mod_path}/module.prop"], timeout=5.0)
        props = parse_module_prop(prop_res.stdout) if prop_res.returncode == 0 else {}
        flag_res = _run_host_text(
            [
                "/system/bin/sh",
                "-c",
                'for f in disable remove update skip_mount; do test -e "$1/$f" && echo "$f"; done',
                "_",
                mod_path,
            ],
            timeout=5.0,
        )
        flags = set(flag_res.stdout.split())
        modules.append(
            {
                "id": props.get("id", name),
                "name": props.get("name", name),
                "version": props.get("version", ""),
                "versionCode": props.get("versionCode", ""),
                "author": props.get("author", ""),
                "description": props.get("description", ""),
                "flags": {
                    "disable": "disable" in flags,
                    "remove": "remove" in flags,
                    "update": "update" in flags,
                    "skipMount": "skip_mount" in flags,
                },
            }
        )
    return {"ok": True, "modulesDir": modules_dir, "count": len(modules), "modules": modules}


def parse_getevent_devices(text: str) -> list[dict[str, Any]]:
    """Parses `getevent -lp` output into structured input device descriptors."""
    devices: list[dict[str, Any]] = []
    cur: dict[str, Any] | None = None
    for line in text.splitlines():
        hm = DEV_HEADER_RE.match(line.strip())
        if hm:
            if cur is not None:
                devices.append(cur)
            cur = {
                "device": hm.group(1),
                "name": "",
                "bus": None,
                "vendor": None,
                "product": None,
                "version": None,
                "hasTouch": False,
                "hasKeys": False,
            }
            continue
        if cur is None:
            continue
        stripped = line.strip()
        if stripped.startswith("name:"):
            cur["name"] = stripped.split(":", 1)[1].strip().strip('"')
        else:
            idm = DEV_ID_RE.search(stripped)
            if idm:
                cur["bus"], cur["vendor"], cur["product"], cur["version"] = idm.groups()
        if any(tok in stripped for tok in ("ABS_MT_POSITION_X", "ABS_MT_TRACKING_ID", "BTN_TOUCH")):
            cur["hasTouch"] = True
        if "KEY_" in stripped:
            cur["hasKeys"] = True
    if cur is not None:
        devices.append(cur)
    return devices


def inspect_input_devices() -> dict[str, Any]:
    """Queries Android input devices via `getevent -lp`."""
    res = _run_host_text(["/system/bin/getevent", "-lp"], timeout=10.0)
    if res.returncode != 0 and not res.stdout:
        return {"ok": False, "error": res.stderr.strip()[:240] or "getevent failed"}
    devices = parse_getevent_devices(res.stdout)
    return {"ok": True, "count": len(devices), "devices": devices}


def read_sysfs_node(path: str, *, max_bytes: int = 16384) -> dict[str, Any]:
    """Reads a file or directory listing strictly under /sys/ or /proc/."""
    if not path or not (path.startswith("/sys/") or path.startswith("/proc/")):
        return {"ok": False, "error": "path must start with /sys/ or /proc/"}
    norm = posixpath.normpath(path)
    if not (norm.startswith("/sys/") or norm.startswith("/proc/")):
        return {"ok": False, "error": "path traversal outside /sys/ or /proc/ is forbidden"}

    dir_chk = _run_host_text(
        ["/system/bin/sh", "-c", 'if test -d "$1"; then echo dir; elif test -e "$1"; then echo file; else echo missing; fi', "_", norm],
        timeout=5.0,
    )
    kind = dir_chk.stdout.strip()
    if kind == "missing":
        return {"ok": False, "path": norm, "error": "node not found"}
    if kind == "dir":
        ls_res = _run_host_text(["/system/bin/ls", "-1", norm], timeout=5.0)
        entries = [ln.strip() for ln in ls_res.stdout.splitlines() if ln.strip()]
        return {"ok": True, "path": norm, "type": "dir", "entries": entries}

    read_res = _run_host_bytes(["/system/bin/head", "-c", str(max(1, max_bytes)), norm], timeout=5.0)
    if read_res.returncode != 0 and not read_res.stdout:
        return {"ok": False, "path": norm, "error": read_res.stderr.decode("utf-8", "replace").strip()[:240]}
    text = read_res.stdout.decode("utf-8", errors="replace")
    return {"ok": True, "path": norm, "type": "file", "content": text, "bytes": len(read_res.stdout)}
