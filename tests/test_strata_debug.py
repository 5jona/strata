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
from unittest import mock

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEBUG_DIR = os.path.join(REPO_ROOT, "container", "strata-debug")
SCRIPTS_DIR = os.path.join(REPO_ROOT, "scripts")

for p in (DEBUG_DIR, SCRIPTS_DIR):
    if p not in sys.path:
        sys.path.insert(0, p)

import cdp_bridge
import cli
import forward_bridge
import gesture_replay
import kernel_probe
import logcat_cursor
import perfetto_analyze
import perfetto_ops
import perfetto_session
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

    def test_logical_to_physical_rotations_and_clamping(self) -> None:
        pw, ph = 2400, 3392
        # rotation 0: portrait logical 2400x3392 -> physical 2400x3392
        self.assertEqual(gesture_replay.logical_to_physical(0, 0, pw, ph, pw, ph, 0), (0, 0))
        self.assertEqual(gesture_replay.logical_to_physical(100, 200, pw, ph, pw, ph, 0), (100, 200))
        self.assertEqual(gesture_replay.logical_to_physical(9999, -50, pw, ph, pw, ph, 0), (pw - 1, 0))

        # rotation 1 (90 CCW): landscape logical 3392x2400 -> px = phys_w - 1 - y, py = x
        self.assertEqual(gesture_replay.logical_to_physical(0, 0, ph, pw, pw, ph, 1), (pw - 1, 0))
        self.assertEqual(gesture_replay.logical_to_physical(ph - 1, 0, ph, pw, pw, ph, 1), (pw - 1, ph - 1))
        self.assertEqual(gesture_replay.logical_to_physical(0, pw - 1, ph, pw, pw, ph, 1), (0, 0))
        self.assertEqual(gesture_replay.logical_to_physical(ph - 1, pw - 1, ph, pw, pw, ph, 1), (0, ph - 1))
        self.assertEqual(gesture_replay.logical_to_physical(500, 300, ph, pw, pw, ph, 1), (pw - 1 - 300, 500))
        self.assertEqual(gesture_replay.logical_to_physical(-10, 5000, ph, pw, pw, ph, 1), (0, 0))

        # rotation 2 (180 deg): portrait upside down -> px = phys_w - 1 - x, py = phys_h - 1 - y
        self.assertEqual(gesture_replay.logical_to_physical(0, 0, pw, ph, pw, ph, 2), (pw - 1, ph - 1))
        self.assertEqual(gesture_replay.logical_to_physical(100, 200, pw, ph, pw, ph, 2), (pw - 1 - 100, ph - 1 - 200))

        # rotation 3 (270 CCW / 90 CW): landscape logical 3392x2400 -> px = y, py = phys_h - 1 - x
        self.assertEqual(gesture_replay.logical_to_physical(0, 0, ph, pw, pw, ph, 3), (0, ph - 1))
        self.assertEqual(gesture_replay.logical_to_physical(ph - 1, 0, ph, pw, pw, ph, 3), (0, 0))
        self.assertEqual(gesture_replay.logical_to_physical(0, pw - 1, ph, pw, pw, ph, 3), (pw - 1, ph - 1))
        self.assertEqual(gesture_replay.logical_to_physical(500, 300, ph, pw, pw, ph, 3), (300, ph - 1 - 500))

        vp = (
            "  Viewport INTERNAL: displayId=0, orientation=1, "
            "logicalFrame=[0, 0, 3392, 2400], physicalFrame=[0, 0, 2400, 3392], deviceSize=[2400, 3392], isActive=[1]\n"
        )
        self.assertEqual(
            gesture_replay.parse_dumpsys_input_viewport(vp),
            {"logical_w": 3392, "logical_h": 2400, "phys_w": 2400, "phys_h": 3392, "rotation": 1},
        )
        disp = (
            '  mBaseDisplayInfo=DisplayInfo{"Built-in", displayId 0, real 2400 x 3392, rotation 0}\n'
            '  mOverrideDisplayInfo=DisplayInfo{"Built-in", displayId 0, real 3392 x 2400, rotation 1}\n'
        )
        self.assertEqual(
            gesture_replay.parse_dumpsys_display_info(disp),
            {"logical_w": 3392, "logical_h": 2400, "phys_w": 2400, "phys_h": 3392, "rotation": 1},
        )

    def test_kernel_probe_bpffs_detection(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            bpffs = os.path.join(td, "bpf")
            tracing = os.path.join(td, "tracing")
            os.mkdir(bpffs)
            os.mkdir(tracing)
            mounts_mounted = os.path.join(td, "mounts_yes")
            mounts_absent = os.path.join(td, "mounts_no")
            with open(mounts_mounted, "w", encoding="utf-8") as f:
                f.write(f"bpf {bpffs} bpf rw,nosuid,nodev,noexec,relatime 0 0\n")
            with open(mounts_absent, "w", encoding="utf-8") as f:
                f.write("sysfs /sys sysfs rw,nosuid 0 0\n")

            self.assertTrue(kernel_probe.is_bpffs_mounted(bpffs, mounts_path=mounts_mounted))
            self.assertFalse(kernel_probe.is_bpffs_mounted(bpffs, mounts_path=mounts_absent))
            self.assertFalse(kernel_probe.is_bpffs_mounted(os.path.join(td, "missing"), mounts_path=mounts_mounted))

            diag_missing = kernel_probe.inspect_tracing(tracing, bpffs_dir=bpffs, mounts_path=mounts_absent)
            self.assertFalse(diag_missing["bpffsMounted"])
            self.assertIn("strata config set-l5-diag on", diag_missing.get("hint", ""))

            with open(os.path.join(tracing, "current_tracer"), "w", encoding="utf-8") as f:
                f.write("nop\n")
            with open(os.path.join(tracing, "tracing_on"), "w", encoding="utf-8") as f:
                f.write("0\n")
            diag_ok = kernel_probe.inspect_tracing(tracing, bpffs_dir=bpffs, mounts_path=mounts_mounted)
            self.assertTrue(diag_ok["available"])
            self.assertTrue(diag_ok["bpffsMounted"])
            self.assertNotIn("hint", diag_ok)

    def test_perfetto_presets_bounds_and_path_validation(self) -> None:
        expected_tokens = {
            "ui-jank": ["android.surfaceflinger.frametimeline", 'atrace_categories: "gfx"'],
            "sched": ["sched/sched_switch", "power/cpu_frequency"],
            "binder": ["binder/binder_transaction", 'atrace_categories: "binder_lock"'],
            "memory": ["lowmemorykiller/lowmemory_kill", "linux.sys_stats"],
        }
        for preset, tokens in expected_tokens.items():
            cfg = perfetto_ops.build_perfetto_config(preset, duration_sec=3, buffer_kb=8192)
            self.assertIn("size_kb: 8192", cfg)
            self.assertIn("duration_ms: 3000", cfg)
            for tok in tokens:
                self.assertIn(tok, cfg)

        # Boundary validation
        self.assertEqual(perfetto_ops.validate_duration(1), 1)
        self.assertEqual(perfetto_ops.validate_duration(15), 15)
        for bad_dur in (0, -1, 16, True):
            with self.assertRaises(ValueError):
                perfetto_ops.validate_duration(bad_dur)  # type: ignore[arg-type]

        self.assertEqual(perfetto_ops.validate_buffer_kb(1024), 1024)
        self.assertEqual(perfetto_ops.validate_buffer_kb(32768), 32768)
        for bad_buf in (512, 65536, False):
            with self.assertRaises(ValueError):
                perfetto_ops.validate_buffer_kb(bad_buf)  # type: ignore[arg-type]

        with self.assertRaises(ValueError):
            perfetto_ops.build_perfetto_config("invalid-preset")

        # Output path validation
        default_path = perfetto_ops.validate_out_path(None, "ui-jank", ts=1700000000)
        self.assertEqual(default_path, "/data/local/tmp/strata-perfetto-ui-jank-1700000000.perfetto-trace")
        self.assertEqual(
            perfetto_ops.validate_out_path("/data/local/tmp/custom.perfetto-trace", "sched"),
            "/data/local/tmp/custom.perfetto-trace",
        )
        for bad_path in (
            "/tmp/escape.trace",
            "/data/local/tmp/../etc/passwd",
            "/data/local/tmp/foo;rm -rf /",
            "/data/local/tmp/",
            "",
        ):
            with self.assertRaises(ValueError):
                perfetto_ops.validate_out_path(bad_path, "ui-jank")

        # CLI parser accepts perfetto subcommand (default capture + ring-buffer/analyze actions)
        parsed = cli.build_parser().parse_args(
            ["perfetto", "--preset", "binder", "--duration", "5", "--buffer-kb", "4096", "--out", "/data/local/tmp/a.trace"]
        )
        self.assertEqual(
            (parsed.cmd, parsed.action, parsed.preset, parsed.duration, parsed.buffer_kb),
            ("perfetto", "capture", "binder", 5, 4096),
        )
        p_start = cli.build_parser().parse_args(
            ["perfetto", "--action", "start", "--preset", "ui-jank", "--session", "ring1", "--max-duration", "600"]
        )
        self.assertEqual((p_start.action, p_start.session, p_start.max_duration), ("start", "ring1", 600))
        p_an = cli.build_parser().parse_args(["perfetto", "--action", "analyze", "--path", "/data/local/tmp/t.trace"])
        self.assertEqual((p_an.action, p_an.path), ("analyze", "/data/local/tmp/t.trace"))

    def test_perfetto_protobuf_analyzer(self) -> None:
        def _varint(val: int) -> bytes:
            out = bytearray()
            while True:
                b = val & 0x7F
                val >>= 7
                if val:
                    out.append(b | 0x80)
                else:
                    out.append(b)
                    break
            return bytes(out)

        def _ld(field_no: int, payload: bytes) -> bytes:
            return _varint((field_no << 3) | 2) + _varint(len(payload)) + payload

        def _vi(field_no: int, val: int) -> bytes:
            return _varint((field_no << 3) | 0) + _varint(val)

        # Build FtraceEventBundle with 2 sched_switch(4), 1 print(16), 1 binder_transaction(67)
        ev1 = _vi(1, 1_000_000_000) + _ld(4, b"\x00")
        ev2 = _vi(1, 1_100_000_000) + _ld(4, b"\x00")
        ev3 = _vi(1, 1_200_000_000) + _ld(16, b"B|1|draw")
        ev4 = _vi(1, 1_300_000_000) + _ld(67, b"\x00")
        bundle = _vi(1, 0) + _ld(2, ev1) + _ld(2, ev2) + _ld(2, ev3) + _ld(2, ev4)

        pkt1 = _vi(8, 1_000_000_000) + _vi(10, 1) + _ld(1, bundle)
        pkt2 = _vi(8, 1_250_000_000) + _vi(10, 2) + _ld(2, b"\x08\x01")
        pkt3 = _vi(8, 1_500_000_000) + _vi(10, 3) + _ld(53, b"\x08\x01")
        trace_bytes = _ld(1, pkt1) + _ld(1, pkt2) + _ld(1, pkt3)

        with tempfile.NamedTemporaryFile("wb", suffix=".perfetto-trace", delete=False) as tf:
            tf.write(trace_bytes)
            trace_path = tf.name
        try:
            with mock.patch("shutil.which", return_value=None):
                res = perfetto_analyze.analyze_trace_file(trace_path)
            self.assertTrue(res["ok"])
            self.assertEqual(res["analyzer"], "pure-python-protobuf")
            self.assertEqual(res["sizeBytes"], len(trace_bytes))
            self.assertEqual(res["packetCount"], 3)
            self.assertEqual(res["traceDurationMs"], 500.0)
            self.assertEqual(res["packetTypes"], {"ftrace_events": 1, "process_tree": 1, "surfaceflinger_layers": 1})
            self.assertEqual(res["ftraceEventCounts"], {"sched_switch": 2, "print": 1, "binder_transaction": 1})
            self.assertTrue(any("3 packets over 500.0 ms" in h for h in res["highlights"]))
        finally:
            os.unlink(trace_path)

    def test_perfetto_ring_session_lifecycle(self) -> None:
        cfg = perfetto_session.build_ring_config("ui-jank", buffer_kb=16384, max_duration_sec=1800)
        self.assertIn("fill_policy: RING_BUFFER", cfg)
        self.assertIn("duration_ms: 1800000", cfg)
        self.assertNotIn("write_into_file", cfg)

        for bad_dur in (59, 3601, True):
            with self.assertRaises(ValueError):
                perfetto_session.validate_max_duration(bad_dur)  # type: ignore[arg-type]
        for bad_key in ("", "bad key", "../evil", "a" * 33):
            with self.assertRaises(ValueError):
                perfetto_session.validate_session_key(bad_key)

        with tempfile.TemporaryDirectory() as td:
            calls: list[list[str]] = []
            running = {"active": False}

            def fake_run_host(argv: list[str], *, input_text: str | None = None, timeout: float = 15.0):
                calls.append(argv)
                if "--background" in argv:
                    running["active"] = True
                    return subprocess.CompletedProcess(argv, 0, "42424\n", "")
                if argv[:2] == ["/system/bin/sh", "-c"] and "kill -0" in argv[2] and "kill -INT" not in argv[2]:
                    return subprocess.CompletedProcess(argv, 0 if running["active"] else 1, "", "")
                if argv[:2] == ["/system/bin/sh", "-c"] and "kill -INT" in argv[2]:
                    running["active"] = False
                    return subprocess.CompletedProcess(argv, 0, "", "")
                return subprocess.CompletedProcess(argv, 0, "", "")

            with (
                mock.patch.object(perfetto_session, "_run_host", side_effect=fake_run_host),
                mock.patch.object(perfetto_ops, "stat_remote_size", return_value=4096),
                mock.patch.object(
                    perfetto_analyze,
                    "analyze_trace_file",
                    return_value={"ok": True, "packetCount": 12, "traceDurationMs": 250.0, "highlights": ["ok"]},
                ),
            ):
                s1 = perfetto_session.start_ring_session(
                    preset="sched", buffer_kb=16384, max_duration_sec=600,
                    session_key="ring_test", out_path="/data/local/tmp/ring1.perfetto-trace", meta_dir=td,
                )
                self.assertTrue(s1["ok"])
                self.assertEqual(s1["action"], "start")
                self.assertTrue(os.path.isfile(perfetto_session.session_meta_path("ring_test", td)))

                # Duplicate start rejected while active
                dup = perfetto_session.start_ring_session(session_key="ring_test", meta_dir=td)
                self.assertFalse(dup["ok"])
                self.assertTrue(dup["active"])

                st = perfetto_session.ring_session_status("ring_test", meta_dir=td)
                self.assertTrue(st["ok"])
                self.assertTrue(st["active"])
                self.assertEqual(st["preset"], "sched")

                snap = perfetto_session.snapshot_ring_session("ring_test", keep_running=True, analyze=True, meta_dir=td)
                self.assertTrue(snap["ok"])
                self.assertEqual(snap["action"], "snapshot")
                self.assertTrue(snap["restarted"])
                self.assertEqual(snap["summary"]["packetCount"], 12)
                self.assertTrue(snap["nextRemotePath"].startswith("/data/local/tmp/strata-perfetto-sched-"))

                stop = perfetto_session.stop_ring_session("ring_test", analyze=True, meta_dir=td)
                self.assertTrue(stop["ok"])
                self.assertEqual(stop["sizeBytes"], 4096)
                self.assertFalse(os.path.exists(perfetto_session.session_meta_path("ring_test", td)))

            self.assertTrue(any("--background" in c and "--txt" in c for c in calls))
            self.assertTrue(any(len(c) >= 3 and "kill -INT" in c[2] for c in calls))

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
