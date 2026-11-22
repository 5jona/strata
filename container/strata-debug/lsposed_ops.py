"""Read-only LSPosed module configuration and log inspection."""

from __future__ import annotations

import os
import re
import sqlite3
import subprocess
import tempfile
from typing import Any, Callable

HOST_EXEC = "/usr/local/bin/host-exec"


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


def _default_copy_db(db_path: str, tmpdir: str) -> str | None:
    chk = _run_host_text(["/system/bin/sh", "-c", 'test -f "$1" && echo ok', "_", db_path])
    if chk.stdout.strip() != "ok":
        return None
    base = os.path.basename(db_path)
    dst_main = os.path.join(tmpdir, base)
    for suffix in ("", "-wal", "-shm"):
        src = db_path + suffix
        res = _run_host_bytes(["/system/bin/cat", src])
        if res.returncode == 0:
            with open(dst_main + suffix, "wb") as f:
                f.write(res.stdout)
        elif suffix == "":
            return None
    return dst_main


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    cur = conn.execute(f'PRAGMA table_info("{table}")')
    return {str(row[1]) for row in cur.fetchall()}


def query_lsposed_modules(
    module_filter: str | None = None,
    *,
    lspd_dir: str = "/data/adb/lspd",
    copy_fn: Callable[[str, str], str | None] | None = None,
) -> dict[str, Any]:
    """Queries LSPosed module states and scopes from a temporary read-only DB snapshot."""
    db_path = f"{lspd_dir.rstrip('/')}/config/modules_config.db"
    copier = copy_fn or _default_copy_db

    with tempfile.TemporaryDirectory(prefix="strata-lspd-") as tmpdir:
        db_copy = copier(db_path, tmpdir)
        if not db_copy or not os.path.isfile(db_copy):
            return {
                "ok": True,
                "available": False,
                "reason": "LSPosed config database not found",
            }

        conn = sqlite3.connect(f"file:{db_copy}?mode=ro", uri=True)
        conn.row_factory = sqlite3.Row
        try:
            tables = [
                str(r[0])
                for r in conn.execute(
                    "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
                ).fetchall()
            ]
            if "modules" not in tables:
                return {
                    "ok": True,
                    "available": True,
                    "tables": tables,
                    "modules": [],
                    "note": "modules table not found in LSPosed database",
                }

            mod_cols = _table_columns(conn, "modules")
            pkg_col = "module_pkg_name" if "module_pkg_name" in mod_cols else None
            if not pkg_col:
                return {
                    "ok": True,
                    "available": True,
                    "tables": tables,
                    "modules": [],
                    "note": "unrecognized modules table schema",
                }

            enabled_by_pkg: dict[str, bool] = {}
            if "modules_state" in tables:
                st_cols = _table_columns(conn, "modules_state")
                if {"module_pkg_name", "enabled"}.issubset(st_cols):
                    for srow in conn.execute("SELECT module_pkg_name, enabled FROM modules_state").fetchall():
                        pkg_key = str(srow["module_pkg_name"])
                        enabled_by_pkg[pkg_key] = enabled_by_pkg.get(pkg_key, False) or bool(srow["enabled"])

            scopes_by_mid: dict[Any, list[dict[str, Any]]] = {}
            scopes_by_pkg: dict[str, list[dict[str, Any]]] = {}
            if "scope" in tables:
                scope_cols = _table_columns(conn, "scope")
                has_uid = "user_id" in scope_cols
                if {"mid", "app_pkg_name"}.issubset(scope_cols):
                    q = "SELECT mid, app_pkg_name, user_id FROM scope" if has_uid else "SELECT mid, app_pkg_name FROM scope"
                    for srow in conn.execute(q).fetchall():
                        entry: dict[str, Any] = {"package": srow["app_pkg_name"]}
                        if has_uid:
                            entry["userId"] = srow["user_id"]
                        scopes_by_mid.setdefault(srow["mid"], []).append(entry)
                elif {"module_pkg_name", "app_pkg_name"}.issubset(scope_cols):
                    q = (
                        "SELECT module_pkg_name, app_pkg_name, user_id FROM scope"
                        if has_uid
                        else "SELECT module_pkg_name, app_pkg_name FROM scope"
                    )
                    for srow in conn.execute(q).fetchall():
                        entry = {"package": srow["app_pkg_name"]}
                        if has_uid:
                            entry["userId"] = srow["user_id"]
                        scopes_by_pkg.setdefault(str(srow["module_pkg_name"]), []).append(entry)

            select_cols = [c for c in ("mid", pkg_col, "enabled", "apk_path") if c in mod_cols]
            rows = conn.execute(f"SELECT {', '.join(select_cols)} FROM modules").fetchall()
            filt_low = module_filter.lower() if module_filter else None

            modules: list[dict[str, Any]] = []
            for row in rows:
                pkg = str(row[pkg_col])
                if filt_low and filt_low not in pkg.lower():
                    continue
                mid = row["mid"] if "mid" in mod_cols else None
                enabled = bool(row["enabled"]) if "enabled" in mod_cols else enabled_by_pkg.get(pkg, False)
                scopes = scopes_by_mid.get(mid, []) if mid is not None else scopes_by_pkg.get(pkg, [])
                modules.append(
                    {
                        "package": pkg,
                        "enabled": enabled,
                        "apkPath": row["apk_path"] if "apk_path" in mod_cols else None,
                        "scopes": scopes,
                    }
                )

            return {
                "ok": True,
                "available": True,
                "tables": tables,
                "count": len(modules),
                "modules": modules,
            }
        finally:
            conn.close()


def read_lsposed_logs(
    *,
    module_filter: str | None = None,
    grep_pattern: str | None = None,
    limit: int = 80,
    lspd_dir: str = "/data/adb/lspd",
) -> dict[str, Any]:
    """Reads and filters the newest LSPosed log file under `<lspd_dir>/log/`."""
    log_dir = f"{lspd_dir.rstrip('/')}/log"
    ls_res = _run_host_text(["/system/bin/ls", "-1t", log_dir], timeout=8.0)
    if ls_res.returncode != 0 or not ls_res.stdout.strip():
        return {
            "ok": True,
            "available": False,
            "reason": "LSPosed log directory not found or empty",
        }

    entries = [ln.strip() for ln in ls_res.stdout.splitlines() if ln.strip()]
    mod_logs = [e for e in entries if e.startswith("modules_") and e.endswith(".log")]
    any_logs = [e for e in entries if e.endswith(".log")]
    chosen = mod_logs[0] if mod_logs else (any_logs[0] if any_logs else None)
    if not chosen:
        return {
            "ok": True,
            "available": False,
            "reason": "no LSPosed .log files found",
        }

    log_path = f"{log_dir}/{chosen}"
    tail_count = max(limit * 5, 400) if limit > 0 else 400
    read_res = _run_host_text(["/system/bin/tail", "-n", str(tail_count), log_path], timeout=10.0)
    if read_res.returncode != 0:
        return {
            "ok": False,
            "available": True,
            "logFile": log_path,
            "error": read_res.stderr.strip()[:240] or "failed to read LSPosed log",
        }

    regex: re.Pattern[str] | None = None
    if grep_pattern:
        try:
            regex = re.compile(grep_pattern)
        except re.error as exc:
            return {"ok": False, "error": f"invalid grep pattern: {exc}"}

    mod_low = module_filter.lower() if module_filter else None
    lines: list[str] = []
    for line in read_res.stdout.splitlines():
        if mod_low and mod_low not in line.lower():
            continue
        if regex and not regex.search(line):
            continue
        lines.append(line)

    if limit > 0 and len(lines) > limit:
        lines = lines[-limit:]

    return {
        "ok": True,
        "available": True,
        "logFile": log_path,
        "count": len(lines),
        "lines": lines,
    }
