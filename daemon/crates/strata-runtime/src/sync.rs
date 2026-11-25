//! Synchronization of container DNS, Android `host.env`, bridge scripts, SSH keys, and log tails.

mod debug_tools;
pub mod layout;
mod ssh_keys;

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use strata_protocol::ContainerConfig;
use strata_sys::{net, procfs, rootfs_io::RootDir};

use crate::error::{RuntimeError, RuntimeResult};
use debug_tools::sync_strata_debug_binary;
pub use ssh_keys::{add_ssh_keys, list_ssh_keys, remove_ssh_keys};

const MAX_LOG_TAIL_READ_BYTES: u64 = 256 * 1024;

/// Synchronizes `/etc/resolv.conf`, `/run/strata/host.env`, `host-exec`, and the static musl
/// `strata-debug` binary inside a mounted container rootfs without following symlinks onto the host.
pub fn sync_container_runtime_files(
    mount_dir: &Path,
    cfg: &ContainerConfig,
    module_dir: Option<&Path>,
) -> RuntimeResult<()> {
    let root = RootDir::open(mount_dir).map_err(RuntimeError::from_sys)?;
    root.ensure_dir(Path::new("etc"), 0o755)
        .map_err(RuntimeError::from_sys)?;
    root.ensure_dir(Path::new("run/strata"), 0o755)
        .map_err(RuntimeError::from_sys)?;
    root.ensure_dir(Path::new("run/sshd"), 0o755)
        .map_err(RuntimeError::from_sys)?;
    root.ensure_dir(Path::new("usr/local/bin"), 0o755)
        .map_err(RuntimeError::from_sys)?;

    let android_dns = if cfg.sync_android_dns {
        net::read_android_dns_servers()
    } else {
        Vec::new()
    };
    let resolv = render_resolv_conf(&android_dns, &cfg.dns_servers);
    root.sync_bytes_if_changed(resolv.as_bytes(), Path::new("etc/resolv.conf"), 0o644)
        .map_err(RuntimeError::from_sys)?;

    let classpath = procfs::read_android_classpath_env();
    let host_env = render_host_env(&classpath);
    root.sync_bytes_if_changed(host_env.as_bytes(), Path::new("run/strata/host.env"), 0o644)
        .map_err(RuntimeError::from_sys)?;

    layout::sync_container_layout(&root)?;

    if let Some(mod_root) = module_dir {
        let host_exec_src = mod_root.join("container/host-exec");
        if let Ok(bytes) = fs::read(&host_exec_src) {
            root.sync_bytes_if_changed(&bytes, Path::new("usr/local/bin/host-exec"), 0o755)
                .map_err(RuntimeError::from_sys)?;
        }
        let _ = root.remove_file(Path::new("usr/local/bin/adb"));
        sync_strata_debug_binary(mod_root, mount_dir, &root)?;
    }

    Ok(())
}

/// Synchronizes only `/etc/resolv.conf` inside a mounted container rootfs when DNS changes.
pub fn sync_container_dns(mount_dir: &Path, cfg: &ContainerConfig) -> RuntimeResult<bool> {
    let root = RootDir::open(mount_dir).map_err(RuntimeError::from_sys)?;
    root.ensure_dir(Path::new("etc"), 0o755)
        .map_err(RuntimeError::from_sys)?;
    let android_dns = if cfg.sync_android_dns {
        net::read_android_dns_servers()
    } else {
        Vec::new()
    };
    let resolv = render_resolv_conf(&android_dns, &cfg.dns_servers);
    root.sync_bytes_if_changed(resolv.as_bytes(), Path::new("etc/resolv.conf"), 0o644)
        .map_err(RuntimeError::from_sys)
}

/// Renders deduplicated `/etc/resolv.conf` prioritizing active Android DNS resolvers.
#[must_use]
pub fn render_resolv_conf(android_dns: &[String], fallback_dns: &[String]) -> String {
    let mut seen = Vec::new();
    let mut out = String::new();
    for server in android_dns.iter().chain(fallback_dns.iter()) {
        let trimmed = server.trim();
        if trimmed.is_empty() || seen.iter().any(|s| s == trimmed) {
            continue;
        }
        seen.push(trimmed.to_owned());
        out.push_str("nameserver ");
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}

/// Renders `/run/strata/host.env` used by container `host-exec` to enter Android PID 1 cleanly.
#[must_use]
pub fn render_host_env(classpath_pairs: &[(String, String)]) -> String {
    let env_or = |key: &str, default: &str| -> String {
        std::env::var(key)
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| default.to_owned())
    };

    let mut lines = vec![
        "PATH=/product/bin:/apex/com.android.runtime/bin:/apex/com.android.art/bin:/apex/com.android.virt/bin:/system_ext/bin:/system/bin:/system/xbin:/odm/bin:/vendor/bin:/vendor/xbin:/data/adb/ksu/bin".to_owned(),
        format!("ANDROID_ROOT={}", env_or("ANDROID_ROOT", "/system")),
        format!("ANDROID_DATA={}", env_or("ANDROID_DATA", "/data")),
        format!("ANDROID_STORAGE={}", env_or("ANDROID_STORAGE", "/storage")),
        format!(
            "ANDROID_ART_ROOT={}",
            env_or("ANDROID_ART_ROOT", "/apex/com.android.art")
        ),
        format!(
            "ANDROID_I18N_ROOT={}",
            env_or("ANDROID_I18N_ROOT", "/apex/com.android.i18n")
        ),
        format!(
            "ANDROID_TZDATA_ROOT={}",
            env_or("ANDROID_TZDATA_ROOT", "/apex/com.android.tzdata")
        ),
        format!(
            "ANDROID_ASSETS={}",
            env_or("ANDROID_ASSETS", "/system/app")
        ),
        format!(
            "EXTERNAL_STORAGE={}",
            env_or("EXTERNAL_STORAGE", "/sdcard")
        ),
        "TMPDIR=/data/local/tmp".to_owned(),
        "HOME=/".to_owned(),
        "USER=root".to_owned(),
    ];

    for (key, val) in classpath_pairs {
        if !key.is_empty() && !val.is_empty() && !val.contains('\n') {
            lines.push(format!("{key}={val}"));
        }
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Returns the last `max_lines` lines of `log_path` without loading unbounded memory.
#[must_use]
pub fn read_log_tail(log_path: &Path, max_lines: usize) -> Vec<String> {
    if max_lines == 0 {
        return Vec::new();
    }
    let Ok(mut file) = fs::File::open(log_path) else {
        return Vec::new();
    };
    let Ok(meta) = file.metadata() else {
        return Vec::new();
    };
    let len = meta.len();
    let truncated_start = len > MAX_LOG_TAIL_READ_BYTES;
    if truncated_start
        && file
            .seek(SeekFrom::Start(len - MAX_LOG_TAIL_READ_BYTES))
            .is_err()
    {
        return Vec::new();
    }
    let mut raw = Vec::with_capacity(len.min(MAX_LOG_TAIL_READ_BYTES) as usize);
    if file
        .take(MAX_LOG_TAIL_READ_BYTES)
        .read_to_end(&mut raw)
        .is_err()
    {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&raw);
    let slice = match (truncated_start, text.find('\n')) {
        (true, Some(pos)) => &text[pos + 1..],
        _ => &text,
    };
    let all: Vec<&str> = slice.lines().collect();
    let start = all.len().saturating_sub(max_lines);
    all[start..].iter().map(|s| (*s).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_deduplicated_resolv_conf_and_host_env() {
        let resolv = render_resolv_conf(
            &["192.0.2.53".to_owned(), "1.1.1.1".to_owned()],
            &["1.1.1.1".to_owned(), "8.8.8.8".to_owned()],
        );
        assert_eq!(
            resolv,
            "nameserver 192.0.2.53\nnameserver 1.1.1.1\nnameserver 8.8.8.8\n"
        );

        let env = render_host_env(&[("BOOTCLASSPATH".to_owned(), "/apex/core.jar".to_owned())]);
        assert!(env.contains("BOOTCLASSPATH=/apex/core.jar\n"));
        assert!(env.contains("TMPDIR=/data/local/tmp\n"));
    }

    #[test]
    fn syncs_host_exec_and_static_strata_debug_binary() {
        let base = std::env::temp_dir().join(format!("strata-sync-test-{}", std::process::id()));
        let mod_root = base.join("mod");
        let mnt = base.join("mnt");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(mod_root.join("bin")).expect("create mod bin");
        fs::create_dir_all(mod_root.join("container")).expect("create mod container");
        fs::create_dir_all(mnt.join("usr/local/bin")).expect("create mnt bin");
        fs::create_dir_all(mnt.join("usr/local/lib/strata-debug")).expect("create legacy dir");
        fs::write(mnt.join("usr/local/bin/adb"), b"legacy").expect("write legacy");
        fs::write(mnt.join("usr/local/lib/strata-debug/cli.py"), b"old\n")
            .expect("write legacy py");
        fs::write(mod_root.join("container/host-exec"), b"#!/bin/sh\n").expect("write host-exec");
        fs::write(mod_root.join("bin/strata-debug"), b"\x7fELF-static").expect("write binary");

        let mut cfg = ContainerConfig::default_for_id("main");
        cfg.sync_android_dns = false;
        sync_container_runtime_files(&mnt, &cfg, Some(&mod_root)).expect("sync");

        assert!(mnt.join("usr/local/bin/host-exec").is_file());
        assert_eq!(
            fs::read(mnt.join("usr/local/bin/strata-debug")).expect("read strata-debug"),
            b"\x7fELF-static"
        );
        assert!(!mnt.join("usr/local/lib/strata-debug").exists());
        assert!(!mnt.join("usr/local/bin/adb").exists());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn read_log_tail_handles_large_files_with_bounded_window() {
        let path = std::env::temp_dir().join(format!("strata-log-tail-{}.log", std::process::id()));
        let content: String = (0..8_000)
            .map(|idx| format!("line-{idx:05}-padding-for-bounded-tail-test\n"))
            .collect();
        assert!(content.len() as u64 > MAX_LOG_TAIL_READ_BYTES);
        fs::write(&path, &content).expect("write large log");
        assert_eq!(
            read_log_tail(&path, 2),
            vec![
                "line-07998-padding-for-bounded-tail-test",
                "line-07999-padding-for-bounded-tail-test",
            ]
        );
        let _ = fs::remove_file(&path);
    }
}
