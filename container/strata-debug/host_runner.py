"""Shared `host-exec` subprocess helpers for the `strata-debug` toolkit."""

from __future__ import annotations

import os
import subprocess

HOST_EXEC = "/usr/local/bin/host-exec"


def host_exec_available() -> bool:
    """Returns True when the in-container `host-exec` bridge binary exists."""
    return os.path.exists(HOST_EXEC)


def run_host_text(
    args: list[str],
    timeout: float = 10.0,
    *,
    input_text: str | None = None,
) -> subprocess.CompletedProcess[str]:
    """Executes an Android host command via `host-exec` and decodes output as UTF-8."""
    return subprocess.run(
        [HOST_EXEC, *args],
        input=input_text,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        check=False,
    )


def run_host_bytes(args: list[str], timeout: float = 10.0) -> subprocess.CompletedProcess[bytes]:
    """Executes an Android host command via `host-exec` and captures raw binary output."""
    return subprocess.run(
        [HOST_EXEC, *args],
        capture_output=True,
        timeout=timeout,
        check=False,
    )


def run_host_stdout(args: list[str], timeout: float = 5.0) -> str:
    """Executes a host command and returns stdout, or an empty string on OS/timeout failure."""
    try:
        res = run_host_text(args, timeout=timeout)
        return res.stdout
    except (OSError, subprocess.TimeoutExpired):
        return ""
