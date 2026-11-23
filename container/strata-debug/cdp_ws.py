"""RFC 6455 WebSocket framing, client, and non-blocking selectors bidirectional tunnel."""

from __future__ import annotations

import base64
import contextlib
import hashlib
import json
import os
import selectors
import socket
import struct
import time
from typing import Any, Callable

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
OPCODE_CONT, OPCODE_TEXT, OPCODE_BIN = 0x0, 0x1, 0x2
OPCODE_CLOSE, OPCODE_PING, OPCODE_PONG = 0x8, 0x9, 0xA


def compute_ws_accept(sec_key: str) -> str:
    return base64.b64encode(hashlib.sha1((sec_key + WS_GUID).encode("ascii")).digest()).decode("ascii")


def apply_ws_mask(payload: bytes, mask_key: bytes) -> bytes:
    return bytes(b ^ mask_key[i & 3] for i, b in enumerate(payload)) if mask_key else payload


def encode_ws_frame(
    opcode: int, payload: bytes, *, fin: bool = True, mask_key: bytes | None = None
) -> bytes:
    """Encodes a single RFC 6455 frame. Random 4-byte mask if mask_key is None; b'' for unmasked."""
    if mask_key is None:
        mask_key = os.urandom(4)
    if mask_key and len(mask_key) != 4:
        raise ValueError("WebSocket mask key must be 4 bytes")
    masked_bit = 0x80 if mask_key else 0x00
    head = bytearray([(0x80 if fin else 0x00) | (opcode & 0x0F)])
    length = len(payload)
    if length < 126:
        head.append(masked_bit | length)
    elif length < 65536:
        head.append(masked_bit | 126)
        head.extend(struct.pack("!H", length))
    else:
        head.append(masked_bit | 127)
        head.extend(struct.pack("!Q", length))
    if mask_key:
        return bytes(head) + mask_key + apply_ws_mask(payload, mask_key)
    return bytes(head) + payload


def read_ws_frame(read_exact: Callable[[int], bytes]) -> tuple[bool, int, bytes]:
    """Reads one RFC 6455 frame using read_exact(n) -> (fin, opcode, unmasked_payload)."""
    b0, b1 = read_exact(2)
    fin, opcode, masked, length = (b0 & 0x80) != 0, b0 & 0x0F, (b1 & 0x80) != 0, b1 & 0x7F
    if length == 126:
        length = struct.unpack("!H", read_exact(2))[0]
    elif length == 127:
        length = struct.unpack("!Q", read_exact(8))[0]
    mask_key = read_exact(4) if masked else b""
    data = read_exact(length) if length > 0 else b""
    return fin, opcode, apply_ws_mask(data, mask_key) if masked else data


def decode_ws_frame(raw: bytes) -> tuple[bool, int, bytes, bytes]:
    """Decodes one RFC 6455 frame from a byte buffer -> (fin, opcode, payload, remainder)."""
    pos = 0

    def _take(n: int) -> bytes:
        nonlocal pos
        if pos + n > len(raw):
            raise EOFError("incomplete WebSocket frame")
        chunk = raw[pos : pos + n]
        pos += n
        return chunk

    fin, opcode, payload = read_ws_frame(_take)
    return fin, opcode, payload, raw[pos:]


class CdpWsClient:
    """RFC 6455 WebSocket client over abstract UDS or TCP with fragmentation and timeout support."""

    def __init__(self, sock: socket.socket, ws_path: str, timeout: float = 5.0) -> None:
        self.sock = sock
        self.default_timeout = timeout
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
        return cls(sock, ws_path, timeout=timeout)

    def _handshake(self, ws_path: str) -> None:
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        req = (
            f"GET {ws_path} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        ).encode("ascii")
        self.sock.sendall(req)
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise EOFError("WebSocket handshake closed before response")
            buf += chunk
        header, _, rest = buf.partition(b"\r\n\r\n")
        self._buf = rest
        lines = header.splitlines()
        if not lines or b"101" not in lines[0]:
            raise RuntimeError(f"WebSocket upgrade rejected: {header.decode(errors='replace')}")

    def _read_exact(self, n: int, deadline: float | None = None) -> bytes:
        while len(self._buf) < n:
            if deadline is not None:
                rem = deadline - time.monotonic()
                if rem <= 0:
                    raise TimeoutError("WebSocket read timed out")
                self.sock.settimeout(rem)
            try:
                chunk = self.sock.recv(65536)
            except socket.timeout as exc:
                raise TimeoutError("WebSocket read timed out") from exc
            if not chunk:
                raise EOFError("WebSocket closed")
            self._buf += chunk
        out, self._buf = self._buf[:n], self._buf[n:]
        return out

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        self.sock.sendall(encode_ws_frame(opcode, payload, fin=True))

    def recv_message(self, deadline: float | None = None) -> str | None:
        """Receives a complete (possibly fragmented) text message; handles PING/PONG/CLOSE."""
        fragments: list[bytes] = []
        frag_opcode: int | None = None
        while True:
            fin, opcode, data = read_ws_frame(lambda n: self._read_exact(n, deadline))
            if opcode == OPCODE_CLOSE:
                with contextlib.suppress(OSError):
                    self._send_frame(OPCODE_CLOSE, data[:2])
                return None
            if opcode == OPCODE_PING:
                self._send_frame(OPCODE_PONG, data)
                continue
            if opcode == OPCODE_PONG:
                continue
            if opcode == OPCODE_CONT and frag_opcode is not None:
                fragments.append(data)
                if fin:
                    full, is_text = b"".join(fragments), frag_opcode == OPCODE_TEXT
                    fragments, frag_opcode = [], None
                    if is_text:
                        return full.decode("utf-8", errors="replace")
            elif opcode in (OPCODE_TEXT, OPCODE_BIN):
                if fin and opcode == OPCODE_TEXT:
                    return data.decode("utf-8", errors="replace")
                if not fin:
                    frag_opcode, fragments = opcode, [data]

    def _record_event(self, msg: dict[str, Any]) -> None:
        method = msg.get("method")
        if method == "Runtime.consoleAPICalled":
            self.console_events.append(msg.get("params", {}))
        elif method == "Runtime.exceptionThrown":
            self.exceptions.append(msg.get("params", {}))

    def call(
        self, method: str, params: dict[str, Any] | None = None, timeout: float | None = None
    ) -> dict[str, Any]:
        req_id = self._next_id
        self._next_id += 1
        eff = self.default_timeout if timeout is None else timeout
        self._send_frame(OPCODE_TEXT, json.dumps({"id": req_id, "method": method, "params": params or {}}).encode())
        deadline = time.monotonic() + eff
        while True:
            raw = self.recv_message(deadline=deadline)
            if raw is None:
                raise EOFError("CDP WebSocket closed before response")
            msg = json.loads(raw)
            if msg.get("id") == req_id:
                return msg
            self._record_event(msg)

    def drain_events(self, duration_s: float = 1.0) -> None:
        deadline = time.monotonic() + max(0.05, duration_s)
        while time.monotonic() < deadline:
            try:
                raw = self.recv_message(deadline=deadline)
            except (TimeoutError, EOFError):
                break
            if raw is None:
                break
            try:
                self._record_event(json.loads(raw))
            except json.JSONDecodeError:
                continue

    def close(self) -> None:
        with contextlib.suppress(OSError):
            self.sock.settimeout(0.5)
            self._send_frame(OPCODE_CLOSE, b"")
        with contextlib.suppress(OSError):
            self.sock.close()


def _safe_shut_wr(sock: socket.socket) -> None:
    with contextlib.suppress(OSError):
        sock.shutdown(socket.SHUT_WR)


def tunnel_bidirectional(
    sock_a: socket.socket,
    sock_b: socket.socket,
    *,
    initial_a_to_b: bytes = b"",
    initial_b_to_a: bytes = b"",
    idle_timeout: float = 300.0,
) -> None:
    """Non-blocking selectors-based bidirectional byte tunnel with half-close handling."""
    sel = selectors.DefaultSelector()
    peers = {sock_a: sock_b, sock_b: sock_a}
    outbuf = {sock_a: bytearray(initial_b_to_a), sock_b: bytearray(initial_a_to_b)}
    eof_read = {sock_a: False, sock_b: False}
    shut_wr = {sock_a: False, sock_b: False}

    def _sync(s: socket.socket) -> None:
        ev = (selectors.EVENT_READ if not eof_read[s] and len(outbuf[peers[s]]) < 262144 else 0) | (
            selectors.EVENT_WRITE if outbuf[s] else 0
        )
        try:
            sel.modify(s, ev) if ev else sel.unregister(s)
        except KeyError:
            if ev:
                sel.register(s, ev)

    try:
        sock_a.setblocking(False)
        sock_b.setblocking(False)
        _sync(sock_a)
        _sync(sock_b)
        while sel.get_map():
            events = sel.select(timeout=idle_timeout)
            if not events:
                break
            for key, mask in events:
                s: socket.socket = key.fileobj  # type: ignore[assignment]
                peer = peers[s]
                if mask & selectors.EVENT_READ:
                    try:
                        chunk: bytes | None = s.recv(65536)
                    except BlockingIOError:
                        chunk = None
                    except OSError:
                        chunk = b""
                    if chunk == b"":
                        eof_read[s] = True
                        if not outbuf[peer] and not shut_wr[peer]:
                            _safe_shut_wr(peer)
                            shut_wr[peer] = True
                    elif chunk:
                        outbuf[peer].extend(chunk)
                if (mask & selectors.EVENT_WRITE) and outbuf[s]:
                    try:
                        del outbuf[s][: s.send(outbuf[s])]
                    except BlockingIOError:
                        pass
                    except OSError:
                        outbuf[s].clear()
                        eof_read[peer] = True
                    if not outbuf[s] and eof_read[peer] and not shut_wr[s]:
                        _safe_shut_wr(s)
                        shut_wr[s] = True
                _sync(s)
                _sync(peer)
            if eof_read[sock_a] and eof_read[sock_b] and not outbuf[sock_a] and not outbuf[sock_b]:
                break
    finally:
        sel.close()
        for s in (sock_a, sock_b):
            with contextlib.suppress(OSError):
                s.close()
