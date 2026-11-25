//! Pure threadtime logcat parser, duplicate-line folder, and crash/ANR highlighter.

use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

/// Parsed single `logcat -v threadtime` record.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct LogEntry {
    pub(crate) ts: String,
    pub(crate) pid: u32,
    pub(crate) tid: u32,
    pub(crate) priority: String,
    pub(crate) tag: String,
    pub(crate) msg: String,
    #[serde(skip_serializing_if = "is_one")]
    pub(crate) repeat: u32,
}

fn is_one(v: &u32) -> bool {
    *v <= 1
}

fn priority_rank(p: &str) -> u8 {
    match p.to_ascii_uppercase().as_str() {
        "V" => 0,
        "D" => 1,
        "I" => 2,
        "W" => 3,
        "E" => 4,
        "F" | "A" => 5,
        _ => 3,
    }
}

fn threadtime_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2}\.\d{3})\s+(\d+)\s+(\d+)\s+([VDIWEFA])\s+([^:]+?)\s*:\s?(.*)$").ok()
    })
    .as_ref()
}

fn crash_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(FATAL EXCEPTION|AndroidRuntime|SIGSEGV|SIGABRT|SIGBUS|ANR in |Build fingerprint:|backtrace:|tombstone)").ok()
    })
    .as_ref()
}

/// Parses one `threadtime`-formatted logcat line.
pub(crate) fn parse_threadtime_line(line: &str) -> Option<LogEntry> {
    let re = threadtime_re()?;
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let c = re.captures(trimmed)?;
    Some(LogEntry {
        ts: c[1].to_owned(),
        pid: c[2].parse().ok()?,
        tid: c[3].parse().ok()?,
        priority: c[4].to_owned(),
        tag: c[5].trim().to_owned(),
        msg: c[6].to_owned(),
        repeat: 1,
    })
}

pub(crate) fn fnv1a_hex(line: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in line.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Filters logcat lines after `last_digest`, folding consecutive identical `(tag, msg)` entries.
pub(crate) fn filter_lines(
    lines: &[&str],
    priority: &str,
    tag: Option<&str>,
    pid: Option<u32>,
    last_digest: &str,
    limit: usize,
) -> (Vec<LogEntry>, String, String) {
    let min_prio = priority_rank(priority);
    let tag_low = tag.map(str::to_lowercase);
    let mut entries: Vec<LogEntry> = Vec::new();
    let mut latest_ts = String::new();
    let mut latest_digest = last_digest.to_owned();
    let mut skipping = !last_digest.is_empty();

    for &raw in lines {
        let Some(parsed) = parse_threadtime_line(raw) else {
            continue;
        };
        let digest = fnv1a_hex(raw);
        latest_ts.clone_from(&parsed.ts);
        latest_digest.clone_from(&digest);
        if skipping {
            if digest == last_digest {
                skipping = false;
            }
            continue;
        }
        if priority_rank(&parsed.priority) < min_prio {
            continue;
        }
        if let Some(t) = tag_low.as_deref()
            && !parsed.tag.to_lowercase().contains(t)
        {
            continue;
        }
        if let Some(target_pid) = pid
            && parsed.pid != target_pid
        {
            continue;
        }
        if let Some(prev) = entries.last_mut()
            && prev.pid == parsed.pid
            && prev.priority == parsed.priority
            && prev.tag == parsed.tag
            && prev.msg == parsed.msg
        {
            prev.repeat = prev.repeat.saturating_add(1);
            prev.ts = parsed.ts;
            continue;
        }
        entries.push(parsed);
    }

    if skipping {
        return filter_lines(lines, priority, tag, pid, "", limit);
    }
    if limit > 0 && entries.len() > limit {
        entries.drain(0..entries.len() - limit);
    }
    (entries, latest_ts, latest_digest)
}

/// Extracts fatal/ANR/crash highlights from filtered entries.
pub(crate) fn extract_crashes(entries: &[LogEntry], limit: usize) -> Vec<LogEntry> {
    let mut hits: Vec<LogEntry> = entries
        .iter()
        .filter(|e| {
            matches!(e.priority.as_str(), "F" | "A")
                || crash_re().is_some_and(|re| re.is_match(&e.tag) || re.is_match(&e.msg))
        })
        .cloned()
        .collect();
    if limit > 0 && hits.len() > limit {
        hits.drain(0..hits.len() - limit);
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_duplicates_and_extracts_crashes() {
        let lines = [
            "11-25 10:00:00.100  1234  1235 W MyTag : repeated msg",
            "11-25 10:00:00.105  1234  1235 W MyTag : repeated msg",
            "11-25 10:00:00.200  1234  1235 E AndroidRuntime: FATAL EXCEPTION: main",
        ];
        let (entries, ts, _) = filter_lines(&lines, "W", None, None, "", 50);
        assert_eq!(ts, "11-25 10:00:00.200");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].repeat, 2);
        assert_eq!(extract_crashes(&entries, 10).len(), 1);
    }
}
