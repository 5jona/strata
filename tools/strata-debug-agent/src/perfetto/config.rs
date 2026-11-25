//! Bounded Perfetto textproto configuration presets and path validators.

use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const ALLOWED_PREFIX: &str = "/data/local/tmp/";

fn preset_sources(preset: &str) -> Result<&'static str, String> {
    match preset {
        "ui-jank" => Ok(
            "data_sources: {\n    config {\n        name: \"linux.ftrace\"\n        ftrace_config {\n            atrace_categories: \"gfx\"\n            atrace_categories: \"view\"\n            atrace_categories: \"wm\"\n            atrace_categories: \"am\"\n            atrace_categories: \"input\"\n            atrace_categories: \"sched\"\n            ftrace_events: \"sched/sched_switch\"\n            ftrace_events: \"sched/sched_waking\"\n        }\n    }\n}\ndata_sources: {\n    config {\n        name: \"android.surfaceflinger.frametimeline\"\n    }\n}",
        ),
        "sched" => Ok(
            "data_sources: {\n    config {\n        name: \"linux.ftrace\"\n        ftrace_config {\n            ftrace_events: \"sched/sched_switch\"\n            ftrace_events: \"sched/sched_wakeup\"\n            ftrace_events: \"sched/sched_waking\"\n            ftrace_events: \"power/cpu_frequency\"\n            ftrace_events: \"power/cpu_idle\"\n        }\n    }\n}\ndata_sources: {\n    config {\n        name: \"linux.process_stats\"\n        process_stats_config { scan_all_processes_on_start: true }\n    }\n}",
        ),
        "binder" => Ok(
            "data_sources: {\n    config {\n        name: \"linux.ftrace\"\n        ftrace_config {\n            atrace_categories: \"binder_driver\"\n            atrace_categories: \"binder_lock\"\n            ftrace_events: \"binder/binder_transaction\"\n            ftrace_events: \"binder/binder_transaction_received\"\n            ftrace_events: \"binder/binder_lock\"\n            ftrace_events: \"binder/binder_locked\"\n            ftrace_events: \"binder/binder_unlock\"\n        }\n    }\n}",
        ),
        "memory" => Ok(
            "data_sources: {\n    config {\n        name: \"linux.ftrace\"\n        ftrace_config {\n            atrace_categories: \"memory\"\n            ftrace_events: \"lowmemorykiller/lowmemory_kill\"\n            ftrace_events: \"oom/oom_score_adj_update\"\n            ftrace_events: \"kmem/rss_stat\"\n        }\n    }\n}\ndata_sources: {\n    config {\n        name: \"linux.sys_stats\"\n        sys_stats_config { meminfo_period_ms: 500 vmstat_period_ms: 500 }\n    }\n}",
        ),
        other => Err(format!(
            "unknown preset '{other}', expected ui-jank|sched|binder|memory"
        )),
    }
}

/// Builds a bounded Perfetto textproto configuration.
pub(crate) fn build_config(
    preset: &str,
    duration_sec: u32,
    buffer_kb: u32,
) -> Result<String, String> {
    let sources = preset_sources(preset)?;
    let buf = buffer_kb.clamp(1024, 32_768);
    let dur = duration_sec.clamp(1, 3600);
    Ok(format!(
        "buffers: {{\n    size_kb: {buf}\n    fill_policy: RING_BUFFER\n}}\n{sources}\nduration_ms: {}\n",
        u64::from(dur) * 1000
    ))
}

pub(crate) fn validate_session_key(key: &str) -> Result<&str, String> {
    if key.is_empty()
        || key.len() > 32
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err("session key must match ^[A-Za-z0-9_-]{1,32}$".to_owned());
    }
    Ok(key)
}

pub(crate) fn validate_out_path(out_path: Option<&str>, preset: &str) -> Result<String, String> {
    let Some(p) = out_path else {
        return Ok(format!(
            "{ALLOWED_PREFIX}strata-perfetto-{preset}-{}.pftrace",
            now_secs()
        ));
    };
    if !p.starts_with(ALLOWED_PREFIX) || p.contains("..") || p.ends_with('/') {
        return Err(format!(
            "out path must be a safe file under '{ALLOWED_PREFIX}'"
        ));
    }
    Ok(p.to_owned())
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_perfetto_config() {
        let cfg = build_config("ui-jank", 3, 8192).expect("cfg");
        assert!(cfg.contains("android.surfaceflinger.frametimeline"));
        assert!(cfg.contains("duration_ms: 3000"));
    }
}
