"""Host-side unit tests for generic Android debugging modules in container/strata-debug."""

from __future__ import annotations

import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEBUG_DIR = os.path.join(REPO_ROOT, "container", "strata-debug")
if DEBUG_DIR not in sys.path:
    sys.path.insert(0, DEBUG_DIR)

import app_ops
import cli
import host_inspect
import lsposed_ops
import sys_probe
import ui_tree


SAMPLE_UI_XML = """<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="com.example.app:id/root" class="android.widget.FrameLayout"
        package="com.example.app" content-desc="" clickable="false" enabled="true" bounds="[0,0][1080,2400]">
    <node index="0" text="Sign In" resource-id="com.example.app:id/btn_login" class="android.widget.Button"
          package="com.example.app" content-desc="Login action" clickable="true" enabled="true" bounds="[100,200][500,320]" />
    <node index="1" text="Settings" resource-id="com.example.app:id/btn_settings" class="android.widget.Button"
          package="com.example.app" content-desc="Open settings" clickable="true" enabled="true" bounds="[600,200][1000,320]" />
    <node index="2" text="Status Banner" resource-id="com.example.app:id/banner" class="android.widget.TextView"
          package="com.example.app" content-desc="Info" clickable="false" enabled="true" bounds="[0,0][1080,100]" />
  </node>
</hierarchy>
"""


class TestUiTreeAndAppParsers(unittest.TestCase):
    def test_parse_uiautomator_xml_bounds_filters_and_limit(self) -> None:
        res = ui_tree.parse_uiautomator_xml(SAMPLE_UI_XML)
        self.assertTrue(res["ok"])
        self.assertEqual(res["nodeCount"], 4)
        self.assertEqual(res["matchedCount"], 4)
        self.assertFalse(res["truncated"])

        login = ui_tree.parse_uiautomator_xml(SAMPLE_UI_XML, query="sign in")
        self.assertEqual(login["matchedCount"], 1)
        node = login["nodes"][0]
        self.assertEqual(node["bounds"], [100, 200, 500, 320])
        self.assertEqual(node["center"], [300, 260])

        by_rid = ui_tree.parse_uiautomator_xml(SAMPLE_UI_XML, resource_id="btn_settings")
        self.assertEqual(by_rid["matchedCount"], 1)
        self.assertEqual(by_rid["nodes"][0]["text"], "Settings")

        by_desc = ui_tree.parse_uiautomator_xml(SAMPLE_UI_XML, content_desc="login action")
        self.assertEqual(by_desc["matchedCount"], 1)

        clickable = ui_tree.parse_uiautomator_xml(SAMPLE_UI_XML, clickable_only=True, limit=1)
        self.assertEqual(clickable["matchedCount"], 2)
        self.assertEqual(len(clickable["nodes"]), 1)
        self.assertTrue(clickable["truncated"])

        bad = ui_tree.parse_uiautomator_xml("<broken")
        self.assertFalse(bad["ok"])

    def test_parse_foreground_activity_variants(self) -> None:
        s1 = "  topResumedActivity=ActivityRecord{1a2b3c u0 com.example.app/.MainActivity t42}"
        self.assertEqual(
            sys_probe.parse_foreground_activity(s1),
            {"package": "com.example.app", "activity": ".MainActivity", "component": "com.example.app/.MainActivity"},
        )
        s2 = "  mCurrentFocus=Window{4d5e6f u0 com.example.demo/com.example.demo.SettingsActivity}"
        self.assertEqual(sys_probe.parse_foreground_activity(s2)["package"], "com.example.demo")
        self.assertIsNone(sys_probe.parse_foreground_activity("no focus here")["package"])

    def test_parse_dumpsys_package_and_meminfo(self) -> None:
        pkg_dump = (
            "Package [com.example.app] (abc123):\n"
            "  userId=10245\n"
            "  codePath=/data/app/~~hash/com.example.app-1\n"
            "  resourcePath=/data/app/~~hash/com.example.app-1\n"
            "  versionCode=42 minSdk=29 targetSdk=35\n"
            "  versionName=2.4.1\n"
        )
        info = app_ops.parse_dumpsys_package(pkg_dump)
        self.assertEqual(info["versionName"], "2.4.1")
        self.assertEqual(info["versionCode"], 42)
        self.assertEqual(info["minSdk"], 29)
        self.assertEqual(info["targetSdk"], 35)
        self.assertEqual(info["uid"], 10245)
        self.assertEqual(info["codePath"], "/data/app/~~hash/com.example.app-1")

        mem_dump = (
            " App Summary\n"
            "   Java Heap:    18432\n"
            "   Native Heap:  32768\n"
            "   TOTAL PSS:    91200\n"
        )
        mem = app_ops.parse_meminfo_summary(mem_dump)
        self.assertEqual(mem, {"totalPssKb": 91200, "javaHeapKb": 18432, "nativeHeapKb": 32768})

    def test_collect_sys_probe_auto_package_and_sideveil_gating(self) -> None:
        def fake_run(args: list[str], timeout: float = 6.0) -> subprocess.CompletedProcess[str]:
            if args[:3] == ["/system/bin/dumpsys", "activity", "activities"]:
                return subprocess.CompletedProcess(args, 0, "topResumedActivity=ActivityRecord{1 u0 com.example.app/.Home t1}", "")
            if args[:3] == ["/system/bin/dumpsys", "window", "windows"]:
                return subprocess.CompletedProcess(args, 0, "  Window #0 Window{1 u0 com.example.app/.Home}:\n    mViewVisibility=0x0 isOnScreen=true\n", "")
            return subprocess.CompletedProcess(args, 0, "", "")

        with mock.patch.object(sys_probe, "_run_host", side_effect=fake_run), mock.patch.object(
            sys_probe, "inspect_sideveil_ui", return_value={"ok": True, "panel": "open"}
        ) as sv_mock:
            res_auto = sys_probe.collect_sys_probe(package="auto", include_surfaceflinger=False)
            self.assertEqual(res_auto["package"], "com.example.app")
            self.assertIsNone(res_auto["sideveilInspect"])
            sv_mock.assert_not_called()

            res_sv = sys_probe.collect_sys_probe(package="pochita.sideveil.host", include_surfaceflinger=False)
            self.assertEqual(res_sv["sideveilInspect"], {"ok": True, "panel": "open"})
            sv_mock.assert_called_once()


class TestLsposedAndHostInspect(unittest.TestCase):
    def test_lsposed_modules_wal_db_and_missing_fallback(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            db_file = os.path.join(td, "modules_config.db")
            conn = sqlite3.connect(db_file)
            conn.execute("PRAGMA journal_mode=WAL")
            conn.execute("CREATE TABLE modules (mid INTEGER PRIMARY KEY, module_pkg_name TEXT, enabled INTEGER, apk_path TEXT)")
            conn.execute("CREATE TABLE scope (mid INTEGER, app_pkg_name TEXT, user_id INTEGER)")
            conn.execute("INSERT INTO modules VALUES (1, 'com.example.hook', 1, '/data/app/hook.apk')")
            conn.execute("INSERT INTO modules VALUES (2, 'com.example.other', 0, '/data/app/other.apk')")
            conn.execute("INSERT INTO scope VALUES (1, 'com.example.target', 0)")
            conn.commit()
            # Keep conn open briefly to ensure WAL file exists during snapshot copy
            def copy_local(_src: str, dst_dir: str) -> str:
                dst = os.path.join(dst_dir, "modules_config.db")
                for sfx in ("", "-wal", "-shm"):
                    if os.path.isfile(db_file + sfx):
                        shutil.copy2(db_file + sfx, dst + sfx)
                return dst

            res = lsposed_ops.query_lsposed_modules(copy_fn=copy_local)
            conn.close()
            self.assertTrue(res["ok"])
            self.assertTrue(res["available"])
            self.assertEqual(res["count"], 2)
            mod1 = next(m for m in res["modules"] if m["package"] == "com.example.hook")
            self.assertTrue(mod1["enabled"])
            self.assertEqual(mod1["scopes"], [{"package": "com.example.target", "userId": 0}])

            filtered = lsposed_ops.query_lsposed_modules(module_filter="hook", copy_fn=copy_local)
            self.assertEqual(filtered["count"], 1)

        with tempfile.TemporaryDirectory() as td2:
            v2_db = os.path.join(td2, "modules_config.db")
            c2 = sqlite3.connect(v2_db)
            c2.execute("CREATE TABLE modules (module_pkg_name TEXT PRIMARY KEY, apk_path TEXT)")
            c2.execute("CREATE TABLE modules_state (module_pkg_name TEXT, user_id INTEGER, enabled INTEGER)")
            c2.execute("CREATE TABLE scope (module_pkg_name TEXT, app_pkg_name TEXT, user_id INTEGER)")
            c2.execute("INSERT INTO modules VALUES ('com.example.v2', '/data/app/v2.apk')")
            c2.execute("INSERT INTO modules_state VALUES ('com.example.v2', 0, 1)")
            c2.execute("INSERT INTO scope VALUES ('com.example.v2', 'com.example.target', 0)")
            c2.commit()
            c2.close()
            r2 = lsposed_ops.query_lsposed_modules(copy_fn=lambda _s, d: shutil.copy2(v2_db, os.path.join(d, "v2.db")))
            self.assertEqual(r2["modules"][0]["enabled"], True)
            self.assertEqual(r2["modules"][0]["scopes"], [{"package": "com.example.target", "userId": 0}])

        missing = lsposed_ops.query_lsposed_modules(copy_fn=lambda _s, _d: None)
        self.assertEqual(missing["ok"], True)
        self.assertEqual(missing["available"], False)

    def test_validate_readonly_sql_and_db_queries(self) -> None:
        for ok_sql in (
            "SELECT * FROM items;",
            "  -- comment\nSELECT 1",
            "/* block */ PRAGMA table_info(items)",
            "EXPLAIN SELECT 1",
            "WITH cte AS (SELECT 1) SELECT * FROM cte",
        ):
            self.assertTrue(host_inspect.validate_readonly_sql(ok_sql))

        for bad_sql in (
            "",
            "   -- only comment",
            "INSERT INTO items VALUES (1)",
            "UPDATE items SET a=1",
            "DELETE FROM items",
            "DROP TABLE items",
            "ATTACH DATABASE '/tmp/x.db' AS x",
            "SELECT 1; DROP TABLE items",
        ):
            with self.assertRaises(ValueError):
                host_inspect.validate_readonly_sql(bad_sql)

        with tempfile.TemporaryDirectory() as td:
            db_file = os.path.join(td, "sample.db")
            conn = sqlite3.connect(db_file)
            conn.execute("CREATE TABLE items (id INTEGER, name TEXT)")
            conn.executemany("INSERT INTO items VALUES (?, ?)", [(1, "alpha"), (2, "beta"), (3, "gamma")])
            conn.commit()
            conn.close()

            tables = host_inspect.db_list_tables(db_file)
            self.assertTrue(tables["ok"])
            self.assertEqual(tables["tables"], [{"name": "items", "type": "table"}])

            q = host_inspect.db_query(db_file, "SELECT id, name FROM items ORDER BY id", limit=2)
            self.assertTrue(q["ok"])
            self.assertEqual(q["columns"], ["id", "name"])
            self.assertEqual(q["rows"], [[1, "alpha"], [2, "beta"]])
            self.assertTrue(q["truncated"])

    def test_parse_getevent_devices_and_sysfs_traversal(self) -> None:
        sample = (
            "add device 1: /dev/input/event2\n"
            "  bus:      0018\n"
            "  vendor    0001\n"
            "  product   0002\n"
            "  version   0100\n"
            '  name:     "virtual_touchscreen"\n'
            "  events:\n"
            "    ABS (0003): ABS_MT_POSITION_X\n"
            "    KEY (0001): BTN_TOUCH\n"
            "add device 2: /dev/input/event3\n"
            '  name:     "gpio_keys"\n'
            "  events:\n"
            "    KEY (0001): KEY_POWER KEY_VOLUMEDOWN\n"
        )
        devs = host_inspect.parse_getevent_devices(sample)
        self.assertEqual(len(devs), 2)
        self.assertTrue(devs[0]["hasTouch"])
        self.assertEqual(devs[0]["name"], "virtual_touchscreen")
        self.assertFalse(devs[1]["hasTouch"])
        self.assertTrue(devs[1]["hasKeys"])

        for bad_path in ("/etc/passwd", "/sys/../etc/passwd", "/proc/../../root", "sys/class/net"):
            res = host_inspect.read_sysfs_node(bad_path)
            self.assertFalse(res["ok"])


class TestCliAndFileSizeLimits(unittest.TestCase):
    def test_cli_subcommands_and_defaults(self) -> None:
        parser = cli.build_parser()

        self.assertEqual(parser.parse_args(["sys-probe"]).package, "auto")
        self.assertEqual(parser.parse_args(["observe"]).package, "auto")
        self.assertIsNone(parser.parse_args(["cdp", "audit"]).target)

        ui_d = parser.parse_args(["ui", "dump", "--compressed", "--query", "Login", "--clickable-only", "--limit", "20"])
        self.assertEqual((ui_d.cmd, ui_d.ui_cmd, ui_d.compressed, ui_d.limit), ("ui", "dump", True, 20))

        ui_t = parser.parse_args(["ui", "tap", "--text", "OK", "--method", "uinput", "--index", "1"])
        self.assertEqual((ui_t.cmd, ui_t.ui_cmd, ui_t.method, ui_t.index), ("ui", "tap", "uinput", 1))

        self.assertEqual(parser.parse_args(["app", "top"]).app_cmd, "top")
        self.assertEqual(parser.parse_args(["app", "inspect"]).package, "auto")
        self.assertEqual(parser.parse_args(["app", "start", "com.example.app", "--activity", ".Main"]).activity, ".Main")
        self.assertEqual(parser.parse_args(["app", "stop", "com.example.app"]).package, "com.example.app")
        self.assertEqual(parser.parse_args(["app", "clear", "com.example.app"]).package, "com.example.app")
        inst = parser.parse_args(["app", "install", "/data/local/tmp/app.apk", "--no-grant", "--downgrade", "--no-replace"])
        self.assertEqual((inst.grant, inst.downgrade, inst.no_replace), (False, True, True))

        self.assertEqual(parser.parse_args(["lsposed", "modules", "--module", "hook"]).module, "hook")
        self.assertEqual(parser.parse_args(["lsposed", "logs", "--grep", "ERR", "--limit", "30"]).limit, 30)
        self.assertEqual(parser.parse_args(["db", "tables", "/tmp/a.db"]).db_path, "/tmp/a.db")
        self.assertEqual(parser.parse_args(["db", "query", "/tmp/a.db", "--sql", "SELECT 1"]).limit, 50)
        self.assertEqual(parser.parse_args(["ksu", "modules"]).ksu_cmd, "modules")
        self.assertEqual(parser.parse_args(["ksu", "inputs"]).ksu_cmd, "inputs")
        self.assertEqual(parser.parse_args(["ksu", "sysfs", "/proc/version"]).path, "/proc/version")

    def test_file_line_counts_within_limit(self) -> None:
        targets = [
            os.path.join(DEBUG_DIR, "sys_probe.py"),
            os.path.join(DEBUG_DIR, "observe_step.py"),
            os.path.join(DEBUG_DIR, "cdp_bridge.py"),
            os.path.join(DEBUG_DIR, "cli.py"),
            os.path.join(DEBUG_DIR, "cli_android.py"),
            os.path.join(DEBUG_DIR, "ui_tree.py"),
            os.path.join(DEBUG_DIR, "app_ops.py"),
            os.path.join(DEBUG_DIR, "lsposed_ops.py"),
            os.path.join(DEBUG_DIR, "host_inspect.py"),
            os.path.abspath(__file__),
        ]
        for path in targets:
            with open(path, encoding="utf-8") as f:
                count = sum(1 for _ in f)
            self.assertLessEqual(count, 300, f"{os.path.basename(path)} has {count} lines (> 300)")


if __name__ == "__main__":
    unittest.main()
