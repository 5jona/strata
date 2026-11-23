"""L3 raw uncompressed screencap, ROI crop, local pixel/blur/SSIM health, and compact encoding."""

from __future__ import annotations

import math
import os
import shutil
import struct
import subprocess
import tempfile
import time
import zlib
from typing import Any

from host_runner import HOST_EXEC, run_host_bytes


def parse_screencap_header(data: bytes) -> tuple[int, int, int, bytes]:
    """Parses raw Android `screencap` (no `-p`) 12-byte or 16-byte header -> `(w, h, fmt, rgba)`."""
    if len(data) < 16:
        raise ValueError(f"raw screencap payload too small ({len(data)} bytes)")
    w, h, fmt = struct.unpack_from("<III", data, 0)
    expected = w * h * 4
    if w == 0 or h == 0 or expected <= 0:
        raise ValueError(f"invalid screencap dimensions {w}x{h}")
    if len(data) - 16 == expected:
        return w, h, fmt, data[16:]
    if len(data) - 12 == expected:
        return w, h, fmt, data[12:]
    if len(data) >= 16 + expected:
        return w, h, fmt, data[16 : 16 + expected]
    if len(data) >= 12 + expected:
        return w, h, fmt, data[12 : 12 + expected]
    raise ValueError(f"screencap byte size mismatch: got {len(data)}, expected ~{expected + 16}")


def parse_roi(spec: str | None) -> tuple[int, int, int, int] | None:
    if not spec:
        return None
    parts = [int(p.strip()) for p in spec.split(",")]
    if len(parts) != 4:
        raise ValueError("ROI must be 'x,y,w,h'")
    return parts[0], parts[1], parts[2], parts[3]


def crop_rgba(
    rgba: bytes, width: int, height: int, roi: tuple[int, int, int, int] | None
) -> tuple[bytes, int, int]:
    if roi is None:
        return rgba, width, height
    x, y, rw, rh = roi
    x0 = max(0, min(width - 1, x))
    y0 = max(0, min(height - 1, y))
    x1 = max(x0 + 1, min(width, x0 + rw))
    y1 = max(y0 + 1, min(height, y0 + rh))
    cw, ch = x1 - x0, y1 - y0
    stride = width * 4
    rows = [rgba[(row * stride) + (x0 * 4) : (row * stride) + (x1 * 4)] for row in range(y0, y1)]
    return b"".join(rows), cw, ch


def _luma_grid(rgba: bytes, width: int, height: int, max_dim: int = 160) -> tuple[list[float], int, int]:
    step_x = max(1, width // max_dim)
    step_y = max(1, height // max_dim)
    gw = (width + step_x - 1) // step_x
    gh = (height + step_y - 1) // step_y
    out: list[float] = []
    stride = width * 4
    for gy in range(gh):
        row_off = (gy * step_y) * stride
        for gx in range(gw):
            off = row_off + (gx * step_x) * 4
            out.append(0.299 * rgba[off] + 0.587 * rgba[off + 1] + 0.114 * rgba[off + 2])
    return out, gw, gh


def analyze_rgba_health(rgba: bytes, width: int, height: int) -> dict[str, Any]:
    luma, gw, gh = _luma_grid(rgba, width, height)
    n = max(1, len(luma))
    mean = sum(luma) / n
    var = sum((v - mean) ** 2 for v in luma) / n
    std = math.sqrt(var)

    # Check RGB uniformity across sampled grid to detect solid-color screens accurately
    step_x = max(1, width // 160)
    step_y = max(1, height // 160)
    stride = width * 4
    first_rgb = (rgba[0], rgba[1], rgba[2]) if len(rgba) >= 3 else (0, 0, 0)
    max_rgb_delta = 0
    for gy in range(gh):
        row_off = (gy * step_y) * stride
        for gx in range(gw):
            off = row_off + (gx * step_x) * 4
            d = max(
                abs(rgba[off] - first_rgb[0]),
                abs(rgba[off + 1] - first_rgb[1]),
                abs(rgba[off + 2] - first_rgb[2]),
            )
            if d > max_rgb_delta:
                max_rgb_delta = d

    # Discrete 4-neighbor Laplacian variance (sharpness / blur proxy)
    lap_vals: list[float] = []
    for y in range(1, gh - 1):
        row = y * gw
        for x in range(1, gw - 1):
            c = luma[row + x]
            lap = luma[row + x - 1] + luma[row + x + 1] + luma[row - gw + x] + luma[row + gw + x] - 4.0 * c
            lap_vals.append(lap)
    if lap_vals:
        lm = sum(lap_vals) / len(lap_vals)
        lap_var = sum((v - lm) ** 2 for v in lap_vals) / len(lap_vals)
    else:
        lap_var = 0.0

    is_solid = max_rgb_delta <= 2
    return {
        "width": width,
        "height": height,
        "lumaMean": round(mean, 2),
        "lumaStd": round(std, 2),
        "laplacianVar": round(lap_var, 2),
        "isBlack": mean < 3.0 and std < 2.0,
        "isBlank": std < 1.5,
        "isSolidColor": is_solid,
    }


def compute_ssim_luma(rgba_a: bytes, rgba_b: bytes, width: int, height: int) -> dict[str, float]:
    la, _, _ = _luma_grid(rgba_a, width, height)
    lb, _, _ = _luma_grid(rgba_b, width, height)
    n = min(len(la), len(lb))
    if n == 0:
        return {"ssim": 0.0, "mad": 255.0}
    ma = sum(la[:n]) / n
    mb = sum(lb[:n]) / n
    va = sum((la[i] - ma) ** 2 for i in range(n)) / n
    vb = sum((lb[i] - mb) ** 2 for i in range(n)) / n
    cov = sum((la[i] - ma) * (lb[i] - mb) for i in range(n)) / n
    c1, c2 = (0.01 * 255) ** 2, (0.03 * 255) ** 2
    ssim = ((2 * ma * mb + c1) * (2 * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2))
    mad = sum(abs(la[i] - lb[i]) for i in range(n)) / n
    return {"ssim": round(ssim, 4), "mad": round(mad, 2)}


def encode_png_pure(rgba: bytes, width: int, height: int) -> bytes:
    """Zero-dependency PNG encoder fallback when Pillow is not installed."""
    def _chunk(tag: bytes, payload: bytes) -> bytes:
        crc = zlib.crc32(tag + payload) & 0xFFFFFFFF
        return struct.pack("!I", len(payload)) + tag + payload + struct.pack("!I", crc)

    stride = width * 4
    raw = b"".join(b"\x00" + rgba[y * stride : (y + 1) * stride] for y in range(height))
    ihdr = struct.pack("!IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + _chunk(b"IHDR", ihdr)
        + _chunk(b"IDAT", zlib.compress(raw, level=6))
        + _chunk(b"IEND", b"")
    )


def downsample_rgba(rgba: bytes, width: int, height: int, max_width: int | None) -> tuple[bytes, int, int]:
    """Proportional nearest-neighbor RGBA downsampler (zero external deps)."""
    if not max_width or max_width <= 0 or width <= max_width:
        return rgba, width, height
    new_w = max(1, int(max_width))
    new_h = max(1, round(height * new_w / width))
    mv, stride = memoryview(rgba), width * 4
    xs = [min(width - 1, (x * width) // new_w) * 4 for x in range(new_w)]
    out = bytearray(new_w * new_h * 4)
    pos = 0
    for y in range(new_h):
        row = min(height - 1, (y * height) // new_h) * stride
        for xo in xs:
            out[pos : pos + 4] = mv[row + xo : row + xo + 4]
            pos += 4
    return bytes(out), new_w, new_h


def save_encoded_image(
    rgba: bytes, width: int, height: int, out_path: str, fmt: str = "webp", quality: int = 75, max_width: int | None = None
) -> dict[str, Any]:
    parent = os.path.dirname(os.path.abspath(out_path)) or "."
    os.makedirs(parent, exist_ok=True)
    tmp_out: str | None = None
    tmp_png: str | None = None
    try:
        try:
            from PIL import Image  # type: ignore[import-not-found]

            img = Image.frombytes("RGBA", (width, height), rgba)
            if max_width and max_width > 0 and width > max_width:
                new_h = max(1, round(height * max_width / width))
                img = img.resize((max_width, new_h), getattr(getattr(Image, "Resampling", Image), "LANCZOS", 1))
                width, height = img.size
            eff_fmt = fmt.upper()
            if eff_fmt in {"JPG", "JPEG"}:
                img, eff_fmt = img.convert("RGB"), "JPEG"
            fd, tmp_out = tempfile.mkstemp(prefix=".strata-vis-", dir=parent)
            os.close(fd)
            img.save(tmp_out, format=eff_fmt, quality=quality)
            os.replace(tmp_out, out_path)
            tmp_out = None
            return {"path": out_path, "format": eff_fmt.lower(), "width": width, "height": height, "bytes": os.path.getsize(out_path)}
        except ImportError:
            rgba, width, height = downsample_rgba(rgba, width, height, max_width)
            png_bytes = encode_png_pure(rgba, width, height)
            if fmt.lower() == "webp" and shutil.which("cwebp"):
                fd_p, tmp_png = tempfile.mkstemp(prefix=".strata-vis-", suffix=".png", dir=parent)
                with os.fdopen(fd_p, "wb") as f:
                    f.write(png_bytes)
                fd_o, tmp_out = tempfile.mkstemp(prefix=".strata-vis-", suffix=".webp", dir=parent)
                os.close(fd_o)
                res = subprocess.run(["cwebp", "-quiet", "-q", str(quality), tmp_png, "-o", tmp_out], timeout=15.0, check=False)
                if res.returncode == 0 and os.path.isfile(tmp_out):
                    os.replace(tmp_out, out_path)
                    tmp_out = None
                    return {"path": out_path, "format": "webp", "width": width, "height": height, "bytes": os.path.getsize(out_path)}
            fd_o, tmp_out = tempfile.mkstemp(prefix=".strata-vis-", suffix=".png", dir=parent)
            with os.fdopen(fd_o, "wb") as f:
                f.write(png_bytes)
            actual_out = out_path
            root, cur_ext = os.path.splitext(out_path)
            if cur_ext.lower() in {".webp", ".jpg", ".jpeg"}:
                actual_out = f"{root}.png"
            os.replace(tmp_out, actual_out)
            tmp_out = None
            return {"path": actual_out, "format": "png", "width": width, "height": height, "bytes": len(png_bytes)}
    finally:
        for p in (tmp_png, tmp_out):
            if p and os.path.exists(p):
                try:
                    os.unlink(p)
                except OSError:
                    pass


def stitch_burst_sheet(frames: list[tuple[bytes, int, int]]) -> tuple[bytes, int, int]:
    """Stitches up to 4 equal-sized RGBA frames into a 2x2 contact sheet."""
    if not frames:
        raise ValueError("no frames to stitch")
    w, h = frames[0][1], frames[0][2]
    padded = list(frames[:4])
    blank = b"\x00" * (w * h * 4)
    while len(padded) < 4:
        padded.append((blank, w, h))
    stride = w * 4
    rows = [padded[0][0][y * stride : (y + 1) * stride] + padded[1][0][y * stride : (y + 1) * stride] for y in range(h)]
    rows.extend(padded[2][0][y * stride : (y + 1) * stride] + padded[3][0][y * stride : (y + 1) * stride] for y in range(h))
    return b"".join(rows), w * 2, h * 2


def capture_raw_frame(timeout: float = 10.0) -> tuple[bytes, int, int]:
    res = run_host_bytes(["/system/bin/screencap"], timeout=timeout)
    if res.returncode != 0 or not res.stdout:
        raise RuntimeError(f"raw screencap failed (rc={res.returncode})")
    w, h, _fmt, rgba = parse_screencap_header(res.stdout)
    return rgba, w, h


def run_visual_check(
    *, roi_spec: str | None = None, out_path: str | None = None, fmt: str = "webp", quality: int = 75,
    golden_raw: str | None = None, burst_count: int = 1, burst_interval_ms: int = 80, max_width: int | None = None,
) -> dict[str, Any]:
    roi = parse_roi(roi_spec)
    frames: list[tuple[bytes, int, int]] = []
    for idx in range(max(1, min(4, burst_count))):
        raw, w, h = capture_raw_frame()
        frames.append(crop_rgba(raw, w, h, roi))
        if idx + 1 < burst_count:
            time.sleep(burst_interval_ms / 1000.0)

    rgba, cw, ch = frames[-1]
    result: dict[str, Any] = {"ok": True, "health": analyze_rgba_health(rgba, cw, ch)}
    if golden_raw and os.path.isfile(golden_raw):
        with open(golden_raw, "rb") as f:
            gw, gh, _, g_rgba = parse_screencap_header(f.read())
        g_crop, gcw, gch = crop_rgba(g_rgba, gw, gh, roi)
        if (gcw, gch) == (cw, ch):
            result["goldenDiff"] = compute_ssim_luma(rgba, g_crop, cw, ch)

    if out_path:
        out_rgba, ow, oh = stitch_burst_sheet(frames) if len(frames) > 1 else (rgba, cw, ch)
        result["saved"] = save_encoded_image(out_rgba, ow, oh, out_path, fmt=fmt, quality=quality, max_width=max_width)
    return result
