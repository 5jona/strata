"""Unit tests for multi-device discovery, strict correlation, auto-fallback, self-heal, and new MCP tools."""

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
SCRIPTS_DIR = os.path.join(REPO_ROOT, "scripts")
if SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, SCRIPTS_DIR)

from strata_mcp import devices, transport


def _load_mcp_module():
    path = os.path.join(SCRIPTS_DIR, "strata-debug-mcp.py")
    spec = importlib.util.spec_from_file_location("strata_debug_mcp_md", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


SAMPLE_ADB = """List of devices attached
DEMO_USB_A             device usb:1-2 product:prod_a model:DemoPhoneA device:dev_a transport_id:1
198.51.100.20:5555     device product:prod_b model:DemoTabletB device:dev_b transport_id:2
DEMO_OFFLINE           offline transport_id:3
"""

SAMPLE_SSH_CONFIG = """
Host *
    ServerAliveInterval 60

Host demo-a-wifi
    HostName 198.51.100.10
    Port 2222
    User root

Host strata-b
    HostName 198.51.100.20
    Port 2222
    User root

Host unrelated-server
    HostName 203.0.113.5
    Port 22
    User admin
"""


class TestMultiDeviceDiscoveryAndSelection(unittest.TestCase):
    def setUp(self) -> None:
        devices.reset_device_state()

    def tearDown(self) -> None:
        devices.reset_device_state()

    def test_adb_and_ssh_config_parsing(self) -> None:
        adb_entries = devices.parse_adb_devices_output(SAMPLE_ADB)
        self.assertEqual(len(adb_entries), 3)
        self.assertEqual((adb_entries[0].serial, adb_entries[0].transport, adb_entries[0].model), ("DEMO_USB_A", "usb", "DemoPhoneA"))
        self.assertEqual((adb_entries[1].serial, adb_entries[1].transport, adb_entries[1].model), ("198.51.100.20:5555", "tcpip", "DemoTabletB"))
        self.assertEqual(adb_entries[2].state, "offline")

        ssh_entries = devices.parse_ssh_config_text(SAMPLE_SSH_CONFIG)
        hosts = [e.host for e in ssh_entries]
        self.assertEqual(hosts, ["demo-a-wifi", "strata-b"])
        self.assertNotIn("unrelated-server", hosts)
        self.assertEqual((ssh_entries[0].hostname, ssh_entries[0].port, ssh_entries[0].user), ("198.51.100.10", 2222, "root"))

    def test_strict_identity_correlation_prevents_cross_device_merge(self) -> None:
        adb_entries = [devices.AdbDeviceEntry(serial="DEMO_USB_A", state="device", transport="usb", model="DemoPhoneA")]
        ssh_entries = [devices.SshHostEntry(host="demo-b-wifi", hostname="198.51.100.20", port=2222, user="root")]

        # Without explicit mapping in devices.json, USB A and Wi-Fi B MUST remain separate records
        unmapped = devices.discover_devices(adb_entries=adb_entries, ssh_entries=ssh_entries, config_map={})
        self.assertEqual(len(unmapped), 2)
        self.assertEqual({r.id for r in unmapped}, {"DEMO_USB_A", "demo-b-wifi"})

        # With explicit mapping in devices.json, only the matching serial + sshHost merge
        cfg = {"phone-a": {"adbSerial": "DEMO_USB_A", "sshHost": "demo-a-wifi", "sshPort": 2222, "model": "DemoPhoneA"}}
        ssh_both = [*ssh_entries, devices.SshHostEntry(host="demo-a-wifi", hostname="198.51.100.10", port=2222, user="root")]
        mapped = devices.discover_devices(adb_entries=adb_entries, ssh_entries=ssh_both, config_map=cfg)
        self.assertEqual(len(mapped), 2)
        by_id = {r.id: r for r in mapped}
        self.assertEqual(by_id["phone-a"].adb_usb_serial, "DEMO_USB_A")
        self.assertEqual(by_id["phone-a"].ssh_host, "demo-a-wifi")
        self.assertIsNone(by_id["demo-b-wifi"].adb_serial)

    def test_fail_fast_multi_device_and_single_device_auto(self) -> None:
        dev_a = devices.DeviceRecord(id="phone-a", model="DemoPhoneA", adb_serial="SER_A", adb_usb_serial="SER_A", adb_state="device")
        dev_b = devices.DeviceRecord(id="tablet-b", model="DemoTabletB", ssh_host="strata-b", ssh_port=2222)

        # Single device: auto-selected without friction
        self.assertEqual(devices.resolve_device(records=[dev_a]).id, "phone-a")  # type: ignore[union-attr]

        # Multiple devices with no selection: fail-fast listing available devices
        with self.assertRaises(RuntimeError) as ctx:
            devices.resolve_device(records=[dev_a, dev_b])
        msg = str(ctx.exception)
        self.assertIn("phone-a", msg)
        self.assertIn("tablet-b", msg)
        self.assertIn("device_select", msg)

        # Session selection persists and resolves cleanly
        chosen = devices.select_device("DemoTabletB", records=[dev_a, dev_b])
        self.assertIsNotNone(chosen)
        self.assertEqual(devices.get_selected_device(), "tablet-b")
        self.assertEqual(devices.resolve_device(records=[dev_a, dev_b]).id, "tablet-b")  # type: ignore[union-attr]

        # Clear selection returns to fail-fast
        devices.select_device("")
        self.assertIsNone(devices.get_selected_device())

    def test_per_device_isolation_ports_control_and_known_hosts(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            with mock.patch.dict(os.environ, {"XDG_RUNTIME_DIR": td}, clear=False):
                ctl_a = devices.device_control_path("phone-a")
                ctl_b = devices.device_control_path("tablet-b")
                kh_a = devices.device_known_hosts_path("phone-a")
                kh_b = devices.device_known_hosts_path("tablet-b")
                self.assertNotEqual(ctl_a, ctl_b)
                self.assertNotEqual(kh_a, kh_b)
                self.assertEqual(stat.S_IMODE(os.stat(os.path.dirname(ctl_a)).st_mode), 0o700)

                p_a = devices.allocate_forward_port("phone-a", probe_bind=False)
                p_b = devices.allocate_forward_port("tablet-b", probe_bind=False)
                self.assertTrue(12222 <= p_a <= 12299)
                self.assertTrue(12222 <= p_b <= 12299)
                self.assertNotEqual(p_a, p_b)
                # Collision avoidance when port is occupied
                p_c = devices.allocate_forward_port("phone-c", used_ports={p_a, p_b}, probe_bind=False)
                self.assertNotIn(p_c, {p_a, p_b})


class TestTransportFallbackAndSelfHeal(unittest.TestCase):
    def setUp(self) -> None:
        devices.reset_device_state()
        transport.reset_transport_state()

    def tearDown(self) -> None:
        devices.reset_device_state()
        transport.reset_transport_state()

    def test_wifi_failure_falls_back_to_adb_ssh(self) -> None:
        rec = devices.DeviceRecord(
            id="phone-a", alias="phone-a", adb_serial="SER_A", adb_usb_serial="SER_A",
            adb_state="device", ssh_host="demo-a-wifi", ssh_port=2222, ssh_user="root",
        )
        calls: list[list[str]] = []

        def _fake_run(argv, **kw):
            calls.append(list(argv))
            if argv[0] == "adb":
                return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")
            if "demo-a-wifi" in argv[-2]:
                return subprocess.CompletedProcess(argv, 255, stdout="", stderr="Connection timed out")
            return subprocess.CompletedProcess(argv, 0, stdout='{"ok": true}', stderr="")

        with tempfile.TemporaryDirectory() as td, \
             mock.patch.dict(os.environ, {"XDG_RUNTIME_DIR": td}, clear=False), \
             mock.patch.object(devices, "resolve_device", return_value=rec), \
             mock.patch.object(devices, "_port_in_use", return_value=False), \
             mock.patch("subprocess.run", side_effect=_fake_run):
            res = transport.ssh_exec("strata-debug sys-probe", device="phone-a")
        self.assertEqual(res["rc"], 0)
        self.assertEqual(res["json"], {"ok": True})
        self.assertTrue(any(c[:4] == ["adb", "-s", "SER_A", "forward"] for c in calls))
        self.assertTrue(any("127.0.0.1" in c[-2] for c in calls if c[0] == "ssh"))

    def test_usb_reenumeration_rc255_self_heals_only_target_device(self) -> None:
        rec = devices.DeviceRecord(
            id="phone-a", adb_serial="SER_A", adb_usb_serial="SER_A", adb_state="device", ssh_user="root",
        )
        with tempfile.TemporaryDirectory() as td, mock.patch.dict(os.environ, {"XDG_RUNTIME_DIR": td}, clear=False):
            ctl_a, ctl_b = devices.device_control_path("phone-a"), devices.device_control_path("tablet-b")
            for p in (ctl_a, ctl_b):
                with open(p, "w", encoding="utf-8") as f:
                    f.write("sock")
            adb_fwd_count, ssh_count, exit_calls = 0, 0, []

            def _fake_run(argv, **kw):
                nonlocal adb_fwd_count, ssh_count
                if argv[0] == "adb":
                    adb_fwd_count += 1
                    return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")
                if "-O" in argv and "exit" in argv:
                    exit_calls.append(list(argv))
                    return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")
                ssh_count += 1
                if ssh_count == 1:
                    return subprocess.CompletedProcess(argv, 255, stdout="", stderr="Connection reset by peer")
                return subprocess.CompletedProcess(argv, 0, stdout='{"healed": true}', stderr="")

            with mock.patch.object(devices, "resolve_device", return_value=rec), \
                 mock.patch.object(devices, "_port_in_use", return_value=False), \
                 mock.patch("subprocess.run", side_effect=_fake_run):
                res = transport.ssh_exec("echo ok", device="phone-a")
            self.assertEqual((res["rc"], res["json"]), (0, {"healed": True}))
            self.assertEqual((adb_fwd_count, ssh_count, len(exit_calls)), (2, 2, 1))
            self.assertFalse(os.path.exists(ctl_a))
            self.assertTrue(os.path.exists(ctl_b))


class TestNewMcpToolsDispatch(unittest.TestCase):
    def setUp(self) -> None:
        devices.reset_device_state()

    def tearDown(self) -> None:
        devices.reset_device_state()

    def test_devices_list_select_and_perfetto_trace_pull(self) -> None:
        mcp = _load_mcp_module()
        recs = [
            devices.DeviceRecord(id="phone-a", alias="phone-a", model="DemoPhoneA", adb_serial="SER_A", adb_usb_serial="SER_A", adb_state="device"),
            devices.DeviceRecord(id="tablet-b", alias="tablet-b", model="DemoTabletB", ssh_host="strata-b", ssh_port=2222),
        ]

        def _call(name: str, arguments: dict) -> dict:
            resp = mcp.handle_rpc_request({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
            assert resp is not None and not resp["result"].get("isError"), resp
            return json.loads(resp["result"]["content"][0]["text"])

        with mock.patch.object(devices, "discover_devices", return_value=recs):
            listed = _call("devices_list", {})
            self.assertEqual(listed["count"], 2)
            self.assertIsNone(listed["selected"])

            sel = _call("device_select", {"device": "tablet-b"})
            self.assertEqual(sel["selected"], "tablet-b")
            self.assertTrue(_call("devices_list", {})["devices"][1]["selected"])

            cleared = _call("device_select", {"clear": True})
            self.assertTrue(cleared["cleared"])
            self.assertIsNone(devices.get_selected_device())

        cmds: list[tuple[str, dict]] = []
        pulls: list[tuple[str, str, dict]] = []

        def _fake_exec(cmd: str, timeout: float = 30.0, **kw):
            cmds.append((cmd, kw))
            return {"rc": 0, "json": {"ok": True, "remotePath": "/data/local/tmp/strata-perfetto-sched-1.perfetto-trace"}, "stdout": "{}", "stderr": ""}

        def _fake_pull(remote: str, local: str, **kw):
            pulls.append((remote, local, kw))
            return {"ok": True, "rc": 0, "bytes": 4096}

        with mock.patch.object(mcp, "ssh_exec", side_effect=_fake_exec), \
             mock.patch.object(mcp, "scp_pull", side_effect=_fake_pull):
            res = _call("perfetto_trace", {
                "preset": "sched", "durationSec": 5, "bufferKb": 8192,
                "pullToLocal": True, "device": "phone-a",
            })
        self.assertEqual(cmds[-1][0], "strata-debug perfetto --preset sched --duration 5 --buffer-kb 8192")
        self.assertEqual(cmds[-1][1], {"device": "phone-a"})
        self.assertTrue(res["pulled"])
        self.assertTrue(res["localPath"].endswith(".perfetto-trace"))
        self.assertEqual(pulls[-1][0], "/data/local/tmp/strata-perfetto-sched-1.perfetto-trace")
        self.assertEqual(pulls[-1][2], {"device": "phone-a", "host_scope": True})

    def test_perfetto_ring_buffer_and_analyze_dispatch(self) -> None:
        mcp = _load_mcp_module()
        cmds: list[str] = []
        pulls: list[tuple[str, str]] = []

        def _fake_exec(cmd: str, timeout: float = 30.0, **kw):
            cmds.append(cmd)
            return {"rc": 0, "json": {"ok": True, "remotePath": "/data/local/tmp/ring-snap.perfetto-trace"}, "stdout": "{}", "stderr": ""}

        def _call(arguments: dict) -> dict:
            resp = mcp.handle_rpc_request({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "perfetto_trace", "arguments": arguments}})
            assert resp is not None and not resp["result"].get("isError"), resp
            return json.loads(resp["result"]["content"][0]["text"])

        with mock.patch.object(mcp, "ssh_exec", side_effect=_fake_exec), \
             mock.patch.object(mcp, "scp_pull", side_effect=lambda r, l, **kw: (pulls.append((r, l)) or {"ok": True, "rc": 0})):
            _call({"action": "start", "preset": "ui-jank", "sessionKey": "ring1", "bufferKb": 16384, "maxDurationSec": 900})
            self.assertEqual(cmds[-1], "strata-debug perfetto --action start --preset ui-jank --session ring1 --buffer-kb 16384 --max-duration 900")
            snap = _call({"action": "snapshot", "sessionKey": "ring1", "keepRunning": False, "analyze": True, "pullToLocal": True})
            self.assertEqual(cmds[-1], "strata-debug perfetto --action snapshot --session ring1 --no-keep-running --analyze")
            self.assertTrue(snap["pulled"] and snap["localPath"].endswith(".perfetto-trace") and len(pulls) == 1)
            stop = _call({"action": "stop", "sessionKey": "ring1", "analyze": False, "pullToLocal": True})
            self.assertEqual(cmds[-1], "strata-debug perfetto --action stop --session ring1 --no-analyze")
            self.assertTrue(stop["pulled"] and len(pulls) == 2)
            _call({"action": "analyze", "remotePath": "/data/local/tmp/ring-snap.perfetto-trace"})
            self.assertEqual(cmds[-1], "strata-debug perfetto --action analyze --path /data/local/tmp/ring-snap.perfetto-trace")


if __name__ == "__main__":
    unittest.main()
