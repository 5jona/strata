"""Abstract Unix Socket HTTP/1.1 client (Content-Length + chunked) and CDP HTTP/WS proxy."""

from __future__ import annotations

import json
import signal
import socket
import threading
from typing import Any
from urllib.parse import urlparse, urlunparse

import cdp_ws

REWRITE_PATHS = frozenset({"/json", "/json/list", "/json/version"})


def parse_http_head(raw_head: bytes) -> tuple[int, str, dict[str, str]]:
    """Parses HTTP response head bytes into (status_code, status_line, lowercase_headers)."""
    lines = raw_head.split(b"\r\n")
    if not lines or not lines[0]:
        raise RuntimeError("empty HTTP response")
    status_line = lines[0].decode("iso-8859-1", errors="replace")
    parts = status_line.split(" ", 2)
    if len(parts) < 2 or not parts[1].isdigit():
        raise RuntimeError(f"malformed HTTP status line: {status_line}")
    status_code = int(parts[1])
    headers: dict[str, str] = {}
    for line in lines[1:]:
        if b":" not in line:
            continue
        k, v = line.split(b":", 1)
        key = k.decode("iso-8859-1", errors="replace").strip().lower()
        val = v.decode("iso-8859-1", errors="replace").strip()
        headers[key] = f"{headers[key]}, {val}" if key in headers else val
    return status_code, status_line, headers


def parse_http_request_head(raw_head: bytes) -> tuple[str, str, dict[str, str]]:
    """Parses HTTP request head bytes into (method, target_path, lowercase_headers)."""
    lines = raw_head.split(b"\r\n")
    if not lines or not lines[0]:
        raise RuntimeError("empty HTTP request")
    req_line = lines[0].decode("iso-8859-1", errors="replace")
    parts = req_line.split(" ", 2)
    if len(parts) < 2:
        raise RuntimeError(f"malformed HTTP request line: {req_line}")
    headers: dict[str, str] = {}
    for line in lines[1:]:
        if b":" not in line:
            continue
        k, v = line.split(b":", 1)
        key = k.decode("iso-8859-1", errors="replace").strip().lower()
        val = v.decode("iso-8859-1", errors="replace").strip()
        headers[key] = f"{headers[key]}, {val}" if key in headers else val
    return parts[0], parts[1], headers


def decode_chunked_body(data: bytes) -> bytes:
    """Decodes an HTTP/1.1 Transfer-Encoding: chunked payload."""
    out = bytearray()
    pos = 0
    n = len(data)
    while True:
        crlf = data.find(b"\r\n", pos)
        if crlf == -1:
            raise ValueError("incomplete chunked size line")
        size_token = data[pos:crlf].split(b";", 1)[0].strip()
        try:
            size = int(size_token, 16)
        except ValueError as exc:
            raise ValueError(f"invalid chunk size: {size_token!r}") from exc
        pos = crlf + 2
        if size == 0:
            break
        if pos + size + 2 > n:
            raise ValueError("truncated chunk payload")
        out.extend(data[pos : pos + size])
        pos += size
        if data[pos : pos + 2] != b"\r\n":
            raise ValueError("missing CRLF after chunk payload")
        pos += 2
    return bytes(out)


def _read_until_sep(sock: socket.socket, sep: bytes, initial: bytes = b"") -> tuple[bytes, bytes]:
    buf = initial
    while sep not in buf:
        chunk = sock.recv(4096)
        if not chunk:
            break
        buf += chunk
    head, found, rest = buf.partition(sep)
    if not found:
        return head, b""
    return head, rest


def _read_exact_from_sock(sock: socket.socket, n: int, initial: bytes) -> tuple[bytes, bytes]:
    buf = bytearray(initial)
    while len(buf) < n:
        chunk = sock.recv(min(65536, n - len(buf)))
        if not chunk:
            break
        buf.extend(chunk)
    return bytes(buf[:n]), bytes(buf[n:])


def _read_chunked_from_sock(sock: socket.socket, initial: bytes) -> bytes:
    buf = bytearray(initial)
    out = bytearray()
    pos = 0

    def _ensure_crlf() -> int:
        while True:
            idx = buf.find(b"\r\n", pos)
            if idx != -1:
                return idx
            chunk = sock.recv(65536)
            if not chunk:
                raise EOFError("unexpected EOF reading chunked header")
            buf.extend(chunk)

    def _ensure_bytes(needed: int) -> None:
        while len(buf) - pos < needed:
            chunk = sock.recv(65536)
            if not chunk:
                raise EOFError("unexpected EOF reading chunked body")
            buf.extend(chunk)

    while True:
        crlf = _ensure_crlf()
        size_token = bytes(buf[pos:crlf]).split(b";", 1)[0].strip()
        size = int(size_token, 16)
        pos = crlf + 2
        if size == 0:
            return bytes(out)
        _ensure_bytes(size + 2)
        out.extend(buf[pos : pos + size])
        pos += size + 2


def read_http_response(sock: socket.socket) -> tuple[int, str, dict[str, str], bytes]:
    """Reads a complete HTTP/1.1 response supporting Content-Length, chunked, and EOF framing."""
    raw_head, rest = _read_until_sep(sock, b"\r\n\r\n")
    status_code, status_line, headers = parse_http_head(raw_head)
    te = headers.get("transfer-encoding", "").lower()
    if "chunked" in te:
        body = _read_chunked_from_sock(sock, rest)
    elif "content-length" in headers:
        length = max(0, int(headers["content-length"].split(",", 1)[0].strip()))
        body, _ = _read_exact_from_sock(sock, length, rest)
    else:
        buf = bytearray(rest)
        while True:
            chunk = sock.recv(65536)
            if not chunk:
                break
            buf.extend(chunk)
        body = bytes(buf)
    return status_code, status_line, headers, body


def http_request_unix(
    socket_name: str,
    path: str = "/json/list",
    *,
    method: str = "GET",
    timeout: float = 4.0,
) -> tuple[int, str, dict[str, str], bytes]:
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(timeout)
    try:
        sock.connect(f"\0{socket_name}")
        req = f"{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n".encode("ascii")
        sock.sendall(req)
        return read_http_response(sock)
    finally:
        sock.close()


def http_get_unix(socket_name: str, path: str = "/json/list", timeout: float = 4.0) -> Any:
    status_code, status_line, _headers, body = http_request_unix(socket_name, path, timeout=timeout)
    if status_code != 200:
        raise RuntimeError(f"CDP HTTP {path} failed: {status_line}")
    return json.loads(body.decode("utf-8"))


def _rewrite_ws_url(url: str, listen_port: int, host: str = "127.0.0.1") -> str:
    parsed = urlparse(url)
    if parsed.scheme not in ("ws", "wss"):
        return url
    return urlunparse((parsed.scheme, f"{host}:{listen_port}", parsed.path, parsed.params, parsed.query, parsed.fragment))


def rewrite_ws_debugger_urls(body: bytes, listen_port: int, host: str = "127.0.0.1") -> bytes:
    """Rewrites webSocketDebuggerUrl fields in /json, /json/list, and /json/version payloads."""
    try:
        payload = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return body

    def _patch(obj: Any) -> None:
        if isinstance(obj, dict) and isinstance(obj.get("webSocketDebuggerUrl"), str):
            obj["webSocketDebuggerUrl"] = _rewrite_ws_url(obj["webSocketDebuggerUrl"], listen_port, host)

    if isinstance(payload, list):
        for item in payload:
            _patch(item)
    elif isinstance(payload, dict):
        _patch(payload)
    else:
        return body
    return json.dumps(payload, ensure_ascii=False).encode("utf-8")


def _handle_cdp_client(client: socket.socket, socket_name: str, listen_port: int) -> None:
    try:
        client.settimeout(5.0)
        raw_head, rest = _read_until_sep(client, b"\r\n\r\n")
        if not raw_head:
            client.close()
            return
        method, target_path, headers = parse_http_request_head(raw_head)
        if headers.get("upgrade", "").lower() == "websocket":
            upstream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            upstream.settimeout(5.0)
            upstream.connect(f"\0{socket_name}")
            initial = raw_head + b"\r\n\r\n" + rest
            cdp_ws.tunnel_bidirectional(client, upstream, initial_a_to_b=initial)
            return

        status_code, status_line, resp_headers, body = http_request_unix(socket_name, target_path, method=method)
        clean_path = target_path.split("?", 1)[0]
        if status_code == 200 and clean_path in REWRITE_PATHS:
            body = rewrite_ws_debugger_urls(body, listen_port)
        ctype = resp_headers.get("content-type", "application/json; charset=UTF-8")
        resp = (
            f"{status_line}\r\n"
            f"Content-Type: {ctype}\r\n"
            f"Content-Length: {len(body)}\r\n"
            "Connection: close\r\n\r\n"
        ).encode("iso-8859-1") + body
        client.sendall(resp)
    except OSError:
        pass
    except Exception:
        try:
            client.sendall(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        except OSError:
            pass
    finally:
        try:
            client.close()
        except OSError:
            pass


def serve_cdp_proxy(port: int, socket_name: str, host: str = "127.0.0.1") -> int:
    """Serves HTTP (rewriting webSocketDebuggerUrl) + WebSocket bidirectional tunnel to abstract UDS."""
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host, int(port)))
    actual_port = srv.getsockname()[1]
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
        threading.Thread(
            target=_handle_cdp_client,
            args=(client, socket_name, actual_port),
            daemon=True,
        ).start()
    return 0
