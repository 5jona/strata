//! Parsing, validation, and normalization for `observe_step(actionGesture)` payloads.

use serde_json::{Map, Value, json};

fn parse_i32_tuple(raw: &str, expected: &[usize]) -> Result<Vec<i32>, String> {
    let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
    if !expected.contains(&parts.len()) {
        return Err(format!(
            "expected {} comma-separated integers, got {}",
            expected
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" or "),
            parts.len()
        ));
    }
    parts
        .iter()
        .map(|p| {
            p.parse::<i32>()
                .map_err(|_| format!("invalid integer '{p}' in gesture shorthand"))
        })
        .collect()
}

fn parse_shorthand(trimmed: &str) -> Result<Value, String> {
    if trimmed == "edge-swipe" {
        return Ok(json!({"kind": "edge-swipe"}));
    }
    if let Some(side) = trimmed
        .strip_prefix("edge-swipe:")
        .or_else(|| trimmed.strip_prefix("edge-swipe "))
    {
        let side = side.trim();
        if matches!(side, "left" | "right") {
            return Ok(json!({"kind": "edge-swipe", "side": side}));
        }
        return Err(format!("invalid edge-swipe side '{side}'"));
    }
    if let Some(rest) = trimmed.strip_prefix("tap:") {
        let nums = parse_i32_tuple(rest, &[2])?;
        return Ok(json!({"kind": "tap", "x": nums[0], "y": nums[1]}));
    }
    if let Some(rest) = trimmed.strip_prefix("long-press:") {
        let nums = parse_i32_tuple(rest, &[2, 3])?;
        let mut obj = json!({"kind": "long-press", "x": nums[0], "y": nums[1]});
        if let Some(&dur) = nums.get(2) {
            if dur < 0 {
                return Err("long-press durationMs must be non-negative".to_owned());
            }
            obj["durationMs"] = json!(dur);
        }
        return Ok(obj);
    }
    if let Some(rest) = trimmed.strip_prefix("swipe:") {
        let nums = parse_i32_tuple(rest, &[4, 5])?;
        let mut obj =
            json!({"kind": "swipe", "x0": nums[0], "y0": nums[1], "x1": nums[2], "y1": nums[3]});
        if let Some(&dur) = nums.get(4) {
            if dur < 0 {
                return Err("swipe durationMs must be non-negative".to_owned());
            }
            obj["durationMs"] = json!(dur);
        }
        return Ok(obj);
    }
    Err(format!(
        "invalid actionGesture string '{trimmed}': expected JSON object like {{\"kind\":\"swipe\",\"x0\":10,\"y0\":300,\"x1\":900,\"y1\":300}} or shorthand"
    ))
}

fn decode_to_object(raw: Value) -> Result<Map<String, Value>, String> {
    match raw {
        Value::Object(map) => Ok(map),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err("actionGesture must not be empty".to_owned());
            }
            match serde_json::from_str::<Value>(trimmed) {
                Ok(Value::Object(map)) => Ok(map),
                Ok(Value::String(inner)) if inner.trim_start().starts_with('{') => {
                    match serde_json::from_str::<Value>(inner.trim()) {
                        Ok(Value::Object(map)) => Ok(map),
                        _ => {
                            Err("actionGesture JSON string must decode to a JSON object".to_owned())
                        }
                    }
                }
                Ok(_) => Err("actionGesture JSON must be an object".to_owned()),
                Err(_) => match parse_shorthand(trimmed)? {
                    Value::Object(map) => Ok(map),
                    _ => Err("actionGesture shorthand did not produce an object".to_owned()),
                },
            }
        }
        other => Err(format!(
            "invalid actionGesture type ({other}): expected JSON object or JSON string"
        )),
    }
}

fn infer_or_read_kind(map: &mut Map<String, Value>) -> Result<String, String> {
    if let Some(val) = map.get("kind").filter(|v| !v.is_null()) {
        return val
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "actionGesture.kind must be a string".to_owned());
    }
    let has_swipe = ["x0", "y0", "x1", "y1"]
        .iter()
        .any(|k| map.contains_key(*k));
    let inferred = if has_swipe {
        "swipe"
    } else if map.contains_key("x") || map.contains_key("y") {
        if map.contains_key("durationMs") || map.contains_key("duration_ms") {
            "long-press"
        } else {
            "tap"
        }
    } else {
        "edge-swipe"
    };
    map.insert("kind".to_owned(), json!(inferred));
    Ok(inferred.to_owned())
}

/// Normalizes `observe_step(actionGesture)` from a JSON object, JSON string, or shorthand.
pub(crate) fn normalize_action_gesture(raw: Value) -> Result<String, String> {
    let mut map = decode_to_object(raw)?;
    let kind = infer_or_read_kind(&mut map)?;
    for dur_key in ["durationMs", "duration_ms"] {
        if let Some(v) = map.get(dur_key).filter(|v| !v.is_null())
            && v.as_u64().is_none()
        {
            return Err(format!("actionGesture.{dur_key} must be an integer >= 0"));
        }
    }
    match kind.as_str() {
        "tap" | "long-press" => {
            if map.get("x").and_then(Value::as_i64).is_none()
                || map.get("y").and_then(Value::as_i64).is_none()
            {
                return Err(format!(
                    "actionGesture kind='{kind}' requires integer x and y"
                ));
            }
        }
        "swipe" => {
            if map.get("x0").and_then(Value::as_i64).is_none()
                || map.get("y0").and_then(Value::as_i64).is_none()
                || map.get("x1").and_then(Value::as_i64).is_none()
                || map.get("y1").and_then(Value::as_i64).is_none()
            {
                return Err("actionGesture kind='swipe' requires integer x0, y0, x1, y1".to_owned());
            }
        }
        "edge-swipe" => {
            if let Some(side) = map.get("side").filter(|v| !v.is_null()) {
                let s = side
                    .as_str()
                    .ok_or_else(|| "actionGesture.side must be 'left' or 'right'".to_owned())?;
                if !matches!(s, "left" | "right") {
                    return Err(format!("invalid edge-swipe side '{s}'"));
                }
            }
        }
        other => {
            return Err(format!(
                "unsupported actionGesture kind '{other}': expected tap|long-press|swipe|edge-swipe"
            ));
        }
    }
    serde_json::to_string(&Value::Object(map))
        .map_err(|e| format!("serialize actionGesture failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_json_string_object_inferred_and_shorthand_gestures() {
        let from_str = normalize_action_gesture(Value::String(
            r#"{"kind":"swipe","x0":10,"y0":336,"x1":900,"y1":336,"durationMs":240}"#.to_owned(),
        ))
        .expect("json string swipe");
        let parsed: Value = serde_json::from_str(&from_str).expect("valid object JSON");
        assert!(parsed.is_object());
        assert_eq!(parsed["kind"], json!("swipe"));
        assert_eq!(parsed["x0"], json!(10));
        assert_eq!(parsed["x1"], json!(900));

        let double_encoded = normalize_action_gesture(Value::String(
            r#""{\"kind\":\"swipe\",\"x0\":10,\"y0\":336,\"x1\":900,\"y1\":336}""#.to_owned(),
        ))
        .expect("double encoded swipe");
        let parsed_de: Value = serde_json::from_str(&double_encoded).expect("valid JSON");
        assert_eq!(parsed_de["kind"], json!("swipe"));

        let inferred_swipe =
            normalize_action_gesture(json!({"x0": 10, "y0": 336, "x1": 900, "y1": 336}))
                .expect("inferred swipe");
        let parsed_inf: Value = serde_json::from_str(&inferred_swipe).expect("valid JSON");
        assert_eq!(parsed_inf["kind"], json!("swipe"));

        let from_obj =
            normalize_action_gesture(json!({"kind": "tap", "x": 120, "y": 240})).expect("obj tap");
        assert_eq!(
            serde_json::from_str::<Value>(&from_obj).expect("json")["kind"],
            json!("tap")
        );

        let lp = normalize_action_gesture(Value::String("long-press:100,200,800".to_owned()))
            .expect("shorthand long-press");
        let lp_v: Value = serde_json::from_str(&lp).expect("json");
        assert_eq!(lp_v["kind"], json!("long-press"));
        assert_eq!(lp_v["durationMs"], json!(800));

        let sw_short =
            normalize_action_gesture(Value::String("swipe:10,336,900,336,300".to_owned()))
                .expect("shorthand swipe");
        let sw_v: Value = serde_json::from_str(&sw_short).expect("json");
        assert_eq!(sw_v["kind"], json!("swipe"));
        assert_eq!(sw_v["x1"], json!(900));

        let legacy =
            normalize_action_gesture(Value::String("edge-swipe:left".to_owned())).expect("legacy");
        let parsed_leg: Value = serde_json::from_str(&legacy).expect("valid legacy JSON");
        assert_eq!(parsed_leg["kind"], json!("edge-swipe"));
        assert_eq!(parsed_leg["side"], json!("left"));

        assert!(normalize_action_gesture(json!({"kind": "pinch"})).is_err());
        assert!(normalize_action_gesture(json!({"kind": 123})).is_err());
        assert!(normalize_action_gesture(json!({"kind": "swipe", "x0": 1})).is_err());
        assert!(normalize_action_gesture(json!({"kind": "edge-swipe", "side": "up"})).is_err());
        assert!(normalize_action_gesture(Value::String("not-json".to_owned())).is_err());
        assert!(normalize_action_gesture(Value::String("   ".to_owned())).is_err());
    }
}
