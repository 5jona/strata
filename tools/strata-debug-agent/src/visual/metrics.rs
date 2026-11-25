//! Pure-Rust RGBA pixel health metrics (black/blank/solid/blur) and SSIM comparison.

use serde_json::{Value, json};

use super::RawFrame;

fn luma_grid(frame: &RawFrame, max_dim: u32) -> (Vec<f64>, usize, usize) {
    let step_x = (frame.width / max_dim).max(1) as usize;
    let step_y = (frame.height / max_dim).max(1) as usize;
    let gw = (frame.width as usize).div_ceil(step_x);
    let gh = (frame.height as usize).div_ceil(step_y);
    let stride = (frame.width as usize) * 4;
    let mut grid = Vec::with_capacity(gw * gh);
    for gy in 0..gh {
        let row_off = (gy * step_y) * stride;
        for gx in 0..gw {
            let off = row_off + (gx * step_x) * 4;
            let r = f64::from(frame.rgba[off]);
            let g = f64::from(frame.rgba[off + 1]);
            let b = f64::from(frame.rgba[off + 2]);
            grid.push(0.299 * r + 0.587 * g + 0.114 * b);
        }
    }
    (grid, gw, gh)
}

/// Computes fast frame health metrics (mean/std luma, black, blank, solid-color, Laplacian blur).
pub(crate) fn analyze_health(frame: &RawFrame) -> Value {
    let (luma, gw, gh) = luma_grid(frame, 160);
    let n = luma.len().max(1) as f64;
    let mean = luma.iter().sum::<f64>() / n;
    let var = luma.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    let std_dev = var.sqrt();

    let step_x = (frame.width / 160).max(1) as usize;
    let step_y = (frame.height / 160).max(1) as usize;
    let stride = (frame.width as usize) * 4;
    let first = (frame.rgba[0], frame.rgba[1], frame.rgba[2]);
    let mut max_delta: u8 = 0;
    for gy in 0..gh {
        let row_off = (gy * step_y) * stride;
        for gx in 0..gw {
            let off = row_off + (gx * step_x) * 4;
            let d = frame.rgba[off]
                .abs_diff(first.0)
                .max(frame.rgba[off + 1].abs_diff(first.1))
                .max(frame.rgba[off + 2].abs_diff(first.2));
            if d > max_delta {
                max_delta = d;
            }
        }
    }

    let mut lap_vals = Vec::with_capacity(gw.saturating_sub(2) * gh.saturating_sub(2));
    for y in 1..gh.saturating_sub(1) {
        let row = y * gw;
        for x in 1..gw.saturating_sub(1) {
            let c = luma[row + x];
            let lap =
                luma[row + x - 1] + luma[row + x + 1] + luma[row - gw + x] + luma[row + gw + x]
                    - 4.0 * c;
            lap_vals.push(lap);
        }
    }
    let lap_var = if lap_vals.is_empty() {
        0.0
    } else {
        let lm = lap_vals.iter().sum::<f64>() / (lap_vals.len() as f64);
        lap_vals.iter().map(|v| (v - lm).powi(2)).sum::<f64>() / (lap_vals.len() as f64)
    };

    let is_solid = max_delta <= 2;
    json!({
        "width": frame.width,
        "height": frame.height,
        "lumaMean": (mean * 100.0).round() / 100.0,
        "lumaStd": (std_dev * 100.0).round() / 100.0,
        "laplacianVar": (lap_var * 100.0).round() / 100.0,
        "isBlack": mean < 3.0 && std_dev < 2.0,
        "isBlank": std_dev < 1.5,
        "isSolidColor": is_solid,
        "isLikelyBlurred": lap_var < 12.0 && !is_solid,
    })
}

/// Computes global luminance SSIM and mean absolute error between two same-sized frames.
pub(crate) fn compute_ssim(a: &RawFrame, b: &RawFrame) -> Value {
    let (la, _, _) = luma_grid(a, 160);
    let (lb, _, _) = luma_grid(b, 160);
    let n = la.len().min(lb.len()).max(1) as f64;
    let ma = la.iter().sum::<f64>() / n;
    let mb = lb.iter().sum::<f64>() / n;
    let (mut va, mut vb, mut cov, mut mad) = (0.0, 0.0, 0.0, 0.0);
    for (x, y) in la.iter().zip(lb.iter()) {
        let dx = x - ma;
        let dy = y - mb;
        va += dx * dx;
        vb += dy * dy;
        cov += dx * dy;
        mad += (x - y).abs();
    }
    va /= n;
    vb /= n;
    cov /= n;
    mad /= n;
    let (c1, c2) = (6.5025, 58.5225);
    let ssim =
        ((2.0 * ma * mb + c1) * (2.0 * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2));
    json!({
        "ssim": (ssim.clamp(-1.0, 1.0) * 10_000.0).round() / 10_000.0,
        "mad": (mad * 100.0).round() / 100.0,
    })
}

/// Stitches up to 4 equal-sized frames into a 2x2 contact sheet.
pub(crate) fn stitch_burst_sheet(frames: &[RawFrame]) -> Result<RawFrame, String> {
    let first = frames
        .first()
        .ok_or_else(|| "no frames to stitch".to_owned())?;
    let (w, h) = (first.width, first.height);
    let blank = RawFrame {
        width: w,
        height: h,
        rgba: vec![0u8; (w as usize) * (h as usize) * 4],
    };
    let get = |idx: usize| frames.get(idx).unwrap_or(&blank);
    let stride = (w as usize) * 4;
    let mut out = Vec::with_capacity((w as usize) * 2 * (h as usize) * 2 * 4);
    for row in 0..(h as usize) {
        out.extend_from_slice(&get(0).rgba[row * stride..(row + 1) * stride]);
        out.extend_from_slice(&get(1).rgba[row * stride..(row + 1) * stride]);
    }
    for row in 0..(h as usize) {
        out.extend_from_slice(&get(2).rgba[row * stride..(row + 1) * stride]);
        out.extend_from_slice(&get(3).rgba[row * stride..(row + 1) * stride]);
    }
    Ok(RawFrame {
        width: w * 2,
        height: h * 2,
        rgba: out,
    })
}
