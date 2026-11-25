//! Read-only LSPosed module database inspection (legacy & v2.2+ schemas) and log reader.

use std::collections::BTreeSet;
use std::time::Duration;

use regex::Regex;
use serde_json::{Value, json};

use super::android::{create_sqlite_snapshot, query_snapshot_json};
use super::parsers::validate_readonly_sql;
use crate::host::run_host;

const LSPOSED_DB_PATH: &str = "/data/adb/lspd/config/modules_config.db";

/// Extracts lowercased column names from `PRAGMA table_info(...)` JSON rows.
pub(crate) fn extract_pragma_columns(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .filter_map(|r| r.get("name").and_then(Value::as_str))
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Builds a schema-safe `SELECT` query for LSPosed modules across legacy (`mid` + `modules.enabled`)
/// and LSPosed v2.2+ (`module_pkg_name` PK + `modules_state` + `scope.module_pkg_name`) tables.
pub(crate) fn build_lsposed_modules_query(
    modules_cols: &BTreeSet<String>,
    state_cols: &BTreeSet<String>,
    scope_cols: &BTreeSet<String>,
) -> Result<String, String> {
    let pkg_col = if modules_cols.contains("module_pkg_name") {
        "module_pkg_name"
    } else if modules_cols.contains("pkg_name") {
        "pkg_name"
    } else {
        return Err("unsupported LSPosed modules table schema".to_owned());
    };
    let has_mid = modules_cols.contains("mid");

    let mut cols = Vec::with_capacity(5);
    if has_mid {
        cols.push("m.mid".to_owned());
    }
    cols.push(format!("m.{pkg_col} AS package"));

    let enabled_expr = if modules_cols.contains("enabled") {
        "m.enabled AS enabled".to_owned()
    } else if state_cols.contains("enabled") && state_cols.contains(pkg_col) {
        format!(
            "COALESCE((SELECT MAX(ms.enabled) FROM modules_state ms WHERE ms.{pkg_col} = m.{pkg_col}), 0) AS enabled"
        )
    } else if has_mid && state_cols.contains("enabled") && state_cols.contains("mid") {
        "COALESCE((SELECT MAX(ms.enabled) FROM modules_state ms WHERE ms.mid = m.mid), 0) AS enabled"
            .to_owned()
    } else {
        "NULL AS enabled".to_owned()
    };
    cols.push(enabled_expr);

    if modules_cols.contains("apk_path") {
        cols.push("m.apk_path AS apkPath".to_owned());
    } else {
        cols.push("NULL AS apkPath".to_owned());
    }

    let scopes_expr = if scope_cols.contains("app_pkg_name") && scope_cols.contains(pkg_col) {
        format!(
            "(SELECT GROUP_CONCAT(app_pkg_name) FROM (SELECT DISTINCT s.app_pkg_name FROM scope s WHERE s.{pkg_col} = m.{pkg_col} ORDER BY s.app_pkg_name)) AS scopes"
        )
    } else if has_mid && scope_cols.contains("app_pkg_name") && scope_cols.contains("mid") {
        "(SELECT GROUP_CONCAT(app_pkg_name) FROM (SELECT DISTINCT s.app_pkg_name FROM scope s WHERE s.mid = m.mid ORDER BY s.app_pkg_name)) AS scopes"
            .to_owned()
    } else {
        "NULL AS scopes".to_owned()
    };
    cols.push(scopes_expr);

    let order_by = if has_mid {
        "m.mid".to_owned()
    } else {
        format!("m.{pkg_col}")
    };
    validate_readonly_sql(&format!(
        "SELECT {} FROM modules m ORDER BY {order_by}",
        cols.join(", ")
    ))
}

async fn inspect_lsposed_modules_at(db_path: &str, module: Option<&str>) -> Value {
    let (tmp_dir, dst) = match create_sqlite_snapshot(db_path).await {
        Ok(pair) => pair,
        Err(err) => return json!({"ok": true, "available": false, "reason": err}),
    };

    let result = async {
        let mod_rows = query_snapshot_json(&dst, "PRAGMA table_info(modules)").await?;
        let state_rows = query_snapshot_json(&dst, "PRAGMA table_info(modules_state)")
            .await
            .unwrap_or_default();
        let scope_rows = query_snapshot_json(&dst, "PRAGMA table_info(scope)")
            .await
            .unwrap_or_default();
        let query = build_lsposed_modules_query(
            &extract_pragma_columns(&mod_rows),
            &extract_pragma_columns(&state_rows),
            &extract_pragma_columns(&scope_rows),
        )?;
        query_snapshot_json(&dst, &query).await
    }
    .await;
    let _ = std::fs::remove_dir_all(&tmp_dir);

    match result {
        Ok(rows) => {
            let filt = module.map(str::to_lowercase);
            let mods: Vec<Value> = rows
                .into_iter()
                .filter(|r| {
                    filt.as_deref().is_none_or(|f| {
                        r["package"]
                            .as_str()
                            .is_some_and(|p| p.to_lowercase().contains(f))
                    })
                })
                .take(200)
                .collect();
            json!({"ok": true, "available": true, "count": mods.len(), "modules": mods})
        }
        Err(err) => json!({"ok": true, "available": false, "reason": err}),
    }
}

/// Inspects LSPosed enabled modules (via read-only DB snapshot) or recent module logs.
pub(crate) async fn inspect_lsposed(
    mode: &str,
    module: Option<&str>,
    grep: Option<&str>,
    limit: usize,
) -> Value {
    if mode == "modules" {
        return inspect_lsposed_modules_at(LSPOSED_DB_PATH, module).await;
    }

    let ls = run_host(
        &["/system/bin/ls", "-1t", "/data/adb/lspd/log"],
        None,
        Duration::from_secs(6),
    )
    .await;
    let ls_text = ls.stdout_text();
    let chosen = ls_text
        .lines()
        .map(str::trim)
        .find(|e| e.starts_with("modules_") && e.ends_with(".log"))
        .or_else(|| ls_text.lines().map(str::trim).find(|e| e.ends_with(".log")))
        .map(str::to_owned);
    let Some(file) = chosen else {
        return json!({"ok": true, "available": false, "reason": "no LSPosed .log files found"});
    };
    let log_path = format!("/data/adb/lspd/log/{file}");
    let tail_n = (limit.saturating_mul(5).max(400)).to_string();
    let read = run_host(
        &["/system/bin/tail", "-n", &tail_n, &log_path],
        None,
        Duration::from_secs(8),
    )
    .await;
    let re = match grep {
        Some(g) if !g.is_empty() => match Regex::new(g) {
            Ok(r) => Some(r),
            Err(e) => return json!({"ok": false, "error": format!("invalid grep regex: {e}")}),
        },
        _ => None,
    };
    let mod_low = module.map(str::to_lowercase);
    let stdout = read.stdout_text();
    let mut lines: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            mod_low
                .as_deref()
                .is_none_or(|m| l.to_lowercase().contains(m))
        })
        .filter(|l| re.as_ref().is_none_or(|r| r.is_match(l)))
        .collect();
    if limit > 0 && lines.len() > limit {
        lines.drain(0..lines.len() - limit);
    }
    json!({"ok": true, "available": true, "logFile": log_path, "count": lines.len(), "lines": lines})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::run_local;

    fn set_of(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn builds_compatible_sql_for_legacy_and_v2_2_1_schemas() {
        let pragma = vec![
            json!({"cid": 0, "name": "module_pkg_name"}),
            json!({"cid": 1, "name": "apk_path"}),
        ];
        let v2_mod = extract_pragma_columns(&pragma);
        let v2_state = set_of(&[
            "module_pkg_name",
            "user_id",
            "enabled",
            "scope_request_blocked",
        ]);
        let v2_scope = set_of(&["module_pkg_name", "app_pkg_name", "user_id"]);
        let q_v2 = build_lsposed_modules_query(&v2_mod, &v2_state, &v2_scope).expect("v2 sql");
        assert!(!q_v2.contains("mid"));
        assert!(q_v2.contains("modules_state"));
        assert!(q_v2.contains("s.module_pkg_name = m.module_pkg_name"));

        let leg_mod = set_of(&["mid", "module_pkg_name", "apk_path", "enabled"]);
        let leg_scope = set_of(&["mid", "app_pkg_name", "user_id"]);
        let q_leg = build_lsposed_modules_query(&leg_mod, &BTreeSet::new(), &leg_scope)
            .expect("legacy sql");
        assert!(q_leg.contains("m.mid"));
        assert!(q_leg.contains("m.enabled AS enabled"));
        assert!(!q_leg.contains("modules_state"));
    }

    #[tokio::test]
    async fn queries_legacy_and_v2_2_1_sqlite_databases() {
        let dir = std::env::temp_dir().join(format!("strata-lspd-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let v2_db = dir.join("v2.db");
        let v2_init = "CREATE TABLE modules (module_pkg_name text PRIMARY KEY NOT NULL, apk_path text);
            CREATE TABLE modules_state (module_pkg_name text NOT NULL, user_id integer NOT NULL, enabled BOOLEAN DEFAULT 0, scope_request_blocked BOOLEAN DEFAULT 0, PRIMARY KEY (module_pkg_name, user_id));
            CREATE TABLE scope (module_pkg_name text NOT NULL, app_pkg_name text NOT NULL, user_id integer NOT NULL, PRIMARY KEY (module_pkg_name, app_pkg_name, user_id));
            INSERT INTO modules VALUES ('com.example.alpha', '/data/app/alpha/base.apk'), ('com.example.beta', '/data/app/beta/base.apk');
            INSERT INTO modules_state VALUES ('com.example.alpha', 0, 1, 0), ('com.example.beta', 0, 0, 0);
            INSERT INTO scope VALUES ('com.example.alpha', 'com.example.host', 0), ('com.example.alpha', 'android', 0), ('com.example.alpha', 'android', 999);";
        let init_out = run_local(
            "sqlite3",
            &[v2_db.to_str().expect("path"), v2_init],
            None,
            Duration::from_secs(5),
        )
        .await;
        if init_out.code == 127 {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert!(
            init_out.ok(),
            "sqlite3 v2 init failed: {}",
            init_out.stderr_text()
        );

        let res_v2 = inspect_lsposed_modules_at(v2_db.to_str().expect("path"), Some("alpha")).await;
        assert_eq!(res_v2["available"], json!(true));
        assert_eq!(res_v2["count"], json!(1));
        assert_eq!(res_v2["modules"][0]["package"], json!("com.example.alpha"));
        assert_eq!(res_v2["modules"][0]["enabled"], json!(1));
        assert_eq!(
            res_v2["modules"][0]["scopes"],
            json!("android,com.example.host")
        );

        let leg_db = dir.join("legacy.db");
        let leg_init = "CREATE TABLE modules (mid integer PRIMARY KEY, module_pkg_name text, apk_path text, enabled integer);
            CREATE TABLE scope (mid integer, app_pkg_name text, user_id integer);
            INSERT INTO modules VALUES (1, 'com.example.legacy', '/data/app/leg/base.apk', 1);
            INSERT INTO scope VALUES (1, 'android', 0);";
        let leg_out = run_local(
            "sqlite3",
            &[leg_db.to_str().expect("path"), leg_init],
            None,
            Duration::from_secs(5),
        )
        .await;
        assert!(leg_out.ok());
        let res_leg = inspect_lsposed_modules_at(leg_db.to_str().expect("path"), None).await;
        assert_eq!(res_leg["modules"][0]["mid"], json!(1));
        assert_eq!(res_leg["modules"][0]["enabled"], json!(1));
        assert_eq!(res_leg["modules"][0]["scopes"], json!("android"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
