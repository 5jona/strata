//! Android host diagnostics: KernelSU modules/inputs/sysfs and read-only SQLite snapshots.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::parsers::{
    parse_getevent_devices, parse_module_prop, validate_readonly_sql, validate_sysfs_path,
};
use crate::host::{is_direct_host, run_host, run_local};

/// Inspects KernelSU modules, input devices (`getevent -lp`), or read-only `/sys`/`/proc` nodes.
pub(crate) async fn inspect_ksu(mode: &str, path: Option<&str>) -> Value {
    match mode {
        "modules" => {
            let ls = run_host(
                &["/system/bin/ls", "-1", "/data/adb/modules"],
                None,
                Duration::from_secs(6),
            )
            .await;
            let mut mods = Vec::new();
            for name in ls
                .stdout_text()
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let mdir = format!("/data/adb/modules/{name}");
                let prop_path = format!("{mdir}/module.prop");
                let prop_cmd = ["/system/bin/cat", &prop_path];
                let flag_cmd = [
                    "/system/bin/sh",
                    "-c",
                    r#"for f in disable remove update skip_mount; do test -e "$1/$f" && echo "$f"; done"#,
                    "_",
                    &mdir,
                ];
                let (prop_res, flag_res) = tokio::join!(
                    run_host(&prop_cmd, None, Duration::from_secs(4)),
                    run_host(&flag_cmd, None, Duration::from_secs(4)),
                );
                let props = parse_module_prop(&prop_res.stdout_text());
                let flags = flag_res.stdout_text();
                mods.push(json!({
                    "id": props.get("id").and_then(Value::as_str).unwrap_or(name),
                    "name": props.get("name").and_then(Value::as_str).unwrap_or(name),
                    "version": props.get("version").and_then(Value::as_str).unwrap_or(""),
                    "versionCode": props.get("versionCode").and_then(Value::as_str).unwrap_or(""),
                    "flags": {
                        "disable": flags.lines().any(|l| l == "disable"),
                        "remove": flags.lines().any(|l| l == "remove"),
                        "update": flags.lines().any(|l| l == "update"),
                        "skipMount": flags.lines().any(|l| l == "skip_mount"),
                    },
                }));
            }
            json!({"ok": true, "count": mods.len(), "modules": mods})
        }
        "inputs" => {
            let res = run_host(
                &["/system/bin/getevent", "-lp"],
                None,
                Duration::from_secs(10),
            )
            .await;
            if !res.ok() && res.stdout.is_empty() {
                return json!({"ok": false, "error": res.stderr_text()});
            }
            let devs = parse_getevent_devices(&res.stdout_text());
            json!({"ok": true, "count": devs.len(), "devices": devs})
        }
        "sysfs" => {
            let Some(raw) = path else {
                return json!({"ok": false, "error": "path is required for ksu sysfs"});
            };
            let norm = match validate_sysfs_path(raw) {
                Ok(n) => n,
                Err(e) => return json!({"ok": false, "error": e}),
            };
            let res = run_host(
                &[
                    "/system/bin/sh",
                    "-c",
                    r#"if test -d "$1"; then echo "__DIR__"; /system/bin/ls -1 "$1"; elif test -e "$1"; then echo "__FILE__"; /system/bin/head -c 16384 "$1"; else echo "__MISSING__"; fi"#,
                    "_",
                    &norm,
                ],
                None,
                Duration::from_secs(5),
            )
            .await;
            let text = res.stdout_text();
            if let Some(rest) = text.strip_prefix("__DIR__\n") {
                let entries: Vec<&str> = rest.lines().filter(|l| !l.is_empty()).collect();
                json!({"ok": true, "path": norm, "type": "dir", "entries": entries})
            } else if let Some(rest) = text.strip_prefix("__FILE__\n") {
                json!({"ok": true, "path": norm, "type": "file", "content": rest, "bytes": rest.len()})
            } else {
                json!({"ok": false, "path": norm, "error": "node not found"})
            }
        }
        other => json!({"ok": false, "error": format!("unsupported ksu mode: {other}")}),
    }
}

/// Copies a host or local SQLite DB (+ WAL/SHM) to a temp dir and runs container `sqlite3 -readonly -json`.
pub(crate) async fn sqlite_inspect(
    db_path: &str,
    mode: &str,
    sql: Option<&str>,
    limit: usize,
) -> Value {
    let query = if mode == "tables" {
        "SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') ORDER BY name"
            .to_owned()
    } else {
        let Some(raw_sql) = sql else {
            return json!({"ok": false, "error": "sql is required when mode is 'query'"});
        };
        match validate_readonly_sql(raw_sql) {
            Ok(q) => q,
            Err(e) => return json!({"ok": false, "error": e}),
        }
    };

    let (tmp_dir, dst) = match create_sqlite_snapshot(db_path).await {
        Ok(pair) => pair,
        Err(err) => return json!({"ok": false, "dbPath": db_path, "error": err}),
    };
    let rows_res = query_snapshot_json(&dst, &query).await;
    let _ = std::fs::remove_dir_all(&tmp_dir);

    let mut rows = match rows_res {
        Ok(r) => r,
        Err(err) => return json!({"ok": false, "dbPath": db_path, "error": err}),
    };
    let truncated = limit > 0 && rows.len() > limit;
    if truncated {
        rows.truncate(limit);
    }
    if mode == "tables" {
        json!({"ok": true, "dbPath": db_path, "tables": rows})
    } else {
        json!({"ok": true, "dbPath": db_path, "count": rows.len(), "truncated": truncated, "rows": rows})
    }
}

/// Creates a temporary directory containing a consistent `.db` (+ `-wal`/`-shm`) snapshot.
pub(super) async fn create_sqlite_snapshot(src: &str) -> Result<(PathBuf, PathBuf), String> {
    let tmp_dir =
        std::env::temp_dir().join(format!("strata-db-{}-{}", std::process::id(), now_nanos()));
    std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("create tmpdir failed: {e}"))?;
    let dst = tmp_dir.join("snapshot.db");
    if let Err(err) = snapshot_sqlite(src, &dst).await {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(err);
    }
    Ok((tmp_dir, dst))
}

const HOST_SQLITE3_CANDIDATES: [&str; 4] = [
    "/system/bin/sqlite3",
    "/system/xbin/sqlite3",
    "/vendor/bin/sqlite3",
    "/data/adb/ksu/bin/sqlite3",
];

fn select_sqlite3_binary<F: Fn(&str) -> bool>(
    direct_host: bool,
    exists: F,
) -> Option<&'static str> {
    if direct_host {
        HOST_SQLITE3_CANDIDATES
            .iter()
            .copied()
            .find(|cand| exists(cand))
    } else {
        if exists("/usr/bin/sqlite3") {
            return Some("/usr/bin/sqlite3");
        }
        HOST_SQLITE3_CANDIDATES
            .iter()
            .copied()
            .find(|cand| exists(cand))
            .or(Some("sqlite3"))
    }
}

/// Executes a read-only `sqlite3 -readonly -json` query on a local snapshot file.
pub(super) async fn query_snapshot_json(dst: &Path, query: &str) -> Result<Vec<Value>, String> {
    let direct_host = is_direct_host();
    let Some(sqlite_bin) = select_sqlite3_binary(direct_host, |p| Path::new(p).is_file()) else {
        return Err(format!(
            "sqlite3 binary not found on Android host (probed {}); start a Strata container or install sqlite3 for sqlite_inspect",
            HOST_SQLITE3_CANDIDATES.join(", ")
        ));
    };
    let dst_str = dst.to_string_lossy().into_owned();
    let res = run_local(
        sqlite_bin,
        &["-readonly", "-json", &dst_str, query],
        None,
        Duration::from_secs(10),
    )
    .await;
    if !res.ok() {
        let stderr = res.stderr_text().trim().to_owned();
        if direct_host
            && (stderr.contains("unknown option")
                || stderr.contains("unrecognized option")
                || res.code == 127)
        {
            return Err(format!(
                "host sqlite3 ({sqlite_bin}) does not support '-readonly -json' ({stderr}); start a Strata container for full sqlite_inspect support"
            ));
        }
        return Err(stderr);
    }
    let stdout = res.stdout_text();
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&stdout).map_err(|e| format!("invalid sqlite3 JSON output: {e}"))
}

async fn snapshot_sqlite(src: &str, dst: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let s = format!("{src}{suffix}");
        let d_owned = format!("{}{suffix}", dst.to_string_lossy());
        let d = Path::new(&d_owned);
        if Path::new(&s).is_file() {
            std::fs::copy(&s, d).map_err(|e| format!("copy {s} failed: {e}"))?;
        } else {
            let res = run_host(&["/system/bin/cat", &s], None, Duration::from_secs(15)).await;
            if res.ok() {
                std::fs::write(d, &res.stdout)
                    .map_err(|e| format!("write snapshot failed: {e}"))?;
            } else if suffix.is_empty() {
                return Err(format!("database not found or unreadable: {src}"));
            }
        }
    }
    Ok(())
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_sqlite3_binary_for_direct_host_and_container() {
        assert_eq!(
            select_sqlite3_binary(true, |p| p == "/system/bin/sqlite3"),
            Some("/system/bin/sqlite3")
        );
        assert_eq!(
            select_sqlite3_binary(true, |p| p == "/data/adb/ksu/bin/sqlite3"),
            Some("/data/adb/ksu/bin/sqlite3")
        );
        assert_eq!(select_sqlite3_binary(true, |_| false), None);
        assert_eq!(
            select_sqlite3_binary(false, |p| p == "/usr/bin/sqlite3"),
            Some("/usr/bin/sqlite3")
        );
        assert_eq!(select_sqlite3_binary(false, |_| false), Some("sqlite3"));
    }
}
