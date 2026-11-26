//! Gesture specification parser for `observe --action-gesture`.

use serde_json::{Value, json};

use crate::gesture::GestureKind;

pub(super) fn parse_gesture_spec(raw: &Value) -> Result<GestureKind, String> {
    let owned_obj;
    let spec = match raw {
        Value::Object(_) => raw,
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed == "edge-swipe" {
                owned_obj = json!({"kind": "edge-swipe"});
            } else if let Some(side) = trimmed.strip_prefix("edge-swipe:")
                && matches!(side.trim(), "left" | "right")
            {
                owned_obj = json!({"kind": "edge-swipe", "side": side.trim()});
            } else {
                let first = serde_json::from_str::<Value>(trimmed)
                    .map_err(|e| format!("invalid actionGesture JSON string: {e}"))?;
                owned_obj = match first {
                    Value::String(inner) => serde_json::from_str::<Value>(inner.trim())
                        .map_err(|e| format!("invalid nested actionGesture JSON: {e}"))?,
                    other => other,
                };
                if !owned_obj.is_object() {
                    return Err("actionGesture must decode to a JSON object".to_owned());
                }
            }
            &owned_obj
        }
        _ => return Err("actionGesture must be a JSON object".to_owned()),
    };

    let duration = spec["durationMs"]
        .as_u64()
        .or_else(|| spec["duration_ms"].as_u64());
    let kind = match spec.get("kind") {
        Some(Value::String(s)) => s.as_str(),
        Some(v) if !v.is_null() => return Err("gesture.kind must be a string".to_owned()),
        _ if ["x0", "y0", "x1", "y1"].iter().any(|k| !spec[*k].is_null()) => "swipe",
        _ if !spec["x"].is_null() || !spec["y"].is_null() => "tap",
        _ => "edge-swipe",
    };

    match kind {
        "tap" => Ok(GestureKind::Tap {
            x: spec["x"].as_i64().ok_or("gesture.x required")? as i32,
            y: spec["y"].as_i64().ok_or("gesture.y required")? as i32,
        }),
        "long-press" => Ok(GestureKind::LongPress {
            x: spec["x"].as_i64().ok_or("gesture.x required")? as i32,
            y: spec["y"].as_i64().ok_or("gesture.y required")? as i32,
            duration_ms: duration.unwrap_or(600),
        }),
        "swipe" => Ok(GestureKind::Swipe {
            x0: spec["x0"].as_i64().ok_or("gesture.x0 required")? as i32,
            y0: spec["y0"].as_i64().ok_or("gesture.y0 required")? as i32,
            x1: spec["x1"].as_i64().ok_or("gesture.x1 required")? as i32,
            y1: spec["y1"].as_i64().ok_or("gesture.y1 required")? as i32,
            duration_ms: duration.unwrap_or(240),
        }),
        "edge-swipe" => Ok(GestureKind::EdgeSwipe {
            side: spec["side"].as_str().unwrap_or("right").to_owned(),
            y_percent: spec["yPercent"]
                .as_f64()
                .or_else(|| spec["y_percent"].as_f64())
                .unwrap_or(50.0),
            distance_dp: spec["distanceDp"]
                .as_f64()
                .or_else(|| spec["distance_dp"].as_f64())
                .unwrap_or(96.0),
            duration_ms: duration.unwrap_or(240),
            cancel_back: spec["cancelBack"]
                .as_bool()
                .or_else(|| spec["cancel_back"].as_bool())
                .unwrap_or(false),
        }),
        other => Err(format!(
            "unsupported gesture kind '{other}' (expected tap|long-press|swipe|edge-swipe)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_gesture_spec_variants() {
        let tap = parse_gesture_spec(&json!({"kind": "tap", "x": 10, "y": 20})).expect("tap");
        assert!(matches!(tap, GestureKind::Tap { x: 10, y: 20 }));

        let edge =
            parse_gesture_spec(&json!({"kind": "edge-swipe", "side": "left", "cancelBack": true}))
                .expect("edge");
        assert!(matches!(
            edge,
            GestureKind::EdgeSwipe {
                cancel_back: true,
                ..
            }
        ));

        let swipe_str = parse_gesture_spec(&Value::String(
            r#"{"kind":"swipe","x0":10,"y0":336,"x1":900,"y1":336,"duration_ms":300}"#.to_owned(),
        ))
        .expect("string-encoded swipe");
        assert!(matches!(
            swipe_str,
            GestureKind::Swipe {
                x0: 10,
                y0: 336,
                x1: 900,
                y1: 336,
                duration_ms: 300
            }
        ));

        let double_enc = parse_gesture_spec(&Value::String(
            r#""{\"kind\":\"swipe\",\"x0\":1,\"y0\":2,\"x1\":3,\"y1\":4}""#.to_owned(),
        ))
        .expect("double-encoded swipe");
        assert!(matches!(
            double_enc,
            GestureKind::Swipe { x0: 1, x1: 3, .. }
        ));

        let shorthand = parse_gesture_spec(&Value::String("edge-swipe:left".to_owned()))
            .expect("shorthand edge-swipe");
        assert!(matches!(shorthand, GestureKind::EdgeSwipe { ref side, .. } if side == "left"));

        assert!(parse_gesture_spec(&json!({"kind": "unknown"})).is_err());
        assert!(parse_gesture_spec(&json!({"kind": 42})).is_err());
    }
}
