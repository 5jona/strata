"""Host-side unit tests for forward bridge, MCP stdio handshake, Perfetto, and verify_live runner."""

from __future__ import annotations

import json
import os
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

import cli
import forward_bridge
import perfetto_analyze
import perfetto_ops
import perfetto_session
from verify_live.runner import DeviceRunner, resolve_mode


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
