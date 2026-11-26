//! Sequential multi-step action parser and executor for `observe --actions`.

use std::time::Duration;

use serde_json::{Value, json};

use crate::gesture::{GestureKind, run_gesture};
use crate::host::{run_host, tail_chars};

const MAX_SLEEP_MS: u64 = 10_000;

/// Single step in an `observe_step(actions)` sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ObserveAction {
    Tap {
        x: i32,
        y: i32,
    },
    Swipe {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        duration_ms: u64,
    },
    Shell {
        command: String,
    },
    Sleep {
        ms: u64,
    },
}

impl ObserveAction {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Tap { .. } => "tap",
            Self::Swipe { .. } => "swipe",
            Self::Shell { .. } => "shell",
            Self::Sleep { .. } => "sleep",
        }
    }
}

fn req_i32(obj: &Value, key: &str, kind: &str) -> Result<i32, String> {
    obj.get(key)
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("action.{kind}.{key} must be an integer"))
}

pub(super) fn parse_actions_json(raw: &str) -> Result<Vec<ObserveAction>, String> {
    let val: Value =
        serde_json::from_str(raw.trim()).map_err(|e| format!("invalid --actions JSON: {e}"))?;
    let arr = val
        .as_array()
        .ok_or_else(|| "--actions must decode to a JSON array".to_owned())?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let kind = item
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("actions[{idx}].kind must be a string"))?;
        let parsed = match kind {
            "tap" => ObserveAction::Tap {
                x: req_i32(item, "x", "tap")?,
                y: req_i32(item, "y", "tap")?,
            },
            "swipe" => ObserveAction::Swipe {
                x0: req_i32(item, "x0", "swipe")?,
                y0: req_i32(item, "y0", "swipe")?,
                x1: req_i32(item, "x1", "swipe")?,
                y1: req_i32(item, "y1", "swipe")?,
                duration_ms: item
                    .get("durationMs")
                    .or_else(|| item.get("duration_ms"))
                    .and_then(Value::as_u64)
                    .unwrap_or(240),
            },
            "shell" => {
                let command = item
                    .get("command")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("actions[{idx}].command must be a non-empty string"))?;
                ObserveAction::Shell {
                    command: command.to_owned(),
                }
            }
            "sleep" => {
                let ms = item
                    .get("ms")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| format!("actions[{idx}].ms must be a non-negative integer"))?;
                ObserveAction::Sleep { ms }
            }
            other => {
                return Err(format!(
                    "actions[{idx}] unsupported kind '{other}' (expected tap|swipe|shell|sleep)"
                ));
            }
        };
        out.push(parsed);
    }
    Ok(out)
}

async fn run_single_step(action: &ObserveAction) -> Result<Value, String> {
    match action {
        ObserveAction::Tap { x, y } => {
            let res = run_gesture(GestureKind::Tap { x: *x, y: *y }).await;
            if res["ok"] == json!(true) {
                Ok(json!({"kind": "tap", "result": res}))
            } else {
                Err(res["error"]
                    .as_str()
                    .unwrap_or("tap gesture failed")
                    .to_owned())
            }
        }
        ObserveAction::Swipe {
            x0,
            y0,
            x1,
            y1,
            duration_ms,
        } => {
            let res = run_gesture(GestureKind::Swipe {
                x0: *x0,
                y0: *y0,
                x1: *x1,
                y1: *y1,
                duration_ms: *duration_ms,
            })
            .await;
            if res["ok"] == json!(true) {
                Ok(json!({"kind": "swipe", "result": res}))
            } else {
                Err(res["error"]
                    .as_str()
                    .unwrap_or("swipe gesture failed")
                    .to_owned())
            }
        }
        ObserveAction::Shell { command } => {
            let res = run_host(
                &["/system/bin/sh", "-c", command],
                None,
                Duration::from_secs(15),
            )
            .await;
            if res.ok() {
                Ok(json!({
                    "kind": "shell",
                    "rc": res.code,
                    "stdout": tail_chars(res.stdout_text().trim(), 600),
                    "stderr": tail_chars(res.stderr_text().trim(), 300),
                }))
            } else {
                Err(format!(
                    "shell exited rc={}: {}",
                    res.code,
                    tail_chars(res.stderr_text().trim(), 200)
                ))
            }
        }
        ObserveAction::Sleep { ms } => {
            let bounded = (*ms).min(MAX_SLEEP_MS);
            if bounded > 0 {
                tokio::time::sleep(Duration::from_millis(bounded)).await;
            }
            Ok(json!({"kind": "sleep", "ms": bounded}))
        }
    }
}

/// Executes `actions` sequentially, stopping on first failure with completed step count.
pub(super) async fn execute_action_sequence(actions: &[ObserveAction]) -> (Value, Option<String>) {
    let total = actions.len();
    let mut results = Vec::with_capacity(total);
    for (idx, action) in actions.iter().enumerate() {
        match run_single_step(action).await {
            Ok(step_val) => results.push(step_val),
            Err(err) => {
                let msg = format!("action[{idx}] ({}) failed: {err}", action.kind_name());
                return (
                    json!({
                        "type": "actions",
                        "completed": idx,
                        "total": total,
                        "failedStep": idx,
                        "results": results,
                    }),
                    Some(msg),
                );
            }
        }
    }
    (
        json!({
            "type": "actions",
            "completed": total,
            "total": total,
            "results": results,
        }),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_multi_step_action_sequence() {
        let raw = r#"[
            {"kind":"tap","x":120,"y":240},
            {"kind":"sleep","ms":150},
            {"kind":"swipe","x0":10,"y0":500,"x1":900,"y1":500,"durationMs":300},
            {"kind":"shell","command":"echo ok"}
        ]"#;
        let parsed = parse_actions_json(raw).expect("valid actions");
        assert_eq!(
            parsed,
            vec![
                ObserveAction::Tap { x: 120, y: 240 },
                ObserveAction::Sleep { ms: 150 },
                ObserveAction::Swipe {
                    x0: 10,
                    y0: 500,
                    x1: 900,
                    y1: 500,
                    duration_ms: 300
                },
                ObserveAction::Shell {
                    command: "echo ok".to_owned()
                },
            ]
        );
    }

    #[test]
    fn rejects_invalid_action_steps() {
        assert!(parse_actions_json(r#"{"kind":"tap"}"#).is_err());
        assert!(parse_actions_json(r#"[{"kind":"pinch"}]"#).is_err());
        assert!(parse_actions_json(r#"[{"kind":"tap","x":1}]"#).is_err());
        assert!(parse_actions_json(r#"[{"kind":"shell","command":"  "}]"#).is_err());
        assert!(parse_actions_json(r#"[{"kind":"sleep","ms":-5}]"#).is_err());
    }
}
