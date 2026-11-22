"""L1 WebView CDP discovery, proxy, and zero-dependency RFC6455 WebSocket auditor."""

from __future__ import annotations

import base64
import json
import os
import re
import socket
import struct
import time
from typing import Any
from urllib.parse import urlparse

from cdp_audit_js import DOM_AUDIT_JS

DEVTOOLS_RE = re.compile(r"@((?:webview|chrome)_devtools_remote(?:_(\d+))?)")


def _read_cmdline(pid: int) -> str:
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return f.read().replace(b"\x00", b" ").decode("utf-8", errors="replace").strip()
    except OSError:
        return ""


def scan_devtools_sockets(proc_unix_path: str = "/proc/net/unix") -> list[dict[str, Any]]:
    seen: dict[str, dict[str, Any]] = {}
    try:
        with open(proc_unix_path, encoding="utf-8", errors="replace") as f:
            lines = f.readlines()
    except OSError:
        return []
    for line in lines[1:]:
        m = DEVTOOLS_RE.search(line)
        if not m:
            continue
        sock_name = m.group(1)
        pid = int(m.group(2)) if m.group(2) else None
        if sock_name not in seen:
            seen[sock_name] = {
                "socket": sock_name,
                "pid": pid,
                "process": _read_cmdline(pid) if pid else "",
            }
    return list(seen.values())


def match_devtools_socket(
    target: str | None = "pochita.sideveil.host",
    pid: int | None = None,
    proc_unix_path: str = "/proc/net/unix",
) -> dict[str, Any] | None:
    sockets = scan_devtools_sockets(proc_unix_path)
    if pid is not None:
        for item in sockets:
            if item["pid"] == pid:
                return item
        return {"socket": f"webview_devtools_remote_{pid}", "pid": pid, "process": _read_cmdline(pid)}
    if target:
        for item in sockets:
            if target in item["process"]:
                return item
    return sockets[0] if sockets else None


def _recv_http_response(sock: socket.socket) -> tuple[bytes, bytes]:
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = sock.recv(4096)
        if not chunk:
            break
        buf += chunk
    header, _, rest = buf.partition(b"\r\n\r\n")
    return header, rest


def http_get_unix(socket_name: str, path: str = "/json/list", timeout: float = 4.0) -> Any:
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(timeout)
    try:
        sock.connect(f"\0{socket_name}")
        req = f"GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n".encode()
        sock.sendall(req)
        header, body = _recv_http_response(sock)
        while True:
            chunk = sock.recv(65536)
            if not chunk:
                break
            body += chunk
        if b"200" not in header.splitlines()[0]:
            raise RuntimeError(f"CDP HTTP {path} failed: {header.splitlines()[0].decode(errors='replace')}")
        return json.loads(body.decode("utf-8"))
    finally:
        sock.close()


class CdpWsClient:
    """Minimal RFC6455 WebSocket client over abstract UDS or loopback TCP."""

    def __init__(self, sock: socket.socket, ws_path: str) -> None:
        self.sock = sock
        self._buf = b""
        self._next_id = 1
        self.console_events: list[dict[str, Any]] = []
        self.exceptions: list[dict[str, Any]] = []
        self._handshake(ws_path)

    @classmethod
    def connect_unix(cls, socket_name: str, ws_path: str, timeout: float = 5.0) -> CdpWsClient:
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(timeout)
        sock.connect(f"\0{socket_name}")
        return cls(sock, ws_path)

    def _handshake(self, ws_path: str) -> None:
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        req = (
            f"GET {ws_path} HTTP/1.1\r\n"
            "Host: localhost\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        ).encode()
        self.sock.sendall(req)
        header, rest = _recv_http_response(self.sock)
        self._buf = rest
        if b"101" not in header.splitlines()[0]:
            raise RuntimeError(f"WebSocket upgrade rejected: {header.decode(errors='replace')}")

    def _read_exact(self, n: int) -> bytes:
        while len(self._buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise EOFError("WebSocket closed")
            self._buf += chunk
        out, self._buf = self._buf[:n], self._buf[n:]
        return out

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        mask = os.urandom(4)
        length = len(payload)
        head = bytearray([0x80 | opcode])
        if length < 126:
            head.append(0x80 | length)
        elif length < 65536:
            head.append(0x80 | 126)
            head.extend(struct.pack("!H", length))
        else:
            head.append(0x80 | 127)
            head.extend(struct.pack("!Q", length))
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(bytes(head) + mask + masked)

    def _recv_text(self) -> str | None:
        while True:
            b0, b1 = self._read_exact(2)
            opcode = b0 & 0x0F
            masked = (b1 & 0x80) != 0
            length = b1 & 0x7F
            if length == 126:
                length = struct.unpack("!H", self._read_exact(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self._read_exact(8))[0]
            mask = self._read_exact(4) if masked else b""
            data = self._read_exact(length)
            if masked:
                data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
            if opcode == 0x8:
                return None
            if opcode == 0x9:
                self._send_frame(0xA, data)
                continue
            if opcode == 0x1:
                return data.decode("utf-8", errors="replace")

    def call(self, method: str, params: dict[str, Any] | None = None, timeout: float = 5.0) -> dict[str, Any]:
        req_id = self._next_id
        self._next_id += 1
        self._send_frame(0x1, json.dumps({"id": req_id, "method": method, "params": params or {}}).encode())
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            raw = self._recv_text()
            if raw is None:
                raise EOFError("CDP WebSocket closed before response")
            msg = json.loads(raw)
            if msg.get("id") == req_id:
                return msg
            if msg.get("method") == "Runtime.consoleAPICalled":
                self.console_events.append(msg.get("params", {}))
            elif msg.get("method") == "Runtime.exceptionThrown":
                self.exceptions.append(msg.get("params", {}))
        raise TimeoutError(f"CDP call {method} timed out")

    def close(self) -> None:
        try:
            self._send_frame(0x8, b"")
        except OSError:
            pass
        self.sock.close()


def audit_webview(
    target: str | None = "pochita.sideveil.host",
    pid: int | None = None,
    url_contains: str | None = None,
    eval_js: str | None = None,
) -> dict[str, Any]:
    matched = match_devtools_socket(target=target, pid=pid)
    if not matched:
        return {"ok": False, "error": "no webview_devtools_remote socket found"}
    targets = http_get_unix(matched["socket"], "/json/list")
    pages = [t for t in targets if t.get("webSocketDebuggerUrl")]
    if url_contains:
        pages = [t for t in pages if url_contains in t.get("url", "")] or pages
    if not pages:
        return {"ok": False, "socket": matched["socket"], "error": "no inspectable WebView pages"}

    page = pages[0]
    ws_path = urlparse(page["webSocketDebuggerUrl"]).path
    client = CdpWsClient.connect_unix(matched["socket"], ws_path)
    try:
        client.call("Runtime.enable")
        res = client.call(
            "Runtime.evaluate",
            {"expression": DOM_AUDIT_JS, "returnByValue": True, "awaitPromise": True},
        )
        audit = res.get("result", {}).get("result", {}).get("value", {})
        eval_out = None
        if eval_js:
            ev = client.call(
                "Runtime.evaluate",
                {"expression": eval_js, "returnByValue": True, "awaitPromise": True},
            )
            eval_out = ev.get("result", {}).get("result", {}).get("value")
        return {
            "ok": True,
            "socket": matched["socket"],
            "pid": matched["pid"],
            "pageUrl": page.get("url"),
            "audit": audit,
            "evalResult": eval_out,
            "consoleErrors": [e for e in client.console_events if e.get("type") in {"error", "warning"}][:10],
            "exceptions": client.exceptions[:10],
        }
    finally:
        client.close()
