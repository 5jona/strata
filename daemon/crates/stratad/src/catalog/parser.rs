//! HTTPS-only anti-SSRF URL validation and Simplestreams `images.json` ARM64 parser.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde_json::Value;
use strata_protocol::constants::{DEFAULT_DISTRO_MIRROR_OFFICIAL, DEFAULT_DISTRO_MIRROR_TUNA};
use strata_protocol::rpc::{DistroEntry, DistroMirror};

use crate::error::{DaemonError, DaemonResult};

/// Validates that a remote catalog or rootfs download URL uses `https://` and does not target
/// loopback, RFC1918 private networks, link-local, unspecified, or multicast addresses.
pub(crate) fn validate_remote_https_url(raw: &str) -> DaemonResult<reqwest::Url> {
    let trimmed = raw.trim();
    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|e| DaemonError::InvalidParams(format!("invalid URL '{trimmed}': {e}")))?;
    if parsed.scheme() != "https" {
        return Err(DaemonError::InvalidParams(format!(
            "remote URL '{trimmed}' must use https://"
        )));
    }
    let Some(host) = parsed.host_str() else {
        return Err(DaemonError::InvalidParams(format!(
            "remote URL '{trimmed}' missing host"
        )));
    };
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") || lower.ends_with(".local") {
        return Err(DaemonError::InvalidParams(format!(
            "remote URL host '{host}' is disallowed"
        )));
    }
    let unbracketed = lower.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = unbracketed.parse::<IpAddr>()
        && is_disallowed_remote_ip(ip)
    {
        return Err(DaemonError::InvalidParams(format!(
            "remote URL IP '{ip}' is disallowed (loopback/private/link-local/multicast)"
        )));
    }
    Ok(parsed)
}

fn is_disallowed_remote_ip(ip: IpAddr) -> bool {
    let norm = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    };
    match norm {
        IpAddr::V4(v4) => is_disallowed_v4(v4),
        IpAddr::V6(v6) => is_disallowed_v6(v6),
    }
}

fn is_disallowed_v4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || (a == 100 && (64..=127).contains(&b))
        || a == 0
}

fn is_disallowed_v6(ip: Ipv6Addr) -> bool {
    let seg0 = ip.segments()[0];
    ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unspecified()
        || (seg0 & 0xfe00) == 0xfc00
        || (seg0 & 0xffc0) == 0xfe80
}

/// Resolves mirror enum + optional custom URL into a validated HTTPS base URL.
pub(crate) fn resolve_mirror_url(
    mirror: DistroMirror,
    custom_url: Option<&str>,
) -> DaemonResult<String> {
    match mirror {
        DistroMirror::Tuna => Ok(DEFAULT_DISTRO_MIRROR_TUNA.to_owned()),
        DistroMirror::Official => Ok(DEFAULT_DISTRO_MIRROR_OFFICIAL.to_owned()),
        DistroMirror::Custom => {
            let raw = custom_url.unwrap_or("").trim().trim_end_matches('/');
            let _ = validate_remote_https_url(raw)?;
            Ok(raw.to_owned())
        }
    }
}

/// Parses Simplestreams `streams/v1/images.json` into sorted ARM64 default Linux entries.
#[must_use]
pub(crate) fn parse_simplestreams_images(base_url: &str, root: &Value) -> Vec<DistroEntry> {
    let Some(products) = root.get("products").and_then(Value::as_object) else {
        return Vec::new();
    };
    let trimmed_base = base_url.trim_end_matches('/');
    let mut out = Vec::new();

    for (product_id, prod) in products {
        let arch = prod.get("arch").and_then(Value::as_str).unwrap_or("");
        let variant = prod.get("variant").and_then(Value::as_str).unwrap_or("");
        if arch != "arm64" || variant != "default" {
            continue;
        }
        let os = prod.get("os").and_then(Value::as_str).unwrap_or("");
        if matches!(os.to_ascii_lowercase().as_str(), "freebsd" | "netbsd") {
            continue;
        }
        let release = prod.get("release").and_then(Value::as_str).unwrap_or("");
        let release_title = prod
            .get("release_title")
            .and_then(Value::as_str)
            .unwrap_or(release);

        let Some(versions) = prod.get("versions").and_then(Value::as_object) else {
            continue;
        };
        let Some((latest_ver, ver_obj)) = versions.iter().max_by(|(a, _), (b, _)| a.cmp(b)) else {
            continue;
        };
        let Some(items) = ver_obj.get("items").and_then(Value::as_object) else {
            continue;
        };
        let item = items
            .get("rootfs.tar.xz")
            .or_else(|| items.get("rootfs.tar.gz"))
            .or_else(|| items.get("rootfs.tar.zst"));
        let Some(item) = item else {
            continue;
        };
        let Some(rel_path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        let size_bytes = item.get("size").and_then(Value::as_u64).unwrap_or(0);
        let sha256 = item
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();

        out.push(DistroEntry {
            id: product_id.clone(),
            os: os.to_owned(),
            release: release.to_owned(),
            release_title: release_title.to_owned(),
            arch: arch.to_owned(),
            variant: variant.to_owned(),
            version: latest_ver.clone(),
            size_bytes,
            sha256,
            download_url: format!("{trimmed_base}/{}", rel_path.trim_start_matches('/')),
        });
    }

    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_remote_https_urls_and_blocks_ssrf_targets() {
        assert!(
            validate_remote_https_url("https://images.linuxcontainers.org/rootfs.tar.xz").is_ok()
        );
        for bad in [
            "http://images.linuxcontainers.org/rootfs.tar.xz",
            "https://127.0.0.1:7681/api/rpc",
            "https://localhost/foo",
            "https://192.168.1.10/img",
            "https://10.0.0.1/img",
            "https://169.254.169.254/latest",
            "https://[::1]/img",
            "https://[::ffff:127.0.0.1]/img",
        ] {
            assert!(validate_remote_https_url(bad).is_err(), "must reject {bad}");
        }
    }

    #[test]
    fn parses_simplestreams_arm64_default_entries_only() {
        let sample = json!({
            "products": {
                "ubuntu:noble:arm64:default": {
                    "os": "Ubuntu",
                    "release": "noble",
                    "release_title": "24.04 LTS",
                    "arch": "arm64",
                    "variant": "default",
                    "versions": {
                        "20260101_07:42": {
                            "items": {
                                "rootfs.tar.xz": {
                                    "path": "images/ubuntu/noble/arm64/default/20260101/rootfs.tar.xz",
                                    "size": 1000,
                                    "sha256": "old"
                                }
                            }
                        },
                        "20261120_07:42": {
                            "items": {
                                "rootfs.tar.xz": {
                                    "path": "images/ubuntu/noble/arm64/default/20261120/rootfs.tar.xz",
                                    "size": 95000000,
                                    "sha256": "abc123"
                                }
                            }
                        }
                    }
                },
                "ubuntu:noble:amd64:default": {
                    "os": "Ubuntu",
                    "release": "noble",
                    "arch": "amd64",
                    "variant": "default",
                    "versions": {}
                }
            }
        });

        let entries = parse_simplestreams_images("https://mirror.example/lxc", &sample);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "ubuntu:noble:arm64:default");
        assert_eq!(entries[0].version, "20261120_07:42");
        assert_eq!(entries[0].sha256, "abc123");
    }
}
