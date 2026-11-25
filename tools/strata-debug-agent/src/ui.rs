//! UI Automator XML dumper and element center tapper.

mod parser;

use std::time::Duration;

use serde_json::{Value, json};

use crate::gesture::{GestureKind, run_gesture};
use crate::host::run_host;
use parser::parse_uiautomator_xml;

/// Filter parameters for UI hierarchy dumps.
#[derive(Debug, Clone)]
pub(crate) struct UiDumpOpts {
    pub(crate) compressed: bool,
    pub(crate) query: Option<String>,
    pub(crate) resource_id: Option<String>,
    pub(crate) content_desc: Option<String>,
    pub(crate) clickable_only: bool,
    pub(crate) compact: bool,
    pub(crate) limit: usize,
}

/// Dumps current Android UI hierarchy via `uiautomator dump` and parses XML.
pub(crate) async fn dump_ui_tree(opts: &UiDumpOpts) -> Value {
    let tmp_xml = format!("/data/local/tmp/strata_ui_{}.xml", std::process::id());
    let mut dump_args: Vec<&str> = vec!["/system/bin/uiautomator", "dump"];
    if opts.compressed {
        dump_args.push("--compressed");
    }
    dump_args.push(&tmp_xml);

    let mut dumped = false;
    let mut last_err = String::new();
    for attempt in 0..2 {
        let res = run_host(&dump_args, None, Duration::from_secs(15)).await;
        let combined = format!("{}\n{}", res.stdout_text(), res.stderr_text());
        if res.ok() && !combined.contains("ERROR:") {
            dumped = true;
            break;
        }
        last_err = combined.trim().chars().take(300).collect();
        if attempt == 0 {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    if !dumped {
        let _ = run_host(
            &["/system/bin/rm", "-f", &tmp_xml],
            None,
            Duration::from_secs(5),
        )
        .await;
        return json!({
            "ok": false,
            "error": last_err,
            "hint": "Ensure the screen is unlocked and idle; transient animations can block uiautomator.",
        });
    }

    let read_res = run_host(
        &["/system/bin/cat", &tmp_xml],
        None,
        Duration::from_secs(10),
    )
    .await;
    let _ = run_host(
        &["/system/bin/rm", "-f", &tmp_xml],
        None,
        Duration::from_secs(5),
    )
    .await;
    if !read_res.ok() || read_res.stdout.is_empty() {
        return json!({"ok": false, "error": "empty uiautomator XML output"});
    }
    parse_uiautomator_xml(&read_res.stdout_text(), opts)
}

/// Locates a UI node by selector and taps its center via `input tap` or `/dev/uinput`.
pub(crate) async fn tap_ui_node(
    text: Option<String>,
    resource_id: Option<String>,
    content_desc: Option<String>,
    index: usize,
    method: &str,
) -> Value {
    if text.is_none() && resource_id.is_none() && content_desc.is_none() {
        return json!({"ok": false, "error": "at least one selector (text, resourceId, or desc) is required"});
    }
    if method != "input" && method != "uinput" {
        return json!({"ok": false, "error": format!("unsupported tap method: {method}")});
    }
    let tree = dump_ui_tree(&UiDumpOpts {
        compressed: false,
        query: text,
        resource_id,
        content_desc,
        clickable_only: false,
        compact: false,
        limit: (index + 1).max(80),
    })
    .await;
    if tree["ok"] != json!(true) {
        return tree;
    }
    let Some(node) = tree["nodes"]
        .as_array()
        .and_then(|arr| arr.get(index))
        .cloned()
    else {
        return json!({
            "ok": false,
            "error": format!("node index {index} out of range (matched {})", tree["matchedCount"]),
            "matchedCount": tree["matchedCount"],
        });
    };
    let Some(center) = node["center"].as_array() else {
        return json!({"ok": false, "error": "selected node has no valid bounds", "node": node});
    };
    let (Some(cx), Some(cy)) = (
        center.first().and_then(Value::as_i64),
        center.get(1).and_then(Value::as_i64),
    ) else {
        return json!({"ok": false, "error": "invalid node center coordinates", "node": node});
    };

    if method == "input" {
        let sx = cx.to_string();
        let sy = cy.to_string();
        let res = run_host(
            &["/system/bin/input", "tap", &sx, &sy],
            None,
            Duration::from_secs(8),
        )
        .await;
        if !res.ok() {
            return json!({"ok": false, "method": method, "center": [cx, cy], "error": res.stderr_text()});
        }
    } else {
        let g = run_gesture(GestureKind::Tap {
            x: cx as i32,
            y: cy as i32,
        })
        .await;
        if g["ok"] != json!(true) {
            return g;
        }
    }
    json!({"ok": true, "method": method, "center": [cx, cy], "node": node})
}
