//! Pure `quick-xml` streaming parser for Android uiautomator hierarchy XML.

use quick_xml::Reader;
use quick_xml::events::Event;
use regex::Regex;
use serde_json::{Value, json};

use super::UiDumpOpts;

fn parse_bounds(raw: &str) -> (Option<[i32; 4]>, Option<[i32; 2]>) {
    let Ok(re) = Regex::new(r"\[(-?\d+),(-?\d+)\]\[(-?\d+),(-?\d+)\]") else {
        return (None, None);
    };
    let Some(c) = re.captures(raw) else {
        return (None, None);
    };
    let (Ok(x0), Ok(y0), Ok(x1), Ok(y1)) = (
        c[1].parse::<i32>(),
        c[2].parse::<i32>(),
        c[3].parse::<i32>(),
        c[4].parse::<i32>(),
    ) else {
        return (None, None);
    };
    (Some([x0, y0, x1, y1]), Some([(x0 + x1) / 2, (y0 + y1) / 2]))
}

/// Parses Android uiautomator XML into filtered JSON node descriptors.
pub(crate) fn parse_uiautomator_xml(xml_text: &str, opts: &UiDumpOpts) -> Value {
    let mut reader = Reader::from_str(xml_text);
    let q_low = opts.query.as_deref().map(str::to_lowercase);
    let rid_low = opts.resource_id.as_deref().map(str::to_lowercase);
    let desc_low = opts.content_desc.as_deref().map(str::to_lowercase);

    let mut total_nodes: usize = 0;
    let mut matched_count: usize = 0;
    let mut nodes: Vec<Value> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) if e.name().as_ref() == "node" => {
                total_nodes += 1;
                let (mut text, mut rid, mut desc, mut cls, mut pkg, mut bounds_raw) = (
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                );
                let (mut clickable, mut enabled, mut focused, mut selected, mut checked) =
                    (false, true, false, false, false);

                for attr in e.attributes().flatten() {
                    let val = attr
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map(std::borrow::Cow::into_owned)
                        .unwrap_or_default();
                    match attr.key.as_ref() {
                        "text" => text = val,
                        "resource-id" => rid = val,
                        "content-desc" => desc = val,
                        "class" => cls = val,
                        "package" => pkg = val,
                        "bounds" => bounds_raw = val,
                        "clickable" => clickable = val == "true",
                        "enabled" => enabled = val == "true",
                        "focused" => focused = val == "true",
                        "selected" => selected = val == "true",
                        "checked" => checked = val == "true",
                        _ => {}
                    }
                }

                if opts.clickable_only && !clickable {
                    continue;
                }
                if opts.compact
                    && text.is_empty()
                    && desc.is_empty()
                    && rid.is_empty()
                    && !clickable
                {
                    continue;
                }
                if let Some(q) = q_low.as_deref()
                    && !text.to_lowercase().contains(q)
                    && !desc.to_lowercase().contains(q)
                {
                    continue;
                }
                if let Some(r) = rid_low.as_deref()
                    && !rid.to_lowercase().contains(r)
                {
                    continue;
                }
                if let Some(d) = desc_low.as_deref()
                    && !desc.to_lowercase().contains(d)
                {
                    continue;
                }

                matched_count += 1;
                if opts.limit > 0 && nodes.len() >= opts.limit {
                    continue;
                }
                let (bounds, center) = parse_bounds(&bounds_raw);
                if opts.compact {
                    nodes.push(json!({
                        "text": text, "resourceId": rid, "contentDesc": desc,
                        "clickable": clickable, "center": center, "bounds": bounds,
                    }));
                } else {
                    nodes.push(json!({
                        "text": text, "resourceId": rid, "contentDesc": desc,
                        "class": cls, "package": pkg, "clickable": clickable,
                        "enabled": enabled, "focused": focused, "selected": selected,
                        "checked": checked, "bounds": bounds, "center": center,
                    }));
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                return json!({"ok": false, "error": format!("failed to parse uiautomator XML: {err}")});
            }
            _ => {}
        }
        buf.clear();
    }

    json!({
        "ok": true, "nodeCount": total_nodes, "matchedCount": matched_count,
        "truncated": matched_count > nodes.len(), "nodes": nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_uiautomator_xml_and_computes_center() {
        let xml = r#"<hierarchy rotation="0">
          <node text="" resource-id="" content-desc="" clickable="false" bounds="[0,0][1080,2400]">
            <node text="Confirm" resource-id="com.demo:id/btn_ok" content-desc="OK button" class="android.widget.Button" package="com.demo" clickable="true" enabled="true" bounds="[100,200][300,280]" />
            <node text="Cancel" resource-id="com.demo:id/btn_cancel" content-desc="" clickable="true" enabled="true" bounds="[400,200][600,280]" />
          </node>
        </hierarchy>"#;
        let out = parse_uiautomator_xml(
            xml,
            &UiDumpOpts {
                compressed: false,
                query: Some("confirm".to_owned()),
                resource_id: None,
                content_desc: None,
                clickable_only: true,
                compact: true,
                limit: 20,
            },
        );
        assert_eq!(out["ok"], json!(true));
        assert_eq!(out["matchedCount"], json!(1));
        assert_eq!(out["nodes"][0]["center"], json!([200, 240]));
    }
}
