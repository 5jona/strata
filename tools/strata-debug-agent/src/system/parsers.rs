//! Pure parsers and validators for L5 kernel diagnostics, SQLite read-only safety, KSU, and input devices.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Value, json};

fn watchdog_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(oplus_secure_guard|malicious_app|Kernel panic|BUG:|Unable to handle kernel|Call trace:|reboot,)").ok()
    })
    .as_ref()
}

/// Extracts OEM watchdog/panic hits and trailing lines from pstore ramoops text.
pub(crate) fn parse_pstore_text(text: &str, max_lines: usize) -> Value {
    let lines: Vec<&str> = text.lines().collect();
    let hits: Vec<String> = lines
        .iter()
        .filter(|ln| watchdog_re().is_some_and(|re| re.is_match(ln)))
        .map(|ln| ln.trim().to_owned())
        .collect();
    let tail: Vec<String> = lines
        .iter()
        .rev()
        .filter(|ln| !ln.trim().is_empty())
        .take(max_lines.min(20))
        .map(|ln| ln.trim().to_owned())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let hit_start = hits.len().saturating_sub(20);
    json!({
        "totalLines": lines.len(),
        "watchdogHits": &hits[hit_start..],
        "tailLines": tail,
    })
}

/// Validates that `sql` is a single read-only statement (`SELECT`, `PRAGMA`, `EXPLAIN`, or `WITH`).
pub(crate) fn validate_readonly_sql(sql: &str) -> Result<String, String> {
    let mut s = sql;
    loop {
        s = s.trim_start();
        if let Some(rest) = s.strip_prefix("--") {
            s = rest.find('\n').map_or("", |idx| &rest[idx + 1..]);
        } else if let Some(rest) = s.strip_prefix("/*") {
            let end = rest.find("*/").ok_or("unterminated block comment in SQL")?;
            s = &rest[end + 2..];
        } else {
            break;
        }
    }
    let trimmed = s.trim().trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return Err("empty SQL query".to_owned());
    }
    if trimmed.contains(';') {
        return Err("multi-statement SQL is not allowed".to_owned());
    }
    let first: String = trimmed
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect::<String>()
        .to_ascii_uppercase();
    if !matches!(first.as_str(), "SELECT" | "PRAGMA" | "EXPLAIN" | "WITH") {
        return Err(
            "only read-only SELECT, PRAGMA, EXPLAIN, or WITH queries are allowed".to_owned(),
        );
    }
    Ok(trimmed.to_owned())
}

/// Normalizes and validates a read-only `/sys/` or `/proc/` path, rejecting traversal.
pub(crate) fn validate_sysfs_path(path: &str) -> Result<String, String> {
    if !(path.starts_with("/sys/") || path.starts_with("/proc/")) {
        return Err("path must start with /sys/ or /proc/".to_owned());
    }
    let mut stack: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    let norm = format!("/{}", stack.join("/"));
    if !(norm.starts_with("/sys/") || norm.starts_with("/proc/")) {
        return Err("path traversal outside /sys/ or /proc/ is forbidden".to_owned());
    }
    Ok(norm)
}

/// Parses `getevent -lp` output into structured input device descriptors.
pub(crate) fn parse_getevent_devices(text: &str) -> Vec<Value> {
    let Ok(hdr_re) = Regex::new(r"^add device \d+:\s+(\S+)") else {
        return Vec::new();
    };
    let Ok(id_re) =
        Regex::new(r"bus:\s*(\S+)\s+vendor\s+(\S+)\s+product\s+(\S+)\s+version\s+(\S+)")
    else {
        return Vec::new();
    };
    let mut devices: Vec<Value> = Vec::new();
    let mut cur: Option<Value> = None;

    for raw in text.lines() {
        let stripped = raw.trim();
        if let Some(c) = hdr_re.captures(stripped) {
            if let Some(prev) = cur.take() {
                devices.push(prev);
            }
            cur = Some(json!({
                "device": c[1].to_owned(),
                "name": "",
                "bus": Value::Null,
                "vendor": Value::Null,
                "product": Value::Null,
                "version": Value::Null,
                "hasTouch": false,
                "hasKeys": false,
            }));
            continue;
        }
        let Some(dev) = cur.as_mut() else {
            continue;
        };
        if let Some(rest) = stripped.strip_prefix("name:") {
            dev["name"] = json!(rest.trim().trim_matches('"'));
        } else if let Some(c) = id_re.captures(stripped) {
            dev["bus"] = json!(c[1].to_owned());
            dev["vendor"] = json!(c[2].to_owned());
            dev["product"] = json!(c[3].to_owned());
            dev["version"] = json!(c[4].to_owned());
        }
        if stripped.contains("ABS_MT_POSITION_X")
            || stripped.contains("ABS_MT_TRACKING_ID")
            || stripped.contains("BTN_TOUCH")
        {
            dev["hasTouch"] = json!(true);
        }
        if stripped.contains("KEY_") {
            dev["hasKeys"] = json!(true);
        }
    }
    if let Some(prev) = cur {
        devices.push(prev);
    }
    devices
}

/// Parses KernelSU `module.prop` key=value lines.
pub(crate) fn parse_module_prop(text: &str) -> serde_json::Map<String, Value> {
    let mut props = serde_json::Map::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            if !key.is_empty() && !key.starts_with('#') {
                props.insert(key.to_owned(), json!(v.trim()));
            }
        }
    }
    props
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_readonly_sql_and_blocks_mutations() {
        assert_eq!(
            validate_readonly_sql("/* hdr */ SELECT * FROM t;").expect("valid"),
            "SELECT * FROM t"
        );
        assert!(validate_readonly_sql("DELETE FROM t").is_err());
        assert!(validate_readonly_sql("SELECT 1; DROP TABLE t").is_err());
    }

    #[test]
    fn validates_sysfs_paths_and_parses_pstore_watchdog() {
        assert_eq!(
            validate_sysfs_path("/sys/class/../fs/pstore").expect("ok"),
            "/sys/fs/pstore"
        );
        assert!(validate_sysfs_path("/sys/../etc/passwd").is_err());

        let p = parse_pstore_text(
            "[12.3] oplus_secure_guard_new: reboot,malicious_app\n[12.4] end\n",
            10,
        );
        assert_eq!(p["watchdogHits"].as_array().map(Vec::len), Some(1));
    }
}
