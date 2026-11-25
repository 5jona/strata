//! L3 visual frame capture, ROI crop, health orchestration, and image persistence/base64 return.

pub(crate) mod encode;
pub(crate) mod metrics;

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::host::run_host;
use metrics::{analyze_health, compute_ssim, stitch_burst_sheet};

/// Parsed raw RGBA8888 frame from Android `screencap`.
#[derive(Debug, Clone)]
pub(crate) struct RawFrame {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

/// Parses raw Android `screencap` (no `-p`) 12-byte or 16-byte header into RGBA bytes.
pub(crate) fn parse_screencap_header(data: &[u8]) -> Result<RawFrame, String> {
    if data.len() < 16 {
        return Err(format!(
            "raw screencap payload too small ({} bytes)",
            data.len()
        ));
    }
    let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let expected = (w as usize)
        .checked_mul(h as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| format!("screencap dimensions overflow: {w}x{h}"))?;
    if w == 0 || h == 0 || expected == 0 {
        return Err(format!("invalid screencap dimensions {w}x{h}"));
    }
    let offset = if data.len() >= expected + 16 {
        16
    } else if data.len() >= expected + 12 {
        12
    } else {
        return Err(format!(
            "screencap byte size mismatch: got {}, expected ~{}",
            data.len(),
            expected + 16
        ));
    };
    Ok(RawFrame {
        width: w,
        height: h,
        rgba: data[offset..offset + expected].to_vec(),
    })
}

/// Parses `"x,y,w,h"` ROI specification.
pub(crate) fn parse_roi(spec: &str) -> Result<(u32, u32, u32, u32), String> {
    let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err("ROI must be 'x,y,w,h'".to_owned());
    }
    let nums: Result<Vec<u32>, _> = parts.iter().map(|p| p.parse::<u32>()).collect();
    let nums = nums.map_err(|e| format!("invalid ROI integer: {e}"))?;
    Ok((nums[0], nums[1], nums[2], nums[3]))
}

/// Crops an RGBA frame to the clamped ROI rectangle.
pub(crate) fn crop_rgba(frame: &RawFrame, roi: Option<(u32, u32, u32, u32)>) -> RawFrame {
    let Some((x, y, rw, rh)) = roi else {
        return frame.clone();
    };
    let x0 = x.min(frame.width.saturating_sub(1));
    let y0 = y.min(frame.height.saturating_sub(1));
    let x1 = x0.saturating_add(rw).max(x0 + 1).min(frame.width);
    let y1 = y0.saturating_add(rh).max(y0 + 1).min(frame.height);
    let (cw, ch) = (x1 - x0, y1 - y0);
    let stride = (frame.width as usize) * 4;
    let mut out = Vec::with_capacity((cw as usize) * (ch as usize) * 4);
    for row in y0..y1 {
        let start = (row as usize) * stride + (x0 as usize) * 4;
        out.extend_from_slice(&frame.rgba[start..start + (cw as usize) * 4]);
    }
    RawFrame {
        width: cw,
        height: ch,
        rgba: out,
    }
}

/// Captures a single raw frame from Android `/system/bin/screencap`.
pub(crate) async fn capture_raw_frame() -> Result<RawFrame, String> {
    let res = run_host(&["/system/bin/screencap"], None, Duration::from_secs(10)).await;
    if !res.ok() || res.stdout.is_empty() {
        return Err(format!(
            "raw screencap failed (rc={}): {}",
            res.code,
            res.stderr_text()
        ));
    }
    parse_screencap_header(&res.stdout)
}

/// Options for running an L3 visual capture & health check.
#[derive(Debug, Clone)]
pub(crate) struct VisualOpts {
    pub(crate) roi: Option<String>,
    pub(crate) out_path: Option<String>,
    pub(crate) format: String,
    pub(crate) quality: u8,
    pub(crate) max_width: Option<u32>,
    pub(crate) golden_raw: Option<String>,
    pub(crate) burst_sheet: bool,
    pub(crate) burst_interval_ms: u64,
    pub(crate) inline_base64: bool,
}

/// Executes raw screencap, computes health/SSIM metrics, and optionally saves or returns base64 image.
pub(crate) async fn run_visual(opts: &VisualOpts) -> Value {
    let roi = match opts.roi.as_deref() {
        Some(spec) if !spec.is_empty() => match parse_roi(spec) {
            Ok(r) => Some(r),
            Err(err) => return json!({"ok": false, "error": err}),
        },
        _ => None,
    };

    let count = if opts.burst_sheet { 4 } else { 1 };
    let mut frames = Vec::with_capacity(count);
    for idx in 0..count {
        match capture_raw_frame().await {
            Ok(raw) => frames.push(crop_rgba(&raw, roi)),
            Err(err) => return json!({"ok": false, "error": err}),
        }
        if idx + 1 < count {
            tokio::time::sleep(Duration::from_millis(opts.burst_interval_ms)).await;
        }
    }

    let Some(last) = frames.last() else {
        return json!({"ok": false, "error": "no frame captured"});
    };
    let mut out = json!({"ok": true, "health": analyze_health(last)});

    if let Some(golden) = opts.golden_raw.as_deref()
        && Path::new(golden).is_file()
        && let Ok(bytes) = std::fs::read(golden)
        && let Ok(g_raw) = parse_screencap_header(&bytes)
    {
        let g_crop = crop_rgba(&g_raw, roi);
        if (g_crop.width, g_crop.height) == (last.width, last.height) {
            out["goldenDiff"] = compute_ssim(last, &g_crop);
        }
    }

    if opts.out_path.is_some() || opts.inline_base64 {
        let target = if frames.len() > 1 {
            match stitch_burst_sheet(&frames) {
                Ok(s) => s,
                Err(e) => return json!({"ok": false, "error": e}),
            }
        } else {
            last.clone()
        };
        let encoded =
            match encode::encode_frame(&target, &opts.format, opts.quality, opts.max_width) {
                Ok(e) => e,
                Err(err) => return json!({"ok": false, "error": err}),
            };
        if let Some(path) = opts.out_path.as_deref() {
            if let Err(err) = crate::host::atomic_write(Path::new(path), &encoded.bytes) {
                return json!({"ok": false, "error": format!("save image failed: {err}")});
            }
            out["saved"] = json!({
                "path": path, "format": encoded.format, "width": encoded.width,
                "height": encoded.height, "bytes": encoded.bytes.len(),
            });
        }
        if opts.inline_base64 {
            use base64::Engine;
            out["inlineImage"] = json!({
                "mimeType": encoded.mime_type, "width": encoded.width, "height": encoded.height,
                "bytes": encoded.bytes.len(),
                "data": base64::engine::general_purpose::STANDARD.encode(&encoded.bytes),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_frame(w: u32, h: u32, rgb: [u8; 3]) -> RawFrame {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        RawFrame {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn detects_black_solid_ssim_and_crops_roi() {
        let black = solid_frame(32, 32, [0, 0, 0]);
        assert_eq!(analyze_health(&black)["isBlack"], json!(true));
        let blue = solid_frame(32, 32, [10, 80, 200]);
        assert_eq!(compute_ssim(&blue, &blue)["ssim"], json!(1.0));

        let mut raw = Vec::new();
        raw.extend_from_slice(&32u32.to_le_bytes());
        raw.extend_from_slice(&32u32.to_le_bytes());
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(&blue.rgba);
        let parsed = parse_screencap_header(&raw).expect("parse");
        let cropped = crop_rgba(&parsed, Some((4, 4, 8, 6)));
        assert_eq!((cropped.width, cropped.height), (8, 6));
    }
}
