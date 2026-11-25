//! Bearer token persistence, verification, and bounded per-IP/IPv6-/64 failure rate limiting.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::net::IpAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, StatusCode, header};
use strata_runtime::fs_atomic;
use strata_sys::pty;
use tokio::sync::{Mutex, RwLock};

use crate::web::auth_policy::{ct_eq, extract_bearer_token, is_allowed_origin, rate_limit_bucket};
pub(crate) use crate::web::auth_policy::{is_allowed_host_header, is_allowed_peer_ip};

const TOKEN_BYTES: usize = 32;
const MAX_RATE_LIMIT_IPS: usize = 256;
const MAX_AUTH_FAILURES: u32 = 5;
const LOCKOUT_DURATION: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct FailureRecord {
    count: u32,
    locked_until: Option<Instant>,
    last_seen: Instant,
}

/// Thread-safe token and rate-limit state for the embedded HTTP/WS API.
#[derive(Debug)]
pub(crate) struct WebAuthState {
    token_path: PathBuf,
    token: RwLock<String>,
    failures: Mutex<HashMap<IpAddr, FailureRecord>>,
}

impl WebAuthState {
    /// Loads an existing 256-bit hex token from `token_path` (rejecting symlinks & enforcing `0600`),
    /// or generates and persists a fresh token atomically.
    pub(crate) fn load_or_generate(token_path: PathBuf) -> Result<Self, String> {
        if let Ok(meta) = fs::symlink_metadata(&token_path) {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(format!(
                    "refusing to use non-regular or symlink web token path '{}'",
                    token_path.display()
                ));
            }
            let _ = fs::set_permissions(&token_path, fs::Permissions::from_mode(0o600));
        }

        let token = match fs::read_to_string(&token_path) {
            Ok(raw) if raw.trim().len() >= 32 => raw.trim().to_owned(),
            _ => {
                let generated = generate_hex_token()?;
                persist_token(&token_path, &generated)?;
                generated
            }
        };
        Ok(Self {
            token_path,
            token: RwLock::new(token),
            failures: Mutex::new(HashMap::new()),
        })
    }

    #[must_use]
    pub(crate) fn token_path(&self) -> &Path {
        &self.token_path
    }

    /// Generates and atomically persists a brand-new 256-bit token, returning `(new_token, path)`.
    pub(crate) async fn rotate_token(&self) -> Result<(String, String), String> {
        let next = generate_hex_token()?;
        let path = self.token_path.clone();
        let to_write = next.clone();
        tokio::task::spawn_blocking(move || persist_token(&path, &to_write))
            .await
            .map_err(|e| format!("rotate token join: {e}"))??;
        *self.token.write().await = next.clone();
        self.failures.lock().await.clear();
        Ok((next, self.token_path.to_string_lossy().into_owned()))
    }

    /// Validates peer IP, `Host`, optional `Origin`, Bearer token (checked BEFORE lockout so valid
    /// local holders cannot be DoS'd on `127.0.0.1`), and rate-limits failures.
    /// Returns `Ok(Some(subprotocol))` when authenticated via `Sec-WebSocket-Protocol`.
    pub(crate) async fn verify_request(
        &self,
        peer_ip: IpAddr,
        lan_enabled: bool,
        headers: &HeaderMap,
    ) -> Result<Option<String>, (StatusCode, &'static str)> {
        if !is_allowed_peer_ip(peer_ip, lan_enabled) {
            return Err((StatusCode::FORBIDDEN, "peer IP not allowed"));
        }
        if let Some(host_val) = headers.get(header::HOST) {
            let host = host_val.to_str().unwrap_or("");
            if !is_allowed_host_header(host, lan_enabled) {
                return Err((StatusCode::FORBIDDEN, "host header not allowed"));
            }
        }
        if let Some(origin_val) = headers.get(header::ORIGIN) {
            let origin = origin_val.to_str().unwrap_or("");
            if !is_allowed_origin(origin, lan_enabled) {
                return Err((StatusCode::FORBIDDEN, "origin not allowed"));
            }
        }

        let bucket = rate_limit_bucket(peer_ip);
        let extracted = extract_bearer_token(headers);
        if let Some((candidate, matched_proto)) = extracted {
            let expected = self.token.read().await;
            if ct_eq(candidate.as_bytes(), expected.as_bytes()) {
                drop(expected);
                self.clear_failures(bucket).await;
                return Ok(matched_proto.map(ToOwned::to_owned));
            }
        }

        if self.is_rate_limited(bucket).await {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                "too many authentication failures",
            ));
        }

        self.record_failure(bucket).await;
        if extracted.is_none() {
            Err((StatusCode::UNAUTHORIZED, "missing bearer token"))
        } else {
            Err((StatusCode::UNAUTHORIZED, "invalid bearer token"))
        }
    }

    async fn is_rate_limited(&self, bucket: IpAddr) -> bool {
        let mut guard = self.failures.lock().await;
        let now = Instant::now();
        if let Some(rec) = guard.get_mut(&bucket)
            && let Some(until) = rec.locked_until
        {
            if now < until {
                return true;
            }
            rec.locked_until = None;
            rec.count = 0;
        }
        false
    }

    async fn record_failure(&self, bucket: IpAddr) {
        let mut guard = self.failures.lock().await;
        let now = Instant::now();
        if guard.len() >= MAX_RATE_LIMIT_IPS && !guard.contains_key(&bucket) {
            guard.retain(|_, rec| {
                rec.locked_until.is_some_and(|until| now < until)
                    || now.duration_since(rec.last_seen) < LOCKOUT_DURATION
            });
            if guard.len() >= MAX_RATE_LIMIT_IPS {
                let victim = guard
                    .iter()
                    .filter(|(_, rec)| !rec.locked_until.is_some_and(|until| now < until))
                    .min_by_key(|(_, rec)| rec.last_seen)
                    .or_else(|| guard.iter().min_by_key(|(_, rec)| rec.last_seen))
                    .map(|(k, _)| *k);
                if let Some(k) = victim {
                    guard.remove(&k);
                }
            }
        }
        let rec = guard.entry(bucket).or_insert(FailureRecord {
            count: 0,
            locked_until: None,
            last_seen: now,
        });
        rec.last_seen = now;
        rec.count = rec.count.saturating_add(1);
        if rec.count >= MAX_AUTH_FAILURES {
            rec.locked_until = Some(now + LOCKOUT_DURATION);
        }
    }

    async fn clear_failures(&self, bucket: IpAddr) {
        self.failures.lock().await.remove(&bucket);
    }
}

fn generate_hex_token() -> Result<String, String> {
    let mut raw = [0u8; TOKEN_BYTES];
    pty::fill_random_bytes(&mut raw).map_err(|e| format!("generate web token: {e}"))?;
    let mut hex = String::with_capacity(TOKEN_BYTES * 2);
    for b in raw {
        let _ = write!(&mut hex, "{b:02x}");
    }
    Ok(hex)
}

fn persist_token(path: &Path, token: &str) -> Result<(), String> {
    let mut content = token.to_owned();
    content.push('\n');
    fs_atomic::write_atomic(path, content.as_bytes(), 0o600)
        .map_err(|e| format!("persist web token '{}': {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_rfc1918_peer_host_and_origin_guards() {
        let lo: IpAddr = "127.0.0.1".parse().expect("lo");
        let lan: IpAddr = "192.168.1.42".parse().expect("lan");
        let pub_ip: IpAddr = "8.8.8.8".parse().expect("pub");

        assert!(is_allowed_peer_ip(lo, false));
        assert!(!is_allowed_peer_ip(lan, false));
        assert!(is_allowed_peer_ip(lan, true));
        assert!(!is_allowed_peer_ip(pub_ip, true));

        assert!(is_allowed_host_header("127.0.0.1:7681", false));
        assert!(is_allowed_host_header("localhost:7681", false));
        assert!(!is_allowed_host_header("evil.example.com:7681", true));
        assert!(!is_allowed_host_header("192.168.1.10:7681", false));
        assert!(is_allowed_host_header("192.168.1.10:7681", true));

        let v6_a: IpAddr = "fd00:1234:5678:9abc::1".parse().expect("v6a");
        let v6_b: IpAddr = "fd00:1234:5678:9abc::9999".parse().expect("v6b");
        assert_eq!(rate_limit_bucket(v6_a), rate_limit_bucket(v6_b));
    }

    #[tokio::test]
    async fn valid_token_bypasses_localhost_429_and_rate_limits_bad_tokens() {
        let dir = std::env::temp_dir().join(format!("strata-auth-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let token_path = dir.join("web.token");
        let auth = WebAuthState::load_or_generate(token_path.clone()).expect("init auth");
        let tok = fs::read_to_string(&token_path)
            .expect("read")
            .trim()
            .to_owned();
        let lo: IpAddr = "127.0.0.1".parse().expect("lo");

        let mut bad_headers = HeaderMap::new();
        bad_headers.insert(header::AUTHORIZATION, "Bearer wrong".parse().expect("hdr"));
        for _ in 0..MAX_AUTH_FAILURES {
            let err = auth
                .verify_request(lo, false, &bad_headers)
                .await
                .expect_err("reject");
            assert_eq!(err.0, StatusCode::UNAUTHORIZED);
        }
        let locked = auth
            .verify_request(lo, false, &bad_headers)
            .await
            .expect_err("lockout");
        assert_eq!(locked.0, StatusCode::TOO_MANY_REQUESTS);

        // Legitimate holder on localhost must NOT be DoS'd by the 429 lockout.
        let mut good_headers = HeaderMap::new();
        good_headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {tok}").parse().expect("hdr"),
        );
        assert!(auth.verify_request(lo, false, &good_headers).await.is_ok());

        // WS subprotocol auth returns matched protocol for echoing in 101 response.
        let mut ws_headers = HeaderMap::new();
        let proto = format!("strata-auth.{tok}");
        ws_headers.insert(header::SEC_WEBSOCKET_PROTOCOL, proto.parse().expect("ws"));
        let matched = auth
            .verify_request(lo, false, &ws_headers)
            .await
            .expect("ws ok");
        assert_eq!(matched.as_deref(), Some(proto.as_str()));

        let _ = fs::remove_dir_all(&dir);
    }
}
