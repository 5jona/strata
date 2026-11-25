//! L2b display geometry mapping, trajectory planning, and gesture dispatch.

mod uinput;

use std::time::Duration;

use regex::Regex;
use serde_json::{Value, json};

use crate::host::run_host;
use uinput::UInputTouch;

/// Active display geometry including orientation for logical-to-physical touch mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DisplayGeometry {
    pub(crate) logical_w: i32,
    pub(crate) logical_h: i32,
    pub(crate) phys_w: i32,
    pub(crate) phys_h: i32,
    pub(crate) dpi: i32,
    pub(crate) rotation: u8,
}

fn scale_axis(v: i32, src_len: i32, dst_len: i32) -> i32 {
    if src_len == dst_len || src_len <= 1 || dst_len <= 1 {
        return v;
    }
    ((i64::from(v) * i64::from(dst_len - 1) + i64::from(src_len - 1) / 2) / i64::from(src_len - 1))
        as i32
}

/// Maps logical screen coordinates into unrotated panel coordinates accounting for rotation.
pub(crate) fn logical_to_physical(x: i32, y: i32, g: DisplayGeometry) -> (i32, i32) {
    let lx = x.clamp(0, g.logical_w.saturating_sub(1));
    let ly = y.clamp(0, g.logical_h.saturating_sub(1));
    let (rot_w, rot_h) = if g.rotation % 2 == 1 {
        (g.phys_h, g.phys_w)
    } else {
        (g.phys_w, g.phys_h)
    };
    let rx = scale_axis(lx, g.logical_w, rot_w);
    let ry = scale_axis(ly, g.logical_h, rot_h);
    let (px, py) = match g.rotation % 4 {
        1 => (g.phys_w - 1 - ry, rx),
        2 => (g.phys_w - 1 - rx, g.phys_h - 1 - ry),
        3 => (ry, g.phys_h - 1 - rx),
        _ => (rx, ry),
    };
    (
        px.clamp(0, g.phys_w.saturating_sub(1)),
        py.clamp(0, g.phys_h.saturating_sub(1)),
    )
}

/// Generates edge-swipe trajectory points `(x, y, dt_ms)` with optional cancel-back rebound.
pub(crate) fn compute_edge_swipe_points(
    g: DisplayGeometry,
    side: &str,
    y_percent: f64,
    distance_dp: f64,
    duration_ms: u64,
    cancel_back: bool,
) -> Vec<(i32, i32, u64)> {
    let y = (((g.logical_h as f64) * y_percent.clamp(5.0, 95.0) / 100.0).round() as i32)
        .clamp(0, g.logical_h - 1);
    let dist_px = ((distance_dp * (g.dpi as f64) / 160.0).round() as i32).max(12);
    let (x0, x1) = if side == "left" {
        (1, (1 + dist_px).min(g.logical_w - 2))
    } else {
        (g.logical_w - 2, (g.logical_w - 2 - dist_px).max(1))
    };
    let steps = (duration_ms / 12).clamp(8, 60) as usize;
    let dt = (duration_ms / (steps as u64)).max(4);
    let mut pts = Vec::with_capacity(steps * 2 + 1);
    let lerp = |a: i32, b: i32, i: usize| -> i32 {
        (f64::from(a) + f64::from(b - a) * (i as f64) / (steps as f64)).round() as i32
    };
    pts.extend((0..=steps).map(|i| (lerp(x0, x1, i), y, dt)));
    if cancel_back {
        pts.extend((1..=steps).map(|i| (lerp(x1, x0, i), y, dt)));
    }
    pts
}

/// Queries live display geometry and rotation via Android `dumpsys input` / `wm`.
pub(crate) async fn query_display_geometry() -> DisplayGeometry {
    let (in_res, wm_size, wm_den) = tokio::join!(
        run_host(
            &["/system/bin/dumpsys", "input"],
            None,
            Duration::from_secs(5)
        ),
        run_host(&["/system/bin/wm", "size"], None, Duration::from_secs(4)),
        run_host(&["/system/bin/wm", "density"], None, Duration::from_secs(4)),
    );
    resolve_display_geometry(
        &in_res.stdout_text(),
        &wm_size.stdout_text(),
        &wm_den.stdout_text(),
    )
}

fn parse_named_size(text: &str, pat: &str) -> Option<(i32, i32)> {
    let c = Regex::new(pat).ok()?.captures(text)?;
    let (w, h) = (c[1].parse::<i32>().ok()?, c[2].parse::<i32>().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

fn parse_last_int(text: &str, pat: &str) -> Option<i32> {
    Regex::new(pat)
        .ok()?
        .captures_iter(text)
        .last()?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

fn normalize_unrotated_size(lw: i32, lh: i32, pw: i32, ph: i32, rot: u8) -> (i32, i32) {
    if lw == lh || pw == ph {
        return (pw, ph);
    }
    let same_aspect = (lw > lh) == (pw > ph);
    if rot.is_multiple_of(2) != same_aspect {
        (ph, pw)
    } else {
        (pw, ph)
    }
}

fn resolve_display_geometry(
    dumpsys_input: &str,
    wm_size: &str,
    wm_density: &str,
) -> DisplayGeometry {
    let dpi = parse_last_int(wm_density, r"(\d+)")
        .or_else(|| parse_last_int(dumpsys_input, r"densityDpi=(\d+)"))
        .unwrap_or(420);
    let wm_phys = parse_named_size(wm_size, r"Physical size:\s*(\d+)x(\d+)");
    let wm_over = parse_named_size(wm_size, r"Override size:\s*(\d+)x(\d+)");

    let ((lw, lh), (pw, ph), rot) =
        if let Some((lw, lh, pw, ph, rot)) = parse_viewport(dumpsys_input) {
            let (raw_w, raw_h) = wm_phys.unwrap_or((pw, ph));
            (
                (lw, lh),
                normalize_unrotated_size(lw, lh, raw_w, raw_h, rot),
                rot,
            )
        } else {
            let phys = wm_phys
                .or_else(|| parse_named_size(wm_size, r"(\d+)x(\d+)"))
                .unwrap_or((1080, 2400));
            (wm_over.unwrap_or(phys), phys, 0)
        };
    DisplayGeometry {
        logical_w: lw,
        logical_h: lh,
        phys_w: pw,
        phys_h: ph,
        dpi,
        rotation: rot,
    }
}

fn parse_viewport(text: &str) -> Option<(i32, i32, i32, i32, u8)> {
    let log_re = Regex::new(r"logicalFrame=\[(\d+),\s*(\d+),\s*(\d+),\s*(\d+)\]").ok()?;
    let bounds = parse_named_size(text, r"DisplayBounds:\s*(\d+)x(\d+)");

    for line in text.lines() {
        if !line.contains("Viewport INTERNAL")
            || line.contains("isActive=[0]")
            || line.contains("isActive=false")
        {
            continue;
        }
        let lc = log_re.captures(line)?;
        let lw = lc[3].parse::<i32>().ok()? - lc[1].parse::<i32>().ok()?;
        let lh = lc[4].parse::<i32>().ok()? - lc[2].parse::<i32>().ok()?;
        let rot = (parse_last_int(line, r"orientation=(\d+)").unwrap_or(0) as u8) % 4;
        let raw_phys = bounds.or_else(|| parse_named_size(line, r"deviceSize=\[(\d+),\s*(\d+)\]"));
        let (pw, ph) = match raw_phys {
            Some((w, h)) => normalize_unrotated_size(lw, lh, w, h, rot),
            None if rot % 2 == 1 => (lh, lw),
            None => (lw, lh),
        };
        if lw > 0 && lh > 0 && pw > 0 && ph > 0 {
            return Some((lw, lh, pw, ph, rot));
        }
    }
    None
}

/// High-level gesture request executed by CLI or `observe`.
#[derive(Debug, Clone)]
pub(crate) enum GestureKind {
    Tap {
        x: i32,
        y: i32,
    },
    LongPress {
        x: i32,
        y: i32,
        duration_ms: u64,
    },
    Swipe {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        duration_ms: u64,
    },
    EdgeSwipe {
        side: String,
        y_percent: f64,
        distance_dp: f64,
        duration_ms: u64,
        cancel_back: bool,
    },
}

/// Dispatches a gesture via `/dev/uinput` on a blocking thread and returns JSON metadata.
pub(crate) async fn run_gesture(kind: GestureKind) -> Value {
    let geom = query_display_geometry().await;
    let res = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let mut dev = UInputTouch::open(geom)?;
        match kind {
            GestureKind::Tap { x, y } => {
                dev.tap(x, y, 35)?;
                Ok(json!({"ok": true, "gesture": "tap", "x": x, "y": y}))
            }
            GestureKind::LongPress { x, y, duration_ms } => {
                dev.tap(x, y, duration_ms)?;
                Ok(json!({"ok": true, "gesture": "long-press", "x": x, "y": y, "durationMs": duration_ms}))
            }
            GestureKind::Swipe { x0, y0, x1, y1, duration_ms } => {
                dev.swipe(x0, y0, x1, y1, duration_ms)?;
                Ok(json!({"ok": true, "gesture": "swipe", "durationMs": duration_ms}))
            }
            GestureKind::EdgeSwipe { side, y_percent, distance_dp, duration_ms, cancel_back } => {
                let pts = compute_edge_swipe_points(geom, &side, y_percent, distance_dp, duration_ms, cancel_back);
                dev.replay_points(&pts)?;
                Ok(json!({"ok": true, "gesture": "edge-swipe", "points": pts.len(), "side": side}))
            }
        }
    })
    .await;

    match res {
        Ok(Ok(mut v)) => {
            v["rotation"] = json!(geom.rotation);
            v["logicalSize"] = json!([geom.logical_w, geom.logical_h]);
            v["physicalSize"] = json!([geom.phys_w, geom.phys_h]);
            v
        }
        Ok(Err(err)) => json!({"ok": false, "error": err}),
        Err(err) => json!({"ok": false, "error": format!("gesture task failed: {err}")}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tablet_portrait_and_landscape_rotations() {
        let dumpsys_rot1 = "      Viewport INTERNAL: displayId=0, uniqueId=local:0, port=1, orientation=1, densityDpi=420, logicalFrame=[0, 0, 3392, 2400], physicalFrame=[0, 0, 3392, 2400], deviceSize=[3392, 2400], isActive=[1]";
        let g1 = resolve_display_geometry(dumpsys_rot1, "Physical size: 2400x3392\n", "420\n");
        assert_eq!((g1.logical_w, g1.logical_h), (3392, 2400));
        assert_eq!((g1.phys_w, g1.phys_h, g1.rotation), (2400, 3392, 1));
        assert_eq!(logical_to_physical(175, 930, g1), (1469, 175));
        assert_eq!(logical_to_physical(0, 0, g1), (2399, 0));

        // Even when `wm size` fails, rotated `deviceSize` is normalized to unrotated panel bounds.
        let g1_no_wm = resolve_display_geometry(dumpsys_rot1, "", "");
        assert_eq!((g1_no_wm.phys_w, g1_no_wm.phys_h), (2400, 3392));

        let g0 = DisplayGeometry {
            logical_w: 2400,
            logical_h: 3392,
            rotation: 0,
            ..g1
        };
        assert_eq!(logical_to_physical(175, 930, g0), (175, 930));

        let g3 = DisplayGeometry { rotation: 3, ..g1 };
        assert_eq!(logical_to_physical(175, 930, g3), (930, 3216));
        assert_eq!(logical_to_physical(0, 0, g3), (0, 3391));

        let pts = compute_edge_swipe_points(g1, "right", 50.0, 96.0, 240, true);
        assert!(pts.len() >= 17);
        assert_eq!(pts.first().map(|p| p.0), pts.last().map(|p| p.0));
    }
}
