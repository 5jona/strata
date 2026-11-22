//! Synchronization of container DNS, Android `host.env`, bridge scripts, SSH keys, and log tails.

use std::fs;
use std::path::Path;

use strata_protocol::ContainerConfig;
use strata_sys::{net, procfs};

use crate::error::{RuntimeError, RuntimeResult};
use crate::fs_atomic::{ensure_dir_mode, sync_bytes_if_changed, write_atomic};

/// Synchronizes `/etc/resolv.conf`, `/run/strata/host.env`, and POSIX/Python host bridge scripts
/// inside a mounted container rootfs.
pub fn sync_container_runtime_files(
    mount_dir: &Path,
    cfg: &ContainerConfig,
    module_dir: Option<&Path>,
) -> RuntimeResult<()> {
    ensure_dir_mode(&mount_dir.join("etc"), 0o755)?;
    ensure_dir_mode(&mount_dir.join("run/strata"), 0o755)?;
    ensure_dir_mode(&mount_dir.join("run/sshd"), 0o755)?;
    ensure_dir_mode(&mount_dir.join("usr/local/bin"), 0o755)?;

    let android_dns = if cfg.sync_android_dns {
        net::read_android_dns_servers()
    } else {
        Vec::new()
    };
    let resolv = render_resolv_conf(&android_dns, &cfg.dns_servers);
    sync_bytes_if_changed(resolv.as_bytes(), &mount_dir.join("etc/resolv.conf"), 0o644)?;

    let classpath = procfs::read_android_classpath_env();
    let host_env = render_host_env(&classpath);
    sync_bytes_if_changed(
        host_env.as_bytes(),
        &mount_dir.join("run/strata/host.env"),
        0o644,
    )?;

    if let Some(mod_root) = module_dir {
        let host_exec_src = mod_root.join("container/host-exec");
        if let Ok(bytes) = fs::read(&host_exec_src) {
            sync_bytes_if_changed(&bytes, &mount_dir.join("usr/local/bin/host-exec"), 0o755)?;
        }
        let legacy_adb = mount_dir.join("usr/local/bin/adb");
        if legacy_adb.exists() {
            let _ = fs::remove_file(&legacy_adb);
        }
        sync_strata_debug_package(mod_root, mount_dir)?;
    }

    Ok(())
}

/// Synchronizes only `/etc/resolv.conf` inside a mounted container rootfs when DNS changes.
pub fn sync_container_dns(mount_dir: &Path, cfg: &ContainerConfig) -> RuntimeResult<bool> {
    ensure_dir_mode(&mount_dir.join("etc"), 0o755)?;
    let android_dns = if cfg.sync_android_dns {
        net::read_android_dns_servers()
    } else {
        Vec::new()
    };
    let resolv = render_resolv_conf(&android_dns, &cfg.dns_servers);
    sync_bytes_if_changed(resolv.as_bytes(), &mount_dir.join("etc/resolv.conf"), 0o644)
}

const STRATA_DEBUG_LAUNCHER: &[u8] =
    b"#!/bin/sh\nexec /usr/bin/python3 /usr/local/lib/strata-debug/cli.py \"$@\"\n";

fn sync_strata_debug_package(mod_root: &Path, mount_dir: &Path) -> RuntimeResult<()> {
    let src_dir = mod_root.join("container/strata-debug");
    if !src_dir.is_dir() {
        return Ok(());
    }
    let dst_dir = mount_dir.join("usr/local/lib/strata-debug");
    ensure_dir_mode(&dst_dir, 0o755)?;

    let entries = fs::read_dir(&src_dir).map_err(|source| RuntimeError::Io {
        op: "read strata-debug dir",
        source,
    })?;
    let mut synced_any = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".py") {
            continue;
        }
        if let Ok(bytes) = fs::read(&path) {
            let mode = if name == "cli.py" || name == "forward_bridge.py" {
                0o755
            } else {
                0o644
            };
            sync_bytes_if_changed(&bytes, &dst_dir.join(name), mode)?;
            synced_any = true;
        }
    }
    if synced_any {
        sync_bytes_if_changed(
            STRATA_DEBUG_LAUNCHER,
            &mount_dir.join("usr/local/bin/strata-debug"),
            0o755,
        )?;
    }
    Ok(())
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

/// Reads authorized SSH public keys from `<mount_dir>/root/.ssh/authorized_keys`.
pub fn list_ssh_keys(mount_dir: &Path) -> RuntimeResult<Vec<String>> {
    let path = mount_dir.join("root/.ssh/authorized_keys");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path).map_err(|source| RuntimeError::Io {
        op: "read authorized_keys",
        source,
    })?;
    Ok(parse_ssh_keys(&content))
}

/// Appends one or more newline-separated SSH public keys deduplicated and persisted atomically.
pub fn add_ssh_keys(mount_dir: &Path, input: &str) -> RuntimeResult<(Vec<String>, usize)> {
    let ssh_dir = mount_dir.join("root/.ssh");
    ensure_dir_mode(&ssh_dir, 0o700)?;
    let key_file = ssh_dir.join("authorized_keys");

    let mut keys = list_ssh_keys(mount_dir)?;
    let mut added = 0;
    for raw_line in input.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !keys.iter().any(|k| k == trimmed) {
            keys.push(trimmed.to_owned());
            added += 1;
        }
    }

    let mut rendered = keys.join("\n");
    if !rendered.is_empty() {
        rendered.push('\n');
    }
    write_atomic(&key_file, rendered.as_bytes(), 0o600)?;
    Ok((keys, added))
}

/// Returns the last `max_lines` lines of `log_path` without loading unbounded memory.
#[must_use]
pub fn read_log_tail(log_path: &Path, max_lines: usize) -> Vec<String> {
    if max_lines == 0 {
        return Vec::new();
    }
    let Ok(content) = fs::read_to_string(log_path) else {
        return Vec::new();
    };
    let all: Vec<&str> = content.lines().collect();
    let start = all.len().saturating_sub(max_lines);
    all[start..].iter().map(|s| (*s).to_owned()).collect()
}

fn parse_ssh_keys(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect()
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
    fn syncs_strata_debug_package_and_launcher() {
        let base = std::env::temp_dir().join(format!("strata-sync-test-{}", std::process::id()));
        let mod_root = base.join("mod");
        let mnt = base.join("mnt");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(mod_root.join("container/strata-debug")).expect("create src");
        fs::create_dir_all(mnt.join("usr/local/bin")).expect("create mnt bin");
        fs::write(mnt.join("usr/local/bin/adb"), b"legacy").expect("write legacy");
        fs::write(
            mod_root.join("container/strata-debug/cli.py"),
            b"print(1)\n",
        )
        .expect("write");
        fs::write(mod_root.join("container/strata-debug/ignore.txt"), b"skip").expect("write");

        let mut cfg = ContainerConfig::default_for_id("main");
        cfg.sync_android_dns = false;
        sync_container_runtime_files(&mnt, &cfg, Some(&mod_root)).expect("sync");

        assert!(mnt.join("usr/local/lib/strata-debug/cli.py").is_file());
        assert!(!mnt.join("usr/local/lib/strata-debug/ignore.txt").exists());
        assert!(mnt.join("usr/local/bin/strata-debug").is_file());
        assert!(!mnt.join("usr/local/bin/adb").exists());

        let _ = fs::remove_dir_all(&base);
    }
}
