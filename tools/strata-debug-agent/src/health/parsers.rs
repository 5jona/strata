//! Pure parsers for `dumpsys activity`, `dumpsys window windows`, and `dumpsys gfxinfo`.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForegroundApp {
    pub(crate) package: Option<String>,
    pub(crate) activity: Option<String>,
    pub(crate) component: Option<String>,
}

fn fg_regexes() -> &'static [Regex] {
    static PATS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATS.get_or_init(|| {
        [
            r"topResumedActivity=ActivityRecord\{[^}]+\s+u\d+\s+([A-Za-z0-9._]+)/([A-Za-z0-9._/$]+)",
            r"(?:mResumedActivity|ResumedActivity):\s*ActivityRecord\{[^}]+\s+u\d+\s+([A-Za-z0-9._]+)/([A-Za-z0-9._/$]+)",
            r"mCurrentFocus=Window\{[^}]+\s+u\d+\s+([A-Za-z0-9._]+)/([A-Za-z0-9._/$]+)",
            r"mFocusedApp=ActivityRecord\{[^}]+\s+u\d+\s+([A-Za-z0-9._]+)/([A-Za-z0-9._/$]+)",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

/// Extracts foreground package and activity from `dumpsys activity/window` output.
pub(crate) fn parse_foreground_activity(text: &str) -> ForegroundApp {
    for re in fg_regexes() {
        if let Some(caps) = re.captures(text) {
            let pkg = caps.get(1).map(|m| m.as_str().to_owned());
            let act = caps
                .get(2)
                .map(|m| m.as_str().trim_end_matches('}').to_owned());
            let comp = match (&pkg, &act) {
                (Some(p), Some(a)) => Some(format!("{p}/{a}")),
                _ => None,
            };
            return ForegroundApp {
                package: pkg,
                activity: act,
                component: comp,
            };
        }
    }
    ForegroundApp {
        package: None,
        activity: None,
        component: None,
    }
}

/// Parses visible windows from `dumpsys window windows`.
pub(crate) fn parse_window_windows(text: &str, visible_only: bool) -> Vec<Value> {
    static HDR: OnceLock<Option<Regex>> = OnceLock::new();
    static FRAME: OnceLock<Option<Regex>> = OnceLock::new();
    static TY: OnceLock<Option<Regex>> = OnceLock::new();
    static LAYER: OnceLock<Option<Regex>> = OnceLock::new();

    let hdr_re = HDR.get_or_init(|| {
        Regex::new(r"^\s*Window #\d+ Window\{[0-9a-fA-F]+\s+u\d+\s+([^}]+)\}:").ok()
    });
    let frame_re = FRAME.get_or_init(|| Regex::new(r"mFrame=\[(\d+),(\d+)\]\[(\d+),(\d+)\]").ok());
    let ty_re = TY.get_or_init(|| Regex::new(r"ty=(\S+)").ok());
    let layer_re = LAYER.get_or_init(|| Regex::new(r"mBaseLayer=(\d+)").ok());

    let mut out = Vec::new();
    let mut cur: Option<(String, String, bool, bool, Value, Value, Value)> = None;

    let flush = |slot: &mut Option<(String, String, bool, bool, Value, Value, Value)>,
                 dest: &mut Vec<Value>| {
        if let Some((name, pkg, vis, has_sf, frame, ty, layer)) = slot.take()
            && (!visible_only || vis)
        {
            dest.push(json!({
                "name": name, "package": pkg, "visible": vis, "hasSurface": has_sf,
                "frame": frame, "type": ty, "layer": layer,
            }));
        }
    };

    for line in text.lines() {
        if let Some(re) = hdr_re.as_ref()
            && let Some(c) = re.captures(line)
        {
            flush(&mut cur, &mut out);
            let name = c.get(1).map_or("", |m| m.as_str()).trim().to_owned();
            let pkg = name.split('/').next().unwrap_or(&name).to_owned();
            cur = Some((
                name,
                pkg,
                false,
                false,
                Value::Null,
                Value::Null,
                Value::Null,
            ));
            continue;
        }
        let Some((_, _, vis, has_sf, frame, ty, layer)) = cur.as_mut() else {
            continue;
        };
        if line.contains("mHasSurface=true") {
            *has_sf = true;
        }
        if line.contains("mViewVisibility=0x0") || line.contains("isOnScreen=true") {
            *vis = true;
        }
        if line.contains("isOnScreen=false") {
            *vis = false;
        }
        if frame.is_null()
            && let Some(re) = frame_re.as_ref()
            && let Some(c) = re.captures(line)
            && let (Ok(x0), Ok(y0), Ok(x1), Ok(y1)) = (
                c[1].parse::<i32>(),
                c[2].parse::<i32>(),
                c[3].parse::<i32>(),
                c[4].parse::<i32>(),
            )
        {
            *frame = json!({"x": x0, "y": y0, "w": x1 - x0, "h": y1 - y0});
        }
        if ty.is_null()
            && let Some(re) = ty_re.as_ref()
            && let Some(c) = re.captures(line)
        {
            *ty = json!(c[1].to_owned());
        }
        if layer.is_null()
            && let Some(re) = layer_re.as_ref()
            && let Some(c) = re.captures(line)
            && let Ok(val) = c[1].parse::<i32>()
        {
            *layer = json!(val);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// Parses frame timing summary metrics from `dumpsys gfxinfo <pkg>`.
pub(crate) fn parse_gfxinfo(text: &str) -> Value {
    let mut map = serde_json::Map::new();
    let specs: [(&str, &str, bool); 7] = [
        ("totalFrames", r"Total frames rendered:\s*(\d+)", false),
        ("jankyFrames", r"Janky frames:\s*(\d+)", false),
        (
            "jankyPercent",
            r"Janky frames:\s*\d+\s*\(([0-9.]+)%\)",
            true,
        ),
        ("p50Ms", r"50th percentile:\s*(\d+)ms", false),
        ("p90Ms", r"90th percentile:\s*(\d+)ms", false),
        ("p95Ms", r"95th percentile:\s*(\d+)ms", false),
        ("p99Ms", r"99th percentile:\s*(\d+)ms", false),
    ];
    for (key, pat, is_float) in specs {
        if let Ok(re) = Regex::new(pat)
            && let Some(c) = re.captures(text)
        {
            if is_float {
                if let Ok(v) = c[1].parse::<f64>() {
                    map.insert(key.to_owned(), json!(v));
                }
            } else if let Ok(v) = c[1].parse::<u64>() {
                map.insert(key.to_owned(), json!(v));
            }
        }
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_foreground_activity_gfxinfo_and_windows() {
        let act = "topResumedActivity=ActivityRecord{12ab u0 com.example.demo/.MainActivity t42}";
        let fg = parse_foreground_activity(act);
        assert_eq!(fg.package.as_deref(), Some("com.example.demo"));

        let gfx = "Total frames rendered: 120\nJanky frames: 6 (5.00%)\n50th percentile: 8ms\n99th percentile: 22ms\n";
        let g = parse_gfxinfo(gfx);
        assert_eq!(g["totalFrames"], json!(120));
        assert_eq!(g["jankyPercent"], json!(5.0));

        let dump = "  Window #0 Window{abcd u0 com.example.demo/com.example.demo.Main}:\n    ty=BASE_APPLICATION mBaseLayer=21000\n    mViewVisibility=0x0 mHasSurface=true isOnScreen=true\n    mFrame=[0,0][1080,2400]\n";
        let wins = parse_window_windows(dump, true);
        assert_eq!(wins.len(), 1);
        assert_eq!(wins[0]["frame"]["w"], json!(1080));
    }
}
