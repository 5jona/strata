"""Host-side unit tests for Strata L1-L5 debug toolkit, MCP server, and verify_live."""

from __future__ import annotations

import importlib.util
import json
import os
import struct
import subprocess
import sys
import tempfile
import unittest

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEBUG_DIR = os.path.join(REPO_ROOT, "container", "strata-debug")
SCRIPTS_DIR = os.path.join(REPO_ROOT, "scripts")

for p in (DEBUG_DIR, SCRIPTS_DIR):
    if p not in sys.path:
        sys.path.insert(0, p)

import cdp_bridge
import forward_bridge
import gesture_replay
import kernel_probe
import logcat_cursor
import sys_probe
import visual_check
from verify_live.runner import DeviceRunner, resolve_mode


def _load_module(name: str, path: str):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class TestVisualCheck(unittest.TestCase):
    def test_header_12_and_16_bytes_roi_health_ssim_and_png(self) -> None:
        w, h = 16, 16
        # Checkerboard pattern (sharp, non-blank)
        pixels = bytearray()
        for y in range(h):
            for x in range(w):
                v = 240 if (x + y) % 2 == 0 else 20
                pixels.extend((v, v, v, 255))
        raw12 = struct.pack("<III", w, h, 1) + bytes(pixels)
        raw16 = struct.pack("<IIII", w, h, 1, 0) + bytes(pixels)

        pw, ph, fmt, rgba12 = visual_check.parse_screencap_header(raw12)
        self.assertEqual((pw, ph, fmt, len(rgba12)), (16, 16, 1, w * h * 4))
        pw2, ph2, _, rgba16 = visual_check.parse_screencap_header(raw16)
        self.assertEqual((pw2, ph2, rgba16), (16, 16, rgba12))

        crop, cw, ch = visual_check.crop_rgba(rgba16, w, h, (4, 4, 8, 8))
        self.assertEqual((cw, ch, len(crop)), (8, 8, 8 * 8 * 4))

        health = visual_check.analyze_rgba_health(crop, cw, ch)
        self.assertFalse(health["isBlack"])
        self.assertFalse(health["isBlank"])
        self.assertGreater(health["laplacianVar"], 100.0)

        black = b"\x00\x00\x00\xff" * (8 * 8)
        b_health = visual_check.analyze_rgba_health(black, 8, 8)
        self.assertTrue(b_health["isBlack"])
        self.assertTrue(b_health["isBlank"])

        self.assertAlmostEqual(visual_check.compute_ssim_luma(crop, crop, 8, 8)["ssim"], 1.0, places=3)
        self.assertLess(visual_check.compute_ssim_luma(crop, black, 8, 8)["ssim"], 0.2)

        png = visual_check.encode_png_pure(crop, cw, ch)
        self.assertTrue(png.startswith(b"\x89PNG\r\n\x1a\n"))

        sheet, sw, sh = visual_check.stitch_burst_sheet([(crop, cw, ch), (black, 8, 8)])
        self.assertEqual((sw, sh, len(sheet)), (16, 16, 16 * 16 * 4))


class TestLogcatAndSysProbe(unittest.TestCase):
    def test_logcat_parse_and_incremental_filter(self) -> None:
        lines = [
            "11-22 10:00:01.100  1234  1235 I SideVeil: panel opened",
            "11-22 10:00:01.200  1234  1235 W SideVeil: blur fallback",
            "11-22 10:00:01.300  5678  5679 E Strata: mount warning",
        ]
        parsed = logcat_cursor.parse_threadtime_line(lines[1])
        assert parsed is not None
        self.assertEqual(parsed["pid"], 1234)
        self.assertEqual(parsed["priority"], "W")
        self.assertEqual(parsed["tag"], "SideVeil")

        entries, ts, digest = logcat_cursor.filter_logcat_lines(lines, priority="W")
        self.assertEqual(len(entries), 2)
        self.assertEqual(ts, "11-22 10:00:01.300")
        # Incremental call with last_digest returns empty when no new lines arrived
        entries2, _, _ = logcat_cursor.filter_logcat_lines(lines, priority="W", last_digest=digest)
        self.assertEqual(entries2, [])

    def test_dumpsys_parsers(self) -> None:
        win_dump = (
            "  Window #0 Window{abcd u0 pochita.sideveil.host/OverlayPanel}:\n"
            "    mViewVisibility=0x0 mHasSurface=true isOnScreen=true\n"
            "    mFrame=[0,120][1080,2200] ty=TYPE_APPLICATION_OVERLAY mBaseLayer=21000\n"
            "  Window #1 Window{ef01 u0 com.hidden/Act}:\n"
            "    mViewVisibility=0x8 isOnScreen=false\n"
        )
        wins = sys_probe.parse_window_windows(win_dump)
        self.assertEqual(len(wins), 1)
        self.assertEqual(wins[0]["frame"], {"x": 0, "y": 120, "w": 1080, "h": 2080})
        self.assertEqual(wins[0]["layer"], 21000)

        gfx = sys_probe.parse_gfxinfo(
            "Total frames rendered: 120\nJanky frames: 3 (2.50%)\n50th percentile: 6ms\n99th percentile: 14ms\n"
        )
        self.assertEqual(gfx["totalFrames"], 120)
        self.assertEqual(gfx["jankyPercent"], 2.5)
        self.assertEqual(gfx["p99Ms"], 14)


class TestGestureAndKernelAndCdp(unittest.TestCase):
    def test_uinput_structs_and_edge_swipe(self) -> None:
        ev = gesture_replay.pack_input_event(gesture_replay.EV_KEY, gesture_replay.BTN_TOUCH, 1)
        self.assertEqual(len(ev), 24)
        self.assertEqual(len(gesture_replay.pack_uinput_setup()), 92)
        self.assertEqual(len(gesture_replay.pack_uinput_abs_setup(gesture_replay.ABS_MT_POSITION_X, 0, 1079)), 28)

        pts = gesture_replay.compute_edge_swipe_points(1080, 2400, 420, side="right", cancel_back=True)
        self.assertGreater(len(pts), 12)
        self.assertEqual(pts[0][0], 1078)
        self.assertEqual(pts[-1][0], 1078)

    def test_pstore_and_devtools_scan(self) -> None:
        sample = "normal boot line\noplus_secure_guard: malicious_app_try_to_root_devices\nKernel panic - not syncing\n"
        res = kernel_probe.parse_pstore_text(sample)
        self.assertEqual(len(res["watchdogHits"]), 2)

        with tempfile.NamedTemporaryFile("w", delete=False) as tf:
            tf.write("Num RefCount Protocol Flags Type St Inode Path\n")
            tf.write("0000: 00000002 00000000 00010000 0001 01 12345 @webview_devtools_remote_4321\n")
            path = tf.name
        try:
            socks = cdp_bridge.scan_devtools_sockets(path)
            self.assertEqual(len(socks), 1)
            self.assertEqual(socks[0]["socket"], "webview_devtools_remote_4321")
            self.assertEqual(socks[0]["pid"], 4321)
        finally:
            os.unlink(path)


class TestForwardAndMcpAndRunner(unittest.TestCase):
    def test_forward_arg_parsing_and_list(self) -> None:
        parsed = forward_bridge.parse_forward_args(["tcp:9222", "localabstract:webview_devtools_remote_99"])
        self.assertEqual(parsed["action"], "add")
        self.assertEqual(forward_bridge.parse_endpoint(parsed["local"]), ("tcp", 9222))
        self.assertEqual(forward_bridge.parse_endpoint(parsed["remote"]), ("localabstract", "webview_devtools_remote_99"))
        self.assertEqual(forward_bridge.parse_forward_args(["--list"])["action"], "list")
        self.assertEqual(forward_bridge.parse_forward_args(["--remove", "tcp:9222"])["action"], "remove")
        self.assertEqual(forward_bridge.parse_forward_args(["--remove-all"])["action"], "remove-all")
        with self.assertRaises(ValueError):
            forward_bridge.parse_forward_args(["bad:spec", "tcp:1"])
        with tempfile.TemporaryDirectory() as td:
            rc, out = forward_bridge.handle_forward(["--list"], state_path=os.path.join(td, "fwd.json"))
            self.assertEqual((rc, out), (0, ""))

    def test_mcp_stdio_initialize_and_tools_list(self) -> None:
        mcp_script = os.path.join(SCRIPTS_DIR, "strata-debug-mcp.py")
        reqs = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            {"jsonrpc": "2.0", "id": 3, "method": "unknown/method"},
        ]
        stdin_payload = "\n".join(json.dumps(r) for r in reqs) + "\n"
        proc = subprocess.run(
            [sys.executable, mcp_script],
            input=stdin_payload,
            capture_output=True,
            text=True,
            check=True,
        )
        out = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
        self.assertEqual(len(out), 3)
        self.assertEqual(out[0]["id"], 1)
        self.assertEqual(out[0]["result"]["serverInfo"]["name"], "strata-debug-mcp")
        tool_names = {t["name"] for t in out[1]["result"]["tools"]}
        self.assertTrue({"observe_step", "cdp_audit", "gesture_replay", "kernel_probe", "pstore_dump"}.issubset(tool_names))
        self.assertEqual(out[2]["error"]["code"], -32601)

    def test_verify_live_runner_modes(self) -> None:
        self.assertEqual(resolve_mode("auto", "192.0.2.10"), "ssh")
        with self.assertRaises(SystemExit):
            resolve_mode("legacy", None)
        runner = DeviceRunner(mode="ssh", ssh_target="192.0.2.10", ssh_port=2222)
        argv = runner.build_host_argv("id")
        self.assertEqual(argv[0], "ssh")
        self.assertIn("root@192.0.2.10", argv)
        self.assertIn("/usr/local/bin/host-exec", argv[-1])


if __name__ == "__main__":
    unittest.main()
