"""Cross-stack CI drift gates for MCP CLI flags, host-exec env parity, and config.json defaults."""

from __future__ import annotations

import json
import os
import re
import shlex
import sys
import unittest
from typing import Any

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEBUG_DIR = os.path.join(REPO_ROOT, "container", "strata-debug")
SCRIPTS_DIR = os.path.join(REPO_ROOT, "scripts")
for _p in (DEBUG_DIR, SCRIPTS_DIR):
    if _p not in sys.path:
        sys.path.insert(0, _p)

import cli as strata_debug_cli  # noqa: E402
from strata_mcp import TOOLS, dispatch_tool  # noqa: E402
from strata_mcp.tools_core import CORE_TOOL_NAMES  # noqa: E402
from strata_mcp.tools_android import ANDROID_TOOL_NAMES  # noqa: E402
from strata_mcp.tools_devices import DEVICE_TOOL_NAMES  # noqa: E402

# Tools that intentionally do not invoke `strata-debug` on the device.
NON_STRATA_DEBUG_TOOLS = {
    "devices_list",
    "device_select",
    "device_exec",
    "file_transfer",
    "reload_and_verify",
}


def _mcp_tool_invocations() -> list[tuple[str, dict[str, Any]]]:
    """Representative argument matrix covering every MCP tool that emits strata-debug CLI flags."""
    return [
        (
            "observe_step",
            {
                "actionRpc": ["daemon.ping", "{}"],
                "actionShell": "true",
                "actionCdpEval": "1+1",
                "actionGesture": {"kind": "tap", "x": 100, "y": 200},
                "cursor": "/tmp/strata-test.cursor",
                "settleMs": 200,
                "package": "com.example.app",
                "priority": "W",
                "tag": "Strata",
                "includeCdp": False,
                "includeVisual": True,
                "roi": "0,0,100,100",
            },
        ),
        ("observe_step", {"noVisual": True}),
        ("cdp_list_targets", {"target": "com.example.app", "pid": 1234}),
        (
            "cdp_eval",
            {"expression": "document.title", "target": "com.example.app", "pid": 1234, "urlMatch": "index", "timeoutSec": 4.0},
        ),
        (
            "cdp_console",
            {"target": "com.example.app", "pid": 1234, "urlMatch": "index", "durationSec": 1.5, "timeoutSec": 4.0},
        ),
        (
            "cdp_audit",
            {"target": "com.example.app", "pid": 1234, "urlContains": "index", "evalJs": "1+1"},
        ),
        ("logcat_cursor", {"mark": True, "cursor": "/tmp/lc.cursor"}),
        (
            "logcat_cursor",
            {"mark": False, "cursor": "/tmp/lc.cursor", "priority": "E", "tag": "Test", "pid": 42, "limit": 50, "noAdvance": True},
        ),
        (
            "visual_check",
            {"roi": "0,0,200,200", "out": "/tmp/cap.webp", "format": "webp", "quality": 80, "maxWidth": 480, "golden": "/tmp/g.png", "burstSheet": True},
        ),
        (
            "gesture_replay",
            {"kind": "edge-swipe", "side": "left", "yPercent": 45.0, "distanceDp": 80.0, "durationMs": 220, "cancelBack": True},
        ),
        ("gesture_replay", {"kind": "tap", "x": 100, "y": 200}),
        ("gesture_replay", {"kind": "long-press", "x": 100, "y": 200, "durationMs": 700}),
        ("gesture_replay", {"kind": "swipe", "x0": 10, "y0": 20, "x1": 300, "y1": 400, "durationMs": 260}),
        ("kernel_diag", {"mode": "pstore", "dir": "/sys/fs/pstore"}),
        ("kernel_diag", {"mode": "dmesg", "limit": 50, "filter": "strata"}),
        ("kernel_diag", {"mode": "tracing", "dir": "/sys/kernel/tracing", "limit": 30}),
        ("kernel_probe", {"preset": "guard-audit", "durationSec": 3}),
        ("pstore_dump", {}),
        ("device_health_snapshot", {"package": "com.example.app"}),
        ("forward_cdp", {"localPort": 9222, "pid": 1234}),
        (
            "ui_dump",
            {"compressed": True, "query": "OK", "resourceId": "btn_ok", "desc": "Confirm", "clickableOnly": True, "limit": 40},
        ),
        ("ui_tap", {"text": "OK", "resourceId": "btn_ok", "desc": "Confirm", "index": 1, "method": "uinput"}),
        ("app_inspect", {"topOnly": True}),
        ("app_inspect", {"package": "com.example.app"}),
        (
            "app_control",
            {"action": "start", "package": "com.example.app", "activity": ".MainActivity", "intentAction": "android.intent.action.VIEW", "dataUri": "https://example.invalid"},
        ),
        ("app_control", {"action": "stop", "package": "com.example.app"}),
        ("app_control", {"action": "clear", "package": "com.example.app"}),
        (
            "apk_install",
            {"remotePath": "/data/local/tmp/sample.apk", "grantPermissions": False, "allowDowngrade": True, "replace": False},
        ),
        ("lsposed_inspect", {"mode": "modules", "module": "com.example.mod"}),
        ("lsposed_inspect", {"mode": "logs", "module": "com.example.mod", "grep": "ERR", "limit": 50}),
        ("sqlite_query", {"dbPath": "/data/local/tmp/sample.db", "mode": "tables"}),
        ("sqlite_query", {"dbPath": "/data/local/tmp/sample.db", "mode": "query", "sql": "SELECT 1", "limit": 20}),
        ("ksu_inspect", {"mode": "modules"}),
        ("ksu_inspect", {"mode": "inputs"}),
        ("ksu_inspect", {"mode": "sysfs", "path": "/proc/version"}),
        ("perfetto_trace", {"action": "capture", "preset": "ui-jank", "durationSec": 3, "bufferKb": 8192, "analyze": True}),
        ("perfetto_trace", {"action": "start", "preset": "sched", "sessionKey": "ci", "bufferKb": 16384, "maxDurationSec": 120}),
        ("perfetto_trace", {"action": "status", "sessionKey": "ci"}),
        ("perfetto_trace", {"action": "snapshot", "sessionKey": "ci", "keepRunning": False, "analyze": False}),
        ("perfetto_trace", {"action": "stop", "sessionKey": "ci", "analyze": True}),
        ("perfetto_trace", {"action": "analyze", "remotePath": "/data/local/tmp/t.perfetto-trace", "sessionKey": "ci"}),
    ]


class CrossStackContractTests(unittest.TestCase):
    def test_every_mcp_tool_emitted_strata_debug_argv_parses_cleanly(self) -> None:
        parser = strata_debug_cli.build_parser()
        invocations = _mcp_tool_invocations()

        registered = {t["name"] for t in TOOLS}
        self.assertEqual(registered, CORE_TOOL_NAMES | ANDROID_TOOL_NAMES | DEVICE_TOOL_NAMES)
        covered = {name for name, _ in invocations}
        expected_strata_debug_tools = registered - NON_STRATA_DEBUG_TOOLS
        self.assertEqual(
            covered,
            expected_strata_debug_tools,
            f"MCP tool coverage mismatch: missing={expected_strata_debug_tools - covered}",
        )

        captured: list[tuple[str, list[str]]] = []

        def fake_exec(cmd: str, timeout: float = 30.0, **_: Any) -> dict[str, Any]:
            argv = shlex.split(cmd)
            if argv and argv[0] == "strata-debug":
                captured.append((current_tool, argv[1:]))
            if argv[:3] == ["strata-debug", "cdp", "list-targets"]:
                return {"rc": 0, "json": {"sockets": [{"pid": 1234, "socket": "webview_devtools_remote_1234", "process": "com.example.app"}]}}
            return {"rc": 0, "stdout": "{}", "stderr": "", "json": {"ok": True}}

        for tool_name, tool_args in invocations:
            current_tool = tool_name
            if tool_name == "forward_cdp":
                import strata_mcp.tools_core as tc
                orig_fwd = tc.open_ssh_port_forward
                tc.open_ssh_port_forward = lambda *a, **kw: {"ok": True, "route": "mock"}  # type: ignore[assignment]
                try:
                    dispatch_tool(tool_name, tool_args, exec_fn=fake_exec)
                finally:
                    tc.open_ssh_port_forward = orig_fwd
            else:
                dispatch_tool(tool_name, tool_args, exec_fn=fake_exec)

        self.assertGreaterEqual(len(captured), len(invocations))
        for tool_name, argv in captured:
            try:
                ns = parser.parse_args(argv)
            except SystemExit as exc:
                self.fail(f"MCP tool '{tool_name}' emitted invalid strata-debug argv {argv!r} (exit={exc.code})")
            self.assertTrue(getattr(ns, "cmd", None), f"Empty parsed cmd for {tool_name}: {argv!r}")

    def test_host_exec_fallback_env_matches_strata_runtime_sync_rs(self) -> None:
        host_exec_text = open(os.path.join(REPO_ROOT, "container", "host-exec"), encoding="utf-8").read()
        sync_rs_text = open(os.path.join(REPO_ROOT, "crates", "strata-runtime", "src", "sync.rs"), encoding="utf-8").read()

        # Extract fallback `KEY="VALUE"` lines from `run_nsenter \` block at end of container/host-exec.
        fallback_block = host_exec_text.split("run_nsenter \\", 1)[1]
        host_exec_env: dict[str, str] = {}
        for key, val in re.findall(r"^\s+([A-Z0-9_]+)=\"([^\"]+)\"", fallback_block, re.MULTILINE):
            if key == "TERM":
                continue
            host_exec_env[key] = val

        # Extract default KEY=VALUE pairs from `render_host_env` in crates/strata-runtime/src/sync.rs.
        render_block = sync_rs_text.split("pub fn render_host_env", 1)[1].split("for (key, val)", 1)[0]
        sync_env: dict[str, str] = {}
        for key, val in re.findall(r'"([A-Z0-9_]+)=([^"\n]+)"\.to_owned\(\)', render_block):
            sync_env[key] = val
        for key, default_val in re.findall(r'env_or\("([A-Z0-9_]+)",\s*"([^"]+)"\)', render_block):
            sync_env[key] = default_val

        self.assertEqual(
            host_exec_env,
            sync_env,
            "Fallback env in container/host-exec drifted from crates/strata-runtime/src/sync.rs",
        )

    def test_config_json_defaults_match_contract_schema_and_golden_fixture(self) -> None:
        config = json.load(open(os.path.join(REPO_ROOT, "config.json"), encoding="utf-8"))
        spec = json.load(open(os.path.join(REPO_ROOT, "contracts", "strata.contract.json"), encoding="utf-8"))
        schema = json.load(open(os.path.join(REPO_ROOT, "contracts", "schemas", "strata.schema.json"), encoding="utf-8"))
        golden = json.load(open(os.path.join(REPO_ROOT, "contracts", "fixtures", "StrataConfig.default.json"), encoding="utf-8"))

        self.assertEqual(config, golden, "Shipped config.json drifted from StrataConfig.default.json")

        proto = {k: v["value"] for k, v in spec["constants"]["protocol"]["items"].items()}
        paths = {k: v["value"] for k, v in spec["constants"]["paths"]["items"].items()}

        self.assertEqual(config["version"], proto["CONFIG_SCHEMA_VERSION"])
        self.assertEqual(config["defaultContainer"], proto["DEFAULT_CONTAINER_ID"])
        self.assertEqual(config["sideveil"]["hubSock"], paths["DEFAULT_SIDEVEIL_HUB_SOCK"])
        self.assertEqual(config["sideveil"]["pluginId"], proto["SIDEVEIL_PLUGIN_ID"])

        main = config["containers"][proto["DEFAULT_CONTAINER_ID"]]
        self.assertEqual(main["ssh"]["listenAddr"], proto["DEFAULT_SSH_LISTEN_ADDR"])
        self.assertEqual(main["ssh"]["port"], proto["DEFAULT_SSH_PORT"])
        self.assertEqual(main["ssh"]["keepaliveSecs"], proto["DEFAULT_SSH_KEEPALIVE_SECS"])
        self.assertEqual(main["oomScoreAdj"], proto["DEFAULT_OOM_SCORE_ADJ"])
        self.assertEqual(main["extraCharDevs"], [proto["DEFAULT_EXTRA_CHAR_DEV"]])

        # Verify exact property sets against JSON Schema $defs.
        strata_props = set(schema["$defs"]["StrataConfig"]["properties"].keys())
        container_props = set(schema["$defs"]["ContainerConfig"]["properties"].keys())
        self.assertEqual(set(config.keys()), strata_props)
        self.assertEqual(set(main.keys()), container_props)


if __name__ == "__main__":
    unittest.main()
