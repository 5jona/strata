//! Pure HTTP/WS authentication policy helpers: peer IP/RFC1918, `Host`, `Origin`,
//! IPv6 `/64` rate-limit bucketing, bearer/subprotocol extraction, and constant-time equality.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use axum::http::{HeaderMap, header};

/// Extracts `(token, optional_matched_ws_protocol)` from `Authorization: Bearer` or `Sec-WebSocket-Protocol`.
#[must_use]
pub(crate) fn extract_bearer_token(headers: &HeaderMap) -> Option<(&str, Option<&str>)> {
    if let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(tok) = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
    {
        let trimmed = tok.trim();
        if !trimmed.is_empty() {
            return Some((trimmed, None));
        }
    }
    if let Some(proto) = headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
    {
        for part in proto.split(',') {
            let trimmed = part.trim();
            if let Some(tok) = trimmed.strip_prefix("strata-auth.")
                && !tok.is_empty()
            {
                return Some((tok, Some(trimmed)));
            }
        }
    }
    None
}

/// Normalizes IPv4-mapped IPv6 and buckets non-loopback IPv6 addresses to `/64` prefixes.
#[must_use]
pub(crate) fn rate_limit_bucket(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                IpAddr::V4(v4)
            } else if v6.is_loopback() {
                IpAddr::V6(v6)
            } else {
                let masked = u128::from(v6) & 0xffff_ffff_ffff_ffff_0000_0000_0000_0000;
                IpAddr::V6(Ipv6Addr::from(masked))
            }
        }
    }
}

/// Checks whether an HTTP `Host` header targets `localhost` or an allowed peer IP (DNS rebinding guard).
#[must_use]
pub(crate) fn is_allowed_host_header(host_header: &str, lan_enabled: bool) -> bool {
    let trimmed = host_header.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('@') {
        return false;
    }
    let host = extract_host_part(trimmed);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_allowed_peer_ip(ip, lan_enabled);
    }
    false
}

/// Checks whether a connecting peer IP is permitted under loopback-only or RFC1918 LAN mode.
#[must_use]
pub(crate) fn is_allowed_peer_ip(ip: IpAddr, lan_enabled: bool) -> bool {
    let normalized = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    };
    match normalized {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                return true;
            }
            lan_enabled && is_rfc1918_v4(v4)
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            lan_enabled && is_private_v6(v6)
        }
    }
}

/// Checks whether an HTTP `Origin` header is a trusted Tauri/localhost/RFC1918 origin.
#[must_use]
pub(crate) fn is_allowed_origin(origin: &str, lan_enabled: bool) -> bool {
    let trimmed = origin.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed == "tauri://localhost"
        || trimmed == "http://tauri.localhost"
        || trimmed == "https://tauri.localhost"
    {
        return true;
    }
    let Some(rest) = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
    else {
        return false;
    };
    let host_port = rest.split('/').next().unwrap_or("");
    let host = extract_host_part(host_port);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_allowed_peer_ip(ip, lan_enabled);
    }
    false
}

fn extract_host_part(host_port: &str) -> &str {
    if host_port.starts_with('[') {
        host_port
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    }
}

/// Constant-time byte-slice comparison using `black_box` to deter compiler short-circuiting.
#[must_use]
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= std::hint::black_box(x ^ y);
    }
    std::hint::black_box(diff) == 0
}

fn is_rfc1918_v4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    let seg0 = ip.segments()[0];
    (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
}
