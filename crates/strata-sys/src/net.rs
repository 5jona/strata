//! Pure-Rust network interface IPv4 detection (`getifaddrs`) and Android DNS property reading.

use std::ffi::CStr;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::Command;

const GETPROP_BIN: &str = "/system/bin/getprop";

/// Detects the primary non-loopback IPv4 address via `getifaddrs`, preferring `wlan0`.
/// Falls back to `"127.0.0.1"` when no active IPv4 interface is present.
#[must_use]
pub fn detect_primary_ipv4() -> String {
    let mut addrs: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `getifaddrs` writes an allocated linked list head pointer into `addrs`.
    if unsafe { libc::getifaddrs(&mut addrs) } != 0 {
        return "127.0.0.1".to_owned();
    }

    let mut wlan_ip: Option<Ipv4Addr> = None;
    let mut fallback_ip: Option<Ipv4Addr> = None;

    let mut cursor = addrs;
    while !cursor.is_null() {
        // SAFETY: `cursor` traverses the kernel-populated linked list terminated by NULL.
        let ifa = unsafe { &*cursor };
        if !ifa.ifa_addr.is_null() && !ifa.ifa_name.is_null() {
            // SAFETY: `ifa_addr` is non-null; inspect `sa_family` before casting to `sockaddr_in`.
            let family = unsafe { (*ifa.ifa_addr).sa_family as i32 };
            if family == libc::AF_INET {
                // SAFETY: `sa_family == AF_INET` guarantees `ifa_addr` points to `sockaddr_in`.
                let sin = unsafe { &*(ifa.ifa_addr.cast::<libc::sockaddr_in>()) };
                let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                if !ip.is_loopback() && !ip.is_unspecified() {
                    // SAFETY: `ifa_name` is a non-null NUL-terminated interface name string.
                    let name = unsafe { CStr::from_ptr(ifa.ifa_name) }.to_string_lossy();
                    if name == "wlan0" {
                        wlan_ip = Some(ip);
                        break;
                    }
                    if fallback_ip.is_none() {
                        fallback_ip = Some(ip);
                    }
                }
            }
        }
        cursor = ifa.ifa_next;
    }

    // SAFETY: `addrs` was allocated by `getifaddrs` and must be freed once via `freeifaddrs`.
    unsafe {
        libc::freeifaddrs(addrs);
    }

    wlan_ip
        .or(fallback_ip)
        .map_or_else(|| "127.0.0.1".to_owned(), |ip| ip.to_string())
}

/// Reads active Android DNS servers from `net.dns1` / `net.dns2` via `/system/bin/getprop`.
#[must_use]
pub fn read_android_dns_servers() -> Vec<String> {
    if !Path::new(GETPROP_BIN).exists() {
        return Vec::new();
    }
    let mut servers = Vec::new();
    for prop in ["net.dns1", "net.dns2"] {
        let Ok(out) = Command::new(GETPROP_BIN).arg(prop).output() else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        let val = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if is_valid_ipv4(&val) && !servers.contains(&val) {
            servers.push(val);
        }
    }
    servers
}

/// Returns true if `raw` parses as a strict IPv4 address.
#[must_use]
pub fn is_valid_ipv4(raw: &str) -> bool {
    raw.parse::<Ipv4Addr>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ipv4_addresses() {
        assert!(is_valid_ipv4("1.1.1.1"));
        assert!(is_valid_ipv4("223.5.5.5"));
        assert!(!is_valid_ipv4("not-an-ip"));
        assert!(!is_valid_ipv4("256.1.1.1"));
        assert!(!is_valid_ipv4(""));
    }
}
