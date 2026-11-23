"""Host-side unit tests for incremental logcat cursor, CDP audit JS, and sys_probe parsers."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEBUG_DIR = os.path.join(REPO_ROOT, "container", "strata-debug")
if DEBUG_DIR not in sys.path:
    sys.path.insert(0, DEBUG_DIR)

import cdp_audit_js
import logcat_cursor
import sys_probe


class TestLogcatAndSysProbe(unittest.TestCase):
    def test_logcat_invalid_utf8_bytes_do_not_crash(self) -> None:
        real_run = subprocess.run
        bad_payload = (
            b"11-23 12:00:00.100  1000  1001 W BinaryTag: bad-byte=\x8f-end\n"
            b"11-23 12:00:00.200  1000  1001 E BinaryTag: ok-line\n"
        )

        def _fake_run(cmd, *args, **kwargs):
            # Execute a real child process writing invalid UTF-8 bytes to verify errors='replace'
            return real_run(
                [sys.executable, "-c", f"import sys; sys.stdout.buffer.write({bad_payload!r})"],
                *args,
                **kwargs,
            )

        with tempfile.TemporaryDirectory() as td:
            cpath = os.path.join(td, "bad.cursor")
            with mock.patch.object(logcat_cursor.subprocess, "run", side_effect=_fake_run):
                marked = logcat_cursor.mark_cursor(cpath)
                self.assertEqual(marked["ts"], "11-23 12:00:00.200")
                res = logcat_cursor.read_incremental(cpath, priority="W", advance=False)
                self.assertTrue(res["ok"])
                # Remove cursor to read both lines including the invalid-UTF-8 line
                os.unlink(cpath)
                res_all = logcat_cursor.read_incremental(cpath, priority="W", advance=False)
                self.assertTrue(res_all["ok"])
                self.assertEqual(res_all["count"], 2)
                self.assertIn("\ufffd", res_all["entries"][0]["msg"])

    def test_cdp_audit_js_includes_truncation_and_small_targets(self) -> None:
        js = cdp_audit_js.DOM_AUDIT_JS
        for key in ("truncatedCount", "truncated", "smallTargetsCount", "smallTargets", "textOverflow"):
            self.assertIn(key, js)

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


if __name__ == "__main__":
    unittest.main()
