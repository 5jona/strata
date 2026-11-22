"""Unit tests for CDP HTTP/1.1 framing, URL rewriting, RFC 6455 WebSocket, and selectors tunnel."""

from __future__ import annotations

import json
import os
import socket
import sys
import threading
import unittest
from unittest import mock

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEBUG_DIR = os.path.join(REPO_ROOT, "container", "strata-debug")
if DEBUG_DIR not in sys.path:
    sys.path.insert(0, DEBUG_DIR)

import cdp_bridge
import cdp_http
import cdp_ws


class TestCdpHttp(unittest.TestCase):
    def test_decode_chunked_body_and_extensions(self) -> None:
        raw = b"5;ext=1\r\nHello\r\n7\r\n, world\r\n0\r\nX-Trailer: ok\r\n\r\n"
        self.assertEqual(cdp_http.decode_chunked_body(raw), b"Hello, world")

    def test_read_http_response_content_length_and_chunked(self) -> None:
        # Content-Length (without closing socket, must not hang)
        a, b = socket.socketpair()
        try:
            a.settimeout(2.0)
            payload = b'[{"id":"1"}]'
            b.sendall(
                f"HTTP/1.1 200 OK\r\nContent-Length: {len(payload)}\r\n\r\n".encode() + payload
            )
            code, _line, _hdrs, body = cdp_http.read_http_response(a)
            self.assertEqual(code, 200)
            self.assertEqual(body, payload)
        finally:
            a.close()
            b.close()

        # Transfer-Encoding: chunked (without closing socket, must not hang)
        a, b = socket.socketpair()
        try:
            a.settimeout(2.0)
            b.sendall(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
                b"4\r\n[{\"a\r\n7\r\n\":123}]\r\n0\r\n\r\n"
            )
            code, _line, _hdrs, body = cdp_http.read_http_response(a)
            self.assertEqual(code, 200)
            self.assertEqual(json.loads(body.decode()), [{"a": 123}])
        finally:
            a.close()
            b.close()

    def test_rewrite_ws_debugger_urls_list_and_dict(self) -> None:
        list_body = json.dumps([
            {"id": "p1", "webSocketDebuggerUrl": "ws://localhost/devtools/page/ABC?x=1"},
            {"id": "p2", "title": "no-ws"},
        ]).encode()
        rewritten = json.loads(cdp_http.rewrite_ws_debugger_urls(list_body, 9333).decode())
        self.assertEqual(rewritten[0]["webSocketDebuggerUrl"], "ws://127.0.0.1:9333/devtools/page/ABC?x=1")
        self.assertNotIn("webSocketDebuggerUrl", rewritten[1])

        ver_body = json.dumps({"webSocketDebuggerUrl": "ws://[::1]:9222/devtools/browser/1"}).encode()
        rewritten_ver = json.loads(cdp_http.rewrite_ws_debugger_urls(ver_body, 9444).decode())
        self.assertEqual(rewritten_ver["webSocketDebuggerUrl"], "ws://127.0.0.1:9444/devtools/browser/1")


class TestCdpWebSocket(unittest.TestCase):
    def test_ws_frame_encode_decode_lengths_and_masking(self) -> None:
        for size in (0, 12, 125, 126, 500, 65535, 65536, 70000):
            payload = (b"abcdef0123" * ((size // 10) + 1))[:size]
            # Masked (client -> server)
            frame = cdp_ws.encode_ws_frame(cdp_ws.OPCODE_TEXT, payload, mask_key=b"\x12\x34\x56\x78")
            fin, op, decoded, rem = cdp_ws.decode_ws_frame(frame)
            self.assertTrue(fin)
            self.assertEqual(op, cdp_ws.OPCODE_TEXT)
            self.assertEqual(decoded, payload)
            self.assertEqual(rem, b"")
            # Unmasked (server -> client)
            frame_u = cdp_ws.encode_ws_frame(cdp_ws.OPCODE_BIN, payload, fin=False, mask_key=b"")
            fin2, op2, decoded2, _ = cdp_ws.decode_ws_frame(frame_u)
            self.assertFalse(fin2)
            self.assertEqual(op2, cdp_ws.OPCODE_BIN)
            self.assertEqual(decoded2, payload)

    def test_ws_client_fragmentation_ping_pong_and_close(self) -> None:
        client_sock, server_sock = socket.socketpair()
        received_pong: list[bytes] = []

        def _server() -> None:
            try:
                # Read HTTP upgrade request
                req = b""
                while b"\r\n\r\n" not in req:
                    req += server_sock.recv(4096)
                server_sock.sendall(
                    b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n"
                )
                # Read client CDP call frame
                buf = server_sock.recv(4096)
                _fin, _op, payload, _ = cdp_ws.decode_ws_frame(buf)
                msg = json.loads(payload.decode())
                # Send PING first
                server_sock.sendall(cdp_ws.encode_ws_frame(cdp_ws.OPCODE_PING, b"hi", mask_key=b""))
                pong_raw = server_sock.recv(4096)
                _pf, pop, pdata, _ = cdp_ws.decode_ws_frame(pong_raw)
                if pop == cdp_ws.OPCODE_PONG:
                    received_pong.append(pdata)
                # Send fragmented response split across 2 frames
                resp_bytes = json.dumps({"id": msg["id"], "result": {"ok": True}}).encode()
                mid = len(resp_bytes) // 2
                server_sock.sendall(cdp_ws.encode_ws_frame(cdp_ws.OPCODE_TEXT, resp_bytes[:mid], fin=False, mask_key=b""))
                server_sock.sendall(cdp_ws.encode_ws_frame(cdp_ws.OPCODE_CONT, resp_bytes[mid:], fin=True, mask_key=b""))
                # Send CLOSE frame
                server_sock.sendall(cdp_ws.encode_ws_frame(cdp_ws.OPCODE_CLOSE, b"\x03\xe8", mask_key=b""))
            finally:
                server_sock.close()

        t = threading.Thread(target=_server, daemon=True)
        t.start()
        ws = cdp_ws.CdpWsClient(client_sock, "/devtools/page/1", timeout=2.0)
        try:
            res = ws.call("Runtime.enable", timeout=2.0)
            self.assertEqual(res["result"], {"ok": True})
            self.assertEqual(received_pong, [b"hi"])
            self.assertIsNone(ws.recv_message(deadline=None))
        finally:
            ws.close()
            t.join(timeout=2.0)

    def test_tunnel_bidirectional_selectors(self) -> None:
        c_end, p_a = socket.socketpair()
        p_b, s_end = socket.socketpair()
        t = threading.Thread(
            target=cdp_ws.tunnel_bidirectional,
            args=(p_a, p_b),
            kwargs={"initial_a_to_b": b"HEAD:"},
            daemon=True,
        )
        t.start()
        try:
            c_end.settimeout(2.0)
            s_end.settimeout(2.0)
            c_end.sendall(b"ping")
            c_end.shutdown(socket.SHUT_WR)
            got = b""
            while len(got) < 9:
                got += s_end.recv(1024)
            self.assertEqual(got, b"HEAD:ping")
            s_end.sendall(b"pong")
            s_end.shutdown(socket.SHUT_WR)
            reply = b""
            while chunk := c_end.recv(1024):
                reply += chunk
            self.assertEqual(reply, b"pong")
        finally:
            c_end.close()
            s_end.close()
            t.join(timeout=2.0)

    def test_bridge_no_sockets_clean_error_and_list_targets(self) -> None:
        with mock.patch.object(cdp_bridge, "scan_devtools_sockets", return_value=[]):
            listed = cdp_bridge.list_targets()
            self.assertEqual(listed, {"ok": True, "sockets": [], "targets": []})
            ev = cdp_bridge.eval_js("1+1")
            self.assertFalse(ev["ok"])
            self.assertIn("error", ev)
            con = cdp_bridge.collect_console(duration_s=0.1)
            self.assertFalse(con["ok"])
            self.assertIn("error", con)


if __name__ == "__main__":
    unittest.main()
