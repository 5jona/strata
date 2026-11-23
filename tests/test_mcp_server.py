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

    def test_all_27_tools_and_schemas(self) -> None:
        mcp = _load_mcp_module()
        listed = mcp.handle_rpc_request({"jsonrpc": "2.0", "id": 10, "method": "tools/list", "params": {}})
        assert listed is not None
        tools = listed["result"]["tools"]
        self.assertEqual(len(tools), 27)
        expected_new = {
            "ui_dump", "ui_tap", "app_inspect", "app_control", "apk_install",
            "lsposed_inspect", "sqlite_query", "ksu_inspect", "device_exec", "file_transfer",
            "devices_list", "device_select", "perfetto_trace",
        }
        self.assertTrue(expected_new.issubset({t["name"] for t in tools}))
        local_only = {"devices_list", "device_select"}
        for tool in tools:
            s = tool["inputSchema"]
            self.assertEqual((s["type"], s["additionalProperties"]), ("object", False))
            if tool["name"] not in local_only:
                self.assertTrue({"device", "host", "port"}.issubset(s["properties"]))

    def test_ssh_alias_and_ip_defaults(self) -> None:
        mcp = _load_mcp_module()
        with mock.patch.dict(os.environ, {"STRATA_SSH_HOST": "demo-device"}, clear=True):
            host, port, user = mcp._ssh_config()
            self.assertEqual((host, port, user), ("demo-device", "", ""))
            args = mcp.build_ssh_base_args(host, port, user)
            self.assertNotIn("-p", args)
            self.assertEqual(args[-1], "demo-device")
        with mock.patch.dict(os.environ, {"STRATA_SSH_HOST": "192.0.2.10"}, clear=True):
            host, port, user = mcp._ssh_config()
            self.assertEqual((host, port, user), ("192.0.2.10", "2222", "root"))
            args = mcp.build_ssh_base_args(host, port, user)
            self.assertIn("-p", args)
            self.assertEqual(args[-1], "root@192.0.2.10")

    def test_android_tools_dispatch_and_quoting(self) -> None:
        mcp = _load_mcp_module()
        cmds: list[tuple[str, dict]] = []
        xfers: list[tuple[str, tuple, dict]] = []

        def _exec(cmd: str, timeout: float = 30.0, **kw):
            cmds.append((cmd, kw))
            return {"rc": 0, "json": {"ok": True}, "stdout": '{"ok": true}', "stderr": ""}

        def _call(name: str, arguments: dict) -> dict:
            r = mcp.handle_rpc_request({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
            assert r is not None and not r["result"].get("isError"), r
            return json.loads(r["result"]["content"][0]["text"])

        with mock.patch.object(mcp, "ssh_exec", side_effect=_exec), \
             mock.patch.object(mcp, "scp_push", side_effect=lambda *a, **kw: (xfers.append(("push", a, kw)) or {"ok": True, "rc": 0})), \
             mock.patch.object(mcp, "scp_pull", side_effect=lambda *a, **kw: (xfers.append(("pull", a, kw)) or {"ok": True, "rc": 0})):
            _call("ui_dump", {"compressed": True, "query": "Allow; $(id)", "clickableOnly": True, "limit": 5})
            self.assertIn("strata-debug ui dump --compressed --query 'Allow; $(id)' --clickable-only --limit 5", cmds[-1][0])
            _call("ui_tap", {"text": "OK", "method": "uinput", "host": "demo-device", "port": 2200})
            self.assertEqual((cmds[-1][0], cmds[-1][1]), ("strata-debug ui tap --text OK --method uinput", {"host": "demo-device", "port": 2200}))
            _call("app_inspect", {"topOnly": True})
            self.assertEqual(cmds[-1][0], "strata-debug app top")
            _call("app_control", {"action": "start", "package": "com.example.app", "activity": ".Main", "intentAction": "android.intent.action.VIEW", "dataUri": "https://example.com"})
            self.assertEqual(cmds[-1][0], "strata-debug app start com.example.app --activity .Main --action android.intent.action.VIEW --data https://example.com")
            _call("lsposed_inspect", {"mode": "logs", "module": "com.example.mod", "limit": 20})
            self.assertEqual(cmds[-1][0], "strata-debug lsposed logs --module com.example.mod --limit 20")
            _call("sqlite_query", {"dbPath": "/data/tmp/a.db", "sql": "SELECT 'x;rm'"})
            self.assertIn("strata-debug db query /data/tmp/a.db --sql 'SELECT '\"'\"'x;rm'\"'\"''", cmds[-1][0])
            _call("ksu_inspect", {"mode": "sysfs", "path": "/proc/version"})
            self.assertEqual(cmds[-1][0], "strata-debug ksu sysfs /proc/version")
            _call("device_exec", {"command": "echo 'hi'", "scope": "host", "timeoutSec": 99})
            self.assertIn("host-exec /system/bin/sh -c", cmds[-1][0])
            _call("file_transfer", {"direction": "pull", "localPath": "/tmp/a", "remotePath": "/data/b", "hostScope": True})
            self.assertEqual(xfers[-1], ("pull", ("/data/b", "/tmp/a"), {"host_scope": True}))
            with tempfile.NamedTemporaryFile(suffix=".apk") as apk:
                apk.write(b"PK\x03\x04")
                apk.flush()
                _call("apk_install", {"localPath": apk.name, "allowDowngrade": True, "grantPermissions": False})
                self.assertEqual(xfers[-1][0], "push")
                self.assertTrue(xfers[-1][2].get("host_scope"))
                self.assertIn("strata-debug app install /data/local/tmp/strata_apk_", cmds[-2][0])
                self.assertIn("--no-grant --downgrade", cmds[-2][0])
                self.assertTrue(cmds[-1][0].startswith("host-exec /system/bin/rm -f /data/local/tmp/strata_apk_"))

    def test_visual_check_pull_to_local(self) -> None:
        mcp = _load_mcp_module()
        cmds: list[str] = []
        pulls: list[tuple[str, str]] = []
        with mock.patch.object(mcp, "ssh_exec", side_effect=lambda c, timeout=30.0: (cmds.append(c) or {"rc": 0, "json": {"ok": True}, "stdout": '{"ok": true}', "stderr": ""})), \
             mock.patch.object(mcp, "scp_pull", side_effect=lambda r, l, **kw: (pulls.append((r, l)) or {"ok": True, "rc": 0, "bytes": 100})):
            resp = mcp.handle_rpc_request({"jsonrpc": "2.0", "id": 11, "method": "tools/call", "params": {"name": "visual_check", "arguments": {"format": "png", "maxWidth": 720, "pullToLocal": True}}})
            assert resp is not None and not resp["result"].get("isError")
            payload = json.loads(resp["result"]["content"][0]["text"])
            self.assertTrue(payload["pulled"] and payload["localPath"].endswith(".png"))
            self.assertIn("--max-width 720", cmds[0])
            self.assertEqual(len(pulls), 1)
            self.assertTrue(pulls[0][0].startswith("/tmp/strata-mcp-cap-"))
            self.assertTrue(cmds[-1].startswith("rm -f /tmp/strata-mcp-cap-"))
            parsed = debug_cli.build_parser().parse_args(["visual", "--max-width", "720", "--format", "png"])
            self.assertEqual(parsed.max_width, 720)


if __name__ == "__main__":
    unittest.main()
