"""Host-side unit tests for Strata L3 visual screencap health, SSIM, and encoding."""

from __future__ import annotations

import os
import struct
import sys
import tempfile
import unittest
from unittest import mock

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEBUG_DIR = os.path.join(REPO_ROOT, "container", "strata-debug")
if DEBUG_DIR not in sys.path:
    sys.path.insert(0, DEBUG_DIR)

import visual_check


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

    def test_downsample_rgba_and_max_width_export(self) -> None:
        w, h = 20, 10
        rgba = b"\x10\x80\xf0\xff" * (w * h)
        # No-op when max_width is None or >= width
        same, sw, sh = visual_check.downsample_rgba(rgba, w, h, None)
        self.assertEqual((sw, sh, len(same)), (w, h, len(rgba)))
        same2, sw2, sh2 = visual_check.downsample_rgba(rgba, w, h, 24)
        self.assertEqual((sw2, sh2, len(same2)), (w, h, len(rgba)))

        # Proportional downscale: 20x10 -> 10x5
        small, nw, nh = visual_check.downsample_rgba(rgba, w, h, 10)
        self.assertEqual((nw, nh, len(small)), (10, 5, 10 * 5 * 4))

        # Pure-Python fallback (PIL blocked) writes valid PNG with downscaled IHDR dimensions
        with tempfile.TemporaryDirectory() as td:
            out_png = os.path.join(td, "down.png")
            real_import = __import__

            def _no_pil(name, *a, **kw):
                if name == "PIL" or name.startswith("PIL."):
                    raise ImportError("blocked for test")
                return real_import(name, *a, **kw)

            with mock.patch("builtins.__import__", side_effect=_no_pil):
                meta = visual_check.save_encoded_image(rgba, w, h, out_png, fmt="png", max_width=8)
            self.assertEqual((meta["format"], meta["width"], meta["height"]), ("png", 8, 4))
            with open(out_png, "rb") as f:
                data = f.read()
            self.assertTrue(data.startswith(b"\x89PNG\r\n\x1a\n"))
            ihdr_w, ihdr_h = struct.unpack_from("!II", data, 16)
            self.assertEqual((ihdr_w, ihdr_h), (8, 4))


if __name__ == "__main__":
    unittest.main()
