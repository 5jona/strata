"""Unit tests for logcat cursor recovery, kernel diag degradation, observe CLI, and MCP server."""

from __future__ import annotations

import importlib.util
import json
import os
import stat
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
import kernel_probe
import logcat_cursor
import visual_check


def _load_mcp_module():
    path = os.path.join(SCRIPTS_DIR, "strata-debug-mcp.py")
    spec = importlib.util.spec_from_file_location("strata_debug_mcp", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class TestLogcatCursorHardening(unittest.TestCase):
    def test_mark_incremental_advance_and_corrupt_cursor_recovery(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            cpath = os.path.join(td, "logcat.cursor")
            line1 = "11-23 05:00:00.100  1000  1001 I TagA: boot"
            line2 = "11-23 05:00:01.200  1000  1001 E AndroidRuntime: FATAL EXCEPTION: main"

            # 1. --mark records exact watermark without dumping historical lines
            mark_proc = subprocess.CompletedProcess([], 0, stdout=line1 + "\n", stderr="")
            with mock.patch.object(logcat_cursor.subprocess, "run", return_value=mark_proc):
                marked = logcat_cursor.mark_cursor(cpath)
            self.assertEqual(marked["ts"], "11-23 05:00:00.100")
            self.assertTrue(marked["lastDigest"])

            # 2. Incremental read returns only lines after mark and advances cursor atomically
            read_proc = subprocess.CompletedProcess([], 0, stdout=f"{line1}\n{line2}\n", stderr="")
            with mock.patch.object(logcat_cursor.subprocess, "run", return_value=read_proc):
                res = logcat_cursor.read_incremental(cpath, priority="W")
            self.assertTrue(res["ok"])
            self.assertEqual(res["count"], 1)
            self.assertEqual(res["cursor"], "11-23 05:00:01.200")
            self.assertEqual(len(res.get("crashes", [])), 1)
            self.assertNotIn("warning", res)

            # 3. Corrupt cursor file falls back gracefully with warning instead of crashing
            with open(cpath, "w", encoding="utf-8") as f:
                f.write("{corrupt-json")
            with mock.patch.object(logcat_cursor.subprocess, "run", return_value=read_proc):
                res_bad = logcat_cursor.read_incremental(cpath, priority="W", advance=False)
            self.assertTrue(res_bad["ok"])
            self.assertIn("warning", res_bad)
            self.assertIn("corrupt", res_bad["warning"])


class TestKernelVisualAndObserveCli(unittest.TestCase):
    def test_kernel_graceful_degradation_when_mounts_missing(self) -> None:
        missing = "/nonexistent-strata-mount-xyz"
        with mock.patch.object(kernel_probe.os.path, "exists", return_value=False):
            pstore = kernel_probe.collect_pstore(missing)
            self.assertTrue(pstore["ok"])
            self.assertFalse(pstore["available"])
            self.assertIn("reason", pstore)

            tracing = kernel_probe.inspect_tracing(missing)
            self.assertTrue(tracing["ok"])
            self.assertFalse(tracing["available"])
            self.assertIn("reason", tracing)

            with mock.patch.object(kernel_probe.shutil, "which", return_value=None):
                dmesg = kernel_probe.collect_dmesg()
                self.assertTrue(dmesg["ok"])
                self.assertFalse(dmesg["available"])
                self.assertIn("reason", dmesg)

    def test_visual_solid_color_and_optional_out(self) -> None:
        solid_blue = b"\x00\x00\xff\xff" * (16 * 16)
        health = visual_check.analyze_rgba_health(solid_blue, 16, 16)
        self.assertTrue(health["isSolidColor"])
        self.assertTrue(health["isBlank"])
        self.assertFalse(health["isBlack"])

        with mock.patch.object(visual_check, "capture_raw_frame", return_value=(solid_blue, 16, 16)):
            res = visual_check.run_visual_check(out_path=None)
            self.assertTrue(res["ok"])
            self.assertNotIn("saved", res)
            self.assertTrue(res["health"]["isSolidColor"])

    def test_observe_and_new_cli_parsing(self) -> None:
        parser = debug_cli.build_parser()
        o_args = parser.parse_args([
            "observe", "--cursor", "/tmp/x.cur", "--settle-ms", "150", "--no-cdp", "--no-visual"
        ])
        self.assertEqual(o_args.cursor, "/tmp/x.cur")
        self.assertEqual(o_args.settle_ms, 150)
        self.assertTrue(o_args.no_cdp)
        self.assertTrue(o_args.no_visual)

        ce = parser.parse_args(["cdp", "eval", "document.title", "--pid", "42", "--url-match", "sideveil"])
        self.assertEqual((ce.expression, ce.pid, ce.url_match), ("document.title", 42, "sideveil"))

        kd = parser.parse_args(["kernel", "dmesg", "--limit", "25", "--filter", "panic"])
        self.assertEqual((kd.kernel_cmd, kd.limit, kd.filter), ("dmesg", 25, "panic"))

        kt = parser.parse_args(["kernel", "tracing", "--dir", "/tmp/tr", "--limit", "10"])
        self.assertEqual((kt.kernel_cmd, kt.dir, kt.limit), ("tracing", "/tmp/tr", 10))


class TestMcpServerDispatch(unittest.TestCase):
    def test_control_path_is_0700_per_user(self) -> None:
        mcp = _load_mcp_module()
        with tempfile.TemporaryDirectory() as td:
            with mock.patch.dict(os.environ, {"XDG_RUNTIME_DIR": td}, clear=False):
                ctl = mcp._control_path()
                ctl_dir = os.path.dirname(ctl)
                self.assertEqual(ctl_dir, os.path.join(td, "strata-mcp"))
                mode = stat.S_IMODE(os.stat(ctl_dir).st_mode)
                self.assertEqual(mode, 0o700)

    def test_initialize_tools_list_and_mocked_tools_call(self) -> None:
        mcp = _load_mcp_module()
        init = mcp.handle_rpc_request({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        assert init is not None
        self.assertEqual(init["result"]["serverInfo"]["name"], "strata-debug-mcp")

        listed = mcp.handle_rpc_request({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        assert listed is not None
        names = {t["name"] for t in listed["result"]["tools"]}
        expected = {
            "observe_step", "cdp_list_targets", "cdp_eval", "cdp_console", "cdp_audit",
            "logcat_cursor", "visual_check", "gesture_replay", "kernel_diag", "kernel_probe",
            "pstore_dump", "device_health_snapshot", "reload_and_verify", "forward_cdp",
        }
        self.assertTrue(expected.issubset(names))

        captured_cmds: list[str] = []

        def _fake_ssh(remote_cmd: str, timeout: float = 30.0):
            captured_cmds.append(remote_cmd)
            return {"rc": 0, "json": {"ok": True}, "stdout": '{"ok": true}', "stderr": ""}

        with mock.patch.object(mcp, "ssh_exec", side_effect=_fake_ssh):
            # Test safe shlex quoting against shell metacharacters
            r1 = mcp.handle_rpc_request({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "cdp_eval", "arguments": {"expression": "1; $(rm -rf /)"}},
            })
            assert r1 is not None and not r1["result"].get("isError")
            self.assertIn("'1; $(rm -rf /)'", captured_cmds[-1])

            r2 = mcp.handle_rpc_request({
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {"name": "logcat_cursor", "arguments": {"mark": True, "cursor": "/tmp/a.cur"}},
            })
            assert r2 is not None and not r2["result"].get("isError")
            self.assertIn("--mark", captured_cmds[-1])
            self.assertIn("/tmp/a.cur", captured_cmds[-1])

            r3 = mcp.handle_rpc_request({
                "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": {"name": "kernel_diag", "arguments": {"mode": "tracing", "limit": 20}},
            })
            assert r3 is not None and not r3["result"].get("isError")
            self.assertIn("strata-debug kernel tracing --limit 20", captured_cmds[-1])

            r4 = mcp.handle_rpc_request({
                "jsonrpc": "2.0", "id": 6, "method": "tools/call",
                "params": {"name": "observe_step", "arguments": {"cursor": "/tmp/o.cur", "noVisual": True}},
            })
            assert r4 is not None and not r4["result"].get("isError")
            self.assertIn("--cursor /tmp/o.cur", captured_cmds[-1])
            self.assertIn("--no-visual", captured_cmds[-1])


if __name__ == "__main__":
    unittest.main()
