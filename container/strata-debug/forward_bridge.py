#!/usr/bin/env python3
"""Lightweight TCP <-> abstract UDS / TCP port forwarder for Strata shared NET_NS.

Powers `strata-debug forward` and `strata-debug cdp serve`.
"""

from __future__ import annotations

import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import threading
from typing import Any

DEFAULT_STATE_PATH = os.environ.get("STRATA_FORWARD_STATE", "/tmp/strata-forwards.json")


def parse_endpoint(spec: str) -> tuple[str, str | int]:
    if spec.startswith("tcp:"):
        raw_port = spec[4:]
        if not raw_port.isdigit():
            raise ValueError(f"invalid tcp port in '{spec}'")
        port = int(raw_port)
        if not (0 <= port <= 65535):
            raise ValueError(f"tcp port out of range in '{spec}'")
        return ("tcp", port)
    if spec.startswith("localabstract:"):
        name = spec[len("localabstract:") :]
        if not name:
            raise ValueError(f"empty abstract socket name in '{spec}'")
        return ("localabstract", name)
    if spec.startswith("localfilesystem:"):
        path = spec[len("localfilesystem:") :]
        if not path:
            raise ValueError(f"empty filesystem socket path in '{spec}'")
        return ("localfilesystem", path)
    raise ValueError(f"unsupported endpoint spec '{spec}'")


def parse_forward_args(args: list[str]) -> dict[str, Any]:
    if not args:
        raise ValueError("missing forward/reverse arguments")
    if args[0] == "--list":
        return {"action": "list"}
    if args[0] == "--remove-all":
        return {"action": "remove-all"}
    if args[0] == "--remove":
        if len(args) < 2:
            raise ValueError("--remove requires <local>")
        return {"action": "remove", "local": args[1]}
    no_rebind = False
    rest = list(args)
    if rest[0] == "--no-rebind":
        no_rebind = True
        rest = rest[1:]
    if len(rest) != 2:
        raise ValueError("expected <local> <remote>")
    parse_endpoint(rest[0])
    parse_endpoint(rest[1])
    return {"action": "add", "local": rest[0], "remote": rest[1], "no_rebind": no_rebind}


def _pid_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def _load_state(state_path: str = DEFAULT_STATE_PATH) -> list[dict[str, Any]]:
    if not os.path.isfile(state_path):
        return []
    try:
        with open(state_path, encoding="utf-8") as f:
            entries = json.load(f)
    except (OSError, json.JSONDecodeError):
        return []
    return [e for e in entries if isinstance(e, dict) and _pid_alive(int(e.get("pid", 0)))]


def _save_state(entries: list[dict[str, Any]], state_path: str = DEFAULT_STATE_PATH) -> None:
    parent = os.path.dirname(os.path.abspath(state_path)) or "/tmp"
    os.makedirs(parent, exist_ok=True)
    fd, tmp_path = tempfile.mkstemp(prefix=".strata-fwd-", dir=parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(entries, f, indent=2)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp_path, state_path)
    except Exception:
        if os.path.exists(tmp_path):
            os.unlink(tmp_path)
        raise


def _kill_pid(pid: int) -> None:
    try:
        os.kill(pid, signal.SIGTERM)
    except OSError:
        pass


def connect_endpoint(spec: str, timeout: float = 5.0) -> socket.socket:
    kind, val = parse_endpoint(spec)
    if kind == "tcp":
        return socket.create_connection(("127.0.0.1", int(val)), timeout=timeout)
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(timeout)
    sock.connect(f"\0{val}" if kind == "localabstract" else str(val))
    return sock


def _pump(src: socket.socket, dst: socket.socket) -> None:
    try:
        while chunk := src.recv(65536):
            dst.sendall(chunk)
    except OSError:
        pass
    finally:
        try:
            dst.shutdown(socket.SHUT_WR)
        except OSError:
            pass


def run_proxy(local_spec: str, remote_spec: str) -> int:
    kind, port = parse_endpoint(local_spec)
    if kind != "tcp":
        raise ValueError(f"local listener must be tcp:<port>, got '{local_spec}'")
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", int(port)))
    srv.listen(64)
    stop = threading.Event()

    def _on_sig(_s: int, _f: Any) -> None:
        stop.set()
        srv.close()

    signal.signal(signal.SIGTERM, _on_sig)
    signal.signal(signal.SIGINT, _on_sig)

    while not stop.is_set():
        try:
            client, _ = srv.accept()
        except OSError:
            break

        def _handle(c: socket.socket) -> None:
            try:
                up = connect_endpoint(remote_spec)
                up.settimeout(None)
                c.settimeout(None)
            except OSError:
                c.close()
                return
            t1 = threading.Thread(target=_pump, args=(c, up), daemon=True)
            t2 = threading.Thread(target=_pump, args=(up, c), daemon=True)
            t1.start()
            t2.start()
            t1.join()
            t2.join()
            c.close()
            up.close()

        threading.Thread(target=_handle, args=(client,), daemon=True).start()
    return 0


def handle_forward(
    args: list[str], *, mode: str = "forward", state_path: str = DEFAULT_STATE_PATH
) -> tuple[int, str]:
    try:
        parsed = parse_forward_args(args)
    except ValueError as exc:
        return 1, f"strata-debug {mode}: {exc}"

    entries = _load_state(state_path)
    action = parsed["action"]
    if action == "list":
        _save_state(entries, state_path)
        return 0, "\n".join(
            f"{e['local']} -> {e['remote']} (pid={e['pid']})"
            for e in entries
            if e.get("mode", "forward") == mode
        )
    if action == "remove-all":
        kept = [e for e in entries if e.get("mode", "forward") != mode]
        for e in entries:
            if e.get("mode", "forward") == mode:
                _kill_pid(int(e["pid"]))
        _save_state(kept, state_path)
        return 0, ""
    if action == "remove":
        target = parsed["local"]
        removed = [e for e in entries if e.get("mode", "forward") == mode and e.get("local") == target]
        for e in removed:
            _kill_pid(int(e["pid"]))
        _save_state([e for e in entries if e not in removed], state_path)
        return (0, "") if removed else (1, f"strata-debug {mode}: '{target}' not found")

    local_spec, remote_spec = parsed["local"], parsed["remote"]
    existing = [e for e in entries if e.get("mode", "forward") == mode and e.get("local") == local_spec]
    if existing and parsed.get("no_rebind"):
        return 1, f"strata-debug {mode}: '{local_spec}' already bound"
    for e in existing:
        _kill_pid(int(e["pid"]))
    entries = [e for e in entries if e not in existing]
    proc = subprocess.Popen(
        [sys.executable, os.path.abspath(__file__), "_serve", local_spec, remote_spec],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
    )
    entries.append({"mode": mode, "local": local_spec, "remote": remote_spec, "pid": proc.pid})
    _save_state(entries, state_path)
    return 0, ""


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "_serve":
        sys.exit(run_proxy(sys.argv[2], sys.argv[3]))
    sys.exit(1)
