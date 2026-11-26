//! Cached `adb forward` mapping table and POSIX/Android host-root shell quoting helpers.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::process::Command;

pub(crate) const HOST_DEBUG_BIN: &str = "/data/adb/modules/strata/bin/strata-debug";

fn forward_cache() -> &'static Mutex<HashMap<(String, u16), u16>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, u16), u16>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_forward_cached(serial: &str, local_port: u16, remote_port: u16) -> bool {
    forward_cache()
        .lock()
        .ok()
        .is_some_and(|m| m.get(&(serial.to_owned(), local_port)) == Some(&remote_port))
}

fn record_forward_cached(serial: &str, local_port: u16, remote_port: u16) {
    if let Ok(mut m) = forward_cache().lock() {
        m.insert((serial.to_owned(), local_port), remote_port);
    }
}

/// Invalidates a cached `adb forward` mapping so the next call re-establishes it.
pub(crate) fn invalidate_adb_forward(serial: &str, local_port: u16) {
    if let Ok(mut m) = forward_cache().lock() {
        m.remove(&(serial.to_owned(), local_port));
    }
}

/// Ensures `adb -s <serial> forward tcp:<local> tcp:<remote>` is active (cached per mapping).
pub(crate) async fn ensure_adb_forward(
    serial: &str,
    local_port: u16,
    remote_port: u16,
) -> Result<(), String> {
    if is_forward_cached(serial, local_port, remote_port) {
        return Ok(());
    }
    let out = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("adb")
            .args([
                "-s",
                serial,
                "forward",
                &format!("tcp:{local_port}"),
                &format!("tcp:{remote_port}"),
            ])
            .output(),
    )
    .await
    .map_err(|_| format!("adb forward timed out for {serial}"))?
    .map_err(|e| format!("failed to invoke adb: {e}"))?;

    if !out.status.success() {
        invalidate_adb_forward(serial, local_port);
        return Err(format!(
            "adb forward failed for {serial}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    record_forward_cached(serial, local_port, remote_port);
    Ok(())
}

/// POSIX single-quote escaping preventing shell injection when composing remote commands.
pub(crate) fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':' | b'=' | b',')
        })
    {
        return arg.to_owned();
    }
    format!("'{}'", arg.replace('\'', "'\"'\"'"))
}

/// Safely formats an argument vector into a POSIX shell command line.
pub(crate) fn argv_to_shell(argv: &[String]) -> String {
    argv.iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Translates an argv slice into a host-root command string for `adb shell su -c ...`.
pub(crate) fn adb_host_shell_command(argv: &[String]) -> String {
    if argv.is_empty() {
        return "/system/bin/true".to_owned();
    }
    let translated: Vec<String> = if argv[0] == "strata-debug" {
        std::iter::once(HOST_DEBUG_BIN.to_owned())
            .chain(argv[1..].iter().cloned())
            .collect()
    } else if argv[0] == "host-exec" && argv.len() > 1 {
        argv[1..].to_vec()
    } else {
        argv.to_vec()
    };
    argv_to_shell(&translated)
}

/// Builds the `su -c <quoted>` payload passed to `adb -s <serial> shell`.
pub(crate) fn wrap_su_command(host_cmd: &str) -> String {
    format!("su -c {}", shell_quote(host_cmd))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_shell_arguments_against_injection() {
        let cmd = argv_to_shell(&[
            "strata-debug".to_owned(),
            "cdp".to_owned(),
            "eval".to_owned(),
            "document.title; $(rm -rf /); 'x'".to_owned(),
        ]);
        assert!(cmd.contains("'document.title; $(rm -rf /); '\"'\"'x'\"'\"''"));
    }

    #[test]
    fn translates_adb_host_commands_and_caches_adb_forward_mappings() {
        let cmd = adb_host_shell_command(&[
            "strata-debug".to_owned(),
            "health".to_owned(),
            "--package".to_owned(),
            "auto".to_owned(),
        ]);
        assert!(cmd.starts_with(HOST_DEBUG_BIN));
        assert!(cmd.contains("--package auto"));

        let stripped = adb_host_shell_command(&[
            "host-exec".to_owned(),
            "/system/bin/rm".to_owned(),
            "-f".to_owned(),
            "/data/local/tmp/a".to_owned(),
        ]);
        assert_eq!(stripped, "/system/bin/rm -f /data/local/tmp/a");

        let ser = "SYNTHETIC_SER_CACHE_TEST";
        let lp = 12291;
        invalidate_adb_forward(ser, lp);
        assert!(!is_forward_cached(ser, lp, 2222));
        record_forward_cached(ser, lp, 2222);
        assert!(is_forward_cached(ser, lp, 2222));
        assert!(!is_forward_cached(ser, lp, 2225));
        invalidate_adb_forward(ser, lp);
        assert!(!is_forward_cached(ser, lp, 2222));
    }
}
