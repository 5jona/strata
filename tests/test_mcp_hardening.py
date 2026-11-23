"""Unit tests for MCP/strata-debug hardening: perfetto --session, ControlMaster cleanup, JSON, APK, visual fallback."""

from __future__ import annotations

import io
import json
import os
import shlex
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

import cli as debug_cli
import perfetto_analyze
import perfetto_session
import visual_check
from strata_mcp import devices, dispatch_tool, transport


class TestPerfettoAnalyzeSessionAndArgvContract(unittest.TestCase):
    def test_mcp_perfetto_argv_parses_with_strata_debug_argparse(self) -> None:
        parser = debug_cli.build_parser()
        cases = [
            {"action": "capture", "preset": "ui-jank", "durationSec": 4, "bufferKb": 8192, "analyze": True},
            {"action": "start", "preset": "sched", "sessionKey": "ring1", "bufferKb": 16384, "maxDurationSec": 300},
            {"action": "status", "sessionKey": "ring1"},
            {"action": "snapshot", "sessionKey": "ring1", "keepRunning": False, "analyze": True},
            {"action": "stop", "sessionKey": "ring1", "analyze": False},
            {"action": "analyze", "sessionKey": "ring1"},
            {"action": "analyze", "remotePath": "/data/local/tmp/sample.perfetto-trace"},
        ]
        for args in cases:
            captured: list[str] = []
            dispatch_tool(
                "perfetto_trace",
                args,
                exec_fn=lambda cmd, timeout=30.0, **kw: (captured.append(cmd) or {"rc": 0, "json": {"ok": True}, "stdout": "{}", "stderr": ""}),
            )
            self.assertEqual(len(captured), 1)
            argv = shlex.split(captured[0])
            self.assertEqual(argv[0], "strata-debug")
            parsed = parser.parse_args(argv[1:])
            self.assertEqual(parsed.cmd, "perfetto")
            self.assertEqual(parsed.action, args["action"])
            if "sessionKey" in args:
                self.assertEqual(parsed.session, args["sessionKey"])
            if "remotePath" in args:
                self.assertEqual(parsed.path, args["remotePath"])

    def test_cli_perfetto_analyze_resolves_session_metadata(self) -> None:
        buf = io.StringIO()
        status_payload = {"ok": True, "action": "status", "active": False, "sessionKey": "ring1", "remotePath": "/data/local/tmp/r1.perfetto-trace"}
        with (
            mock.patch.object(perfetto_session, "ring_session_status", return_value=status_payload) as mock_st,
            mock.patch.object(perfetto_analyze, "analyze_trace_file", return_value={"ok": True, "packetCount": 7}) as mock_an,
            mock.patch("sys.stdout", buf),
        ):
            rc = debug_cli.main(["perfetto", "--action", "analyze", "--session", "ring1"])
        self.assertEqual(rc, 0)
        mock_st.assert_called_once_with(session_key="ring1")
        mock_an.assert_called_once_with("/data/local/tmp/r1.perfetto-trace")
        out = json.loads(buf.getvalue())
        self.assertEqual((out["ok"], out["packetCount"], out["sessionKey"]), (True, 7, "ring1"))

        buf_err = io.StringIO()
        with (
            mock.patch.object(perfetto_session, "ring_session_status", return_value={"ok": True, "active": False, "sessionKey": "missing"}),
            mock.patch("sys.stdout", buf_err),
        ):
            rc_err = debug_cli.main(["perfetto", "--action", "analyze", "--session", "missing"])
        self.assertEqual(rc_err, 1)
        err_out = json.loads(buf_err.getvalue())
        self.assertFalse(err_out["ok"])
        self.assertIn("missing", err_out["error"])


class TestTransportCleanupAndJsonExtraction(unittest.TestCase):
    def setUp(self) -> None:
        devices.reset_device_state()
        transport.reset_transport_state()

    def tearDown(self) -> None:
        devices.reset_device_state()
        transport.reset_transport_state()

    def test_extract_trailing_json_handles_ssh_banners_and_multiline_json(self) -> None:
        banner_obj = 'Welcome to Strata\n[MOTD] ready\n{"ok": true, "nested": {"x": 1}}\n'
        self.assertEqual(transport.extract_trailing_json(banner_obj), {"ok": True, "nested": {"x": 1}})

        banner_pretty = 'Warning: Permanently added host\n{\n  "ok": true,\n  "items": [1, 2]\n}\n'
        self.assertEqual(transport.extract_trailing_json(banner_pretty), {"ok": True, "items": [1, 2]})

        inline_prefix = 'banner prefix: {"ok": true, "count": 3}'
        self.assertEqual(transport.extract_trailing_json(inline_prefix), {"ok": True, "count": 3})

        self.assertEqual(transport.extract_trailing_json('[{"id": 1}]'), [{"id": 1}])
        self.assertIsNone(transport.extract_trailing_json("plain log line without json"))
        self.assertIsNone(transport.extract_trailing_json("error: stray brace }"))
        self.assertIsNone(transport.extract_trailing_json(""))

    def test_cleanup_all_transports_stops_forwards_controlmasters_and_adb(self) -> None:
        with tempfile.TemporaryDirectory() as td, mock.patch.dict(os.environ, {"XDG_RUNTIME_DIR": td}, clear=False):
            rec = devices.DeviceRecord(
                id="dev-a", adb_serial="SER_A", adb_usb_serial="SER_A", adb_state="device", ssh_user="root",
            )
            ctl = devices.device_control_path("dev-a")
            calls: list[list[str]] = []

            def _fake_run(argv, **kw):
                calls.append(list(argv))
                if argv[0] == "ssh" and "-O" not in argv:
                    with open(ctl, "w", encoding="utf-8") as f:
                        f.write("mux")
                return subprocess.CompletedProcess(argv, 0, stdout='{"ok": true}', stderr="")

            with (
                mock.patch.object(devices, "resolve_device", return_value=rec),
                mock.patch.object(devices, "_port_in_use", return_value=False),
                mock.patch("subprocess.run", side_effect=_fake_run),
            ):
                fwd = transport.open_ssh_port_forward(9222, 9222, device="dev-a")
                self.assertTrue(fwd["ok"])
                self.assertTrue(os.path.exists(ctl))
                transport.cleanup_all_transports()

            self.assertFalse(os.path.exists(ctl))
            self.assertTrue(any(c[0] == "ssh" and "-O" in c and "cancel" in c for c in calls))
            self.assertTrue(any(c[0] == "ssh" and "-O" in c and "exit" in c for c in calls))
            self.assertTrue(any(c[:4] == ["adb", "-s", "SER_A", "forward"] and "--remove" in c for c in calls))


class TestDeviceSelectApkUniquenessAndVisualFallback(unittest.TestCase):
    def test_device_select_handles_none_without_assertion_error(self) -> None:
        with mock.patch.object(devices, "select_device", return_value=None):
            res = dispatch_tool("device_select", {"device": "ghost"})
        self.assertFalse(res["ok"])
        self.assertIsNone(res["selected"])
        self.assertIn("ghost", res["error"])

    def test_apk_install_uses_unique_remote_temp_filenames(self) -> None:
        pushed_paths: list[str] = []
        with tempfile.NamedTemporaryFile(suffix=".apk") as apk:
            apk.write(b"PK\x03\x04")
            apk.flush()
            for _ in range(2):
                dispatch_tool(
                    "apk_install",
                    {"localPath": apk.name},
                    exec_fn=lambda cmd, timeout=30.0, **kw: {"rc": 0, "json": {"ok": True}, "stdout": "{}", "stderr": ""},
                    push_fn=lambda local, remote, **kw: (pushed_paths.append(remote) or {"ok": True, "rc": 0}),
                )
        self.assertEqual(len(pushed_paths), 2)
        self.assertNotEqual(pushed_paths[0], pushed_paths[1])
        for rp in pushed_paths:
            self.assertTrue(rp.startswith("/data/local/tmp/strata_apk_"))

    def test_visual_webp_fallback_to_png_aligns_remote_and_local_extensions(self) -> None:
        rgba = b"\x20\x40\x60\xff" * (8 * 8)
        real_import = __import__

        def _no_pil(name, *a, **kw):
            if name == "PIL" or name.startswith("PIL."):
                raise ImportError("blocked")
            return real_import(name, *a, **kw)

        with tempfile.TemporaryDirectory() as td:
            req_path = os.path.join(td, "shot.webp")
            with (
                mock.patch("builtins.__import__", side_effect=_no_pil),
                mock.patch.object(visual_check.shutil, "which", return_value=None),
            ):
                saved = visual_check.save_encoded_image(rgba, 8, 8, req_path, fmt="webp")
            self.assertEqual(saved["format"], "png")
            self.assertTrue(saved["path"].endswith(".png"))
            self.assertTrue(os.path.isfile(saved["path"]))
            self.assertFalse(os.path.exists(req_path))
            with open(saved["path"], "rb") as f:
                self.assertTrue(f.read(8).startswith(b"\x89PNG\r\n\x1a\n"))

        pulls: list[tuple[str, str]] = []
        rm_cmds: list[str] = []

        def _fake_exec(cmd: str, timeout: float = 30.0, **kw):
            if cmd.startswith("rm -f "):
                rm_cmds.append(cmd)
                return {"rc": 0, "json": None, "stdout": "", "stderr": ""}
            return {
                "rc": 0,
                "json": {"ok": True, "saved": {"path": "/tmp/strata-mcp-cap-1.png", "format": "png"}},
                "stdout": "{}",
                "stderr": "",
            }

        res = dispatch_tool(
            "visual_check",
            {"format": "webp", "pullToLocal": True, "localOut": "/tmp/out-cap.webp"},
            exec_fn=_fake_exec,
            pull_fn=lambda r, l, **kw: (pulls.append((r, l)) or {"ok": True, "rc": 0}),
        )
        self.assertTrue(res["pulled"])
        self.assertEqual(pulls, [("/tmp/strata-mcp-cap-1.png", "/tmp/out-cap.png")])
        self.assertEqual(res["localPath"], "/tmp/out-cap.png")
        self.assertEqual(rm_cmds, ["rm -f /tmp/strata-mcp-cap-1.png"])


if __name__ == "__main__":
    unittest.main()
