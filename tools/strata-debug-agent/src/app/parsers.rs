//! Pure parsers for Android `dumpsys package` and `dumpsys meminfo`.

use regex::Regex;
use serde_json::{Value, json};

/// Parses version, SDK levels, UID, and APK paths from `dumpsys package`.
pub(crate) fn parse_dumpsys_package(text: &str) -> Value {
    let mut map = serde_json::Map::new();
    for (k, pat) in [
        ("versionName", r"versionName=(\S+)"),
        ("codePath", r"codePath=(\S+)"),
    ] {
        if let Ok(re) = Regex::new(pat)
            && let Some(c) = re.captures(text)
        {
            map.insert(k.to_owned(), json!(c[1].to_owned()));
        }
    }
    for (k, pat) in [
        ("versionCode", r"versionCode=(\d+)"),
        ("targetSdk", r"targetSdk=(\d+)"),
        ("minSdk", r"minSdk=(\d+)"),
        ("uid", r"(?:userId|appId)=(\d+)"),
    ] {
        if let Ok(re) = Regex::new(pat)
            && let Some(c) = re.captures(text)
            && let Ok(v) = c[1].parse::<u64>()
        {
            map.insert(k.to_owned(), json!(v));
        }
    }
    if let Some(uid) = map.get("uid").cloned() {
        map.insert("userId".to_owned(), uid);
    }
    let mut apks: Vec<String> = Vec::new();
    for pat in [
        r"codePath=(\S+)",
        r"resourcePath=(\S+)",
        r"path:\s*(\S+\.apk)",
    ] {
        if let Ok(re) = Regex::new(pat) {
            for c in re.captures_iter(text) {
                let s = c[1].to_owned();
                if !apks.contains(&s) {
                    apks.push(s);
                }
            }
        }
    }
    map.insert("apkPaths".to_owned(), json!(apks));
    Value::Object(map)
}

/// Parses PSS, Java heap, and Native heap (KB) from `dumpsys meminfo`.
pub(crate) fn parse_meminfo_summary(text: &str) -> Value {
    let mut map = serde_json::Map::new();
    let total = Regex::new(r"TOTAL PSS:\s*(\d+)")
        .ok()
        .and_then(|re| re.captures(text))
        .or_else(|| {
            Regex::new(r"(?m)^\s*TOTAL\s+(\d+)")
                .ok()
                .and_then(|re| re.captures(text))
        });
    if let Some(c) = total
        && let Ok(v) = c[1].parse::<u64>()
    {
        map.insert("totalPssKb".to_owned(), json!(v));
    }
    for (k, pat) in [
        ("javaHeapKb", r"Java Heap:\s*(\d+)"),
        ("nativeHeapKb", r"Native Heap:\s*(\d+)"),
    ] {
        if let Ok(re) = Regex::new(pat)
            && let Some(c) = re.captures(text)
            && let Ok(v) = c[1].parse::<u64>()
        {
            map.insert(k.to_owned(), json!(v));
        }
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_package_and_meminfo() {
        let pkg = "versionCode=120 minSdk=29 targetSdk=36\nversionName=1.2.0\nuserId=10234\ncodePath=/data/app/demo\n";
        let p = parse_dumpsys_package(pkg);
        assert_eq!(p["versionCode"], json!(120));
        assert_eq!(p["uid"], json!(10234));

        let mem = "  Java Heap:   12000\n  Native Heap:   8500\n  TOTAL PSS:   45000\n";
        let m = parse_meminfo_summary(mem);
        assert_eq!(m["totalPssKb"], json!(45000));
        assert_eq!(m["javaHeapKb"], json!(12000));
    }
}
