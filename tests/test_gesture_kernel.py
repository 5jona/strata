"""Host-side unit tests for uinput gesture replay, L5 kernel probes, and CDP socket scan."""

from __future__ import annotations

import os
import sys
import tempfile
import unittest

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEBUG_DIR = os.path.join(REPO_ROOT, "container", "strata-debug")
if DEBUG_DIR not in sys.path:
    sys.path.insert(0, DEBUG_DIR)

import cdp_bridge
import gesture_replay
import kernel_probe


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


if __name__ == "__main__":
    unittest.main()
