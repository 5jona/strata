//! Minimal zero-dependency HTTP/1.1 Bearer-token health probe for `stratad` endpoints.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Probe result returned to the desktop UI when testing connectivity to a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointProbeResult {
    pub reachable: bool,
    pub authenticated: bool,
    pub latency_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Errors returned by raw HTTP probe parsing.
#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("failed to resolve `{host}:{port}`: {source}")]
    Resolve {
        host: String,
        port: u16,
        #[source]
        source: std::io::Error,
    },
    #[error("no socket address resolved for `{host}:{port}`")]
    NoAddress { host: String, port: u16 },
    #[error("connection failed to `{host}:{port}`: {source}")]
    Connect {
        host: String,
        port: u16,
        #[source]
        source: std::io::Error,
    },
    #[error("HTTP I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed HTTP response")]
    MalformedHttp,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InfoPayload {
    version: Option<String>,
    lan_enabled: Option<bool>,
}

/// Probes `GET /api/info` and (when `token` is provided) `POST /api/rpc` (`daemon.ping`)
/// with `Authorization: Bearer <token>`.
#[must_use]
pub fn probe_stratad_endpoint(
    host: &str,
    port: u16,
    token: Option<&str>,
    timeout: Duration,
) -> EndpointProbeResult {
    let started = Instant::now();
    let info_resp = match http_request(host, port, "GET", "/api/info", None, None, timeout) {
        Ok(resp) => resp,
        Err(err) => {
            return EndpointProbeResult {
                reachable: false,
                authenticated: false,
                latency_ms: elapsed_ms(started),
                version: None,
                lan_enabled: None,
                error: Some(err.to_string()),
            };
        }
    };

    if info_resp.status != 200 {
        return EndpointProbeResult {
            reachable: true,
            authenticated: false,
            latency_ms: elapsed_ms(started),
            version: None,
            lan_enabled: None,
            error: Some(format!("/api/info returned HTTP {}", info_resp.status)),
        };
    }

    let info = serde_json::from_str::<InfoPayload>(&info_resp.body).ok();
    let version = info.as_ref().and_then(|i| i.version.clone());
    let lan_enabled = info.as_ref().and_then(|i| i.lan_enabled);

    let Some(tok) = token.map(str::trim).filter(|t| !t.is_empty()) else {
        return EndpointProbeResult {
            reachable: true,
            authenticated: false,
            latency_ms: elapsed_ms(started),
            version,
            lan_enabled,
            error: None,
        };
    };

    let rpc_body = r#"{"jsonrpc":"2.0","id":1,"method":"daemon.ping","params":null}"#;
    match http_request(
        host,
        port,
        "POST",
        "/api/rpc",
        Some(tok),
        Some(rpc_body),
        timeout,
    ) {
        Ok(resp) if resp.status == 200 => EndpointProbeResult {
            reachable: true,
            authenticated: true,
            latency_ms: elapsed_ms(started),
            version,
            lan_enabled,
            error: None,
        },
        Ok(resp) => EndpointProbeResult {
            reachable: true,
            authenticated: false,
            latency_ms: elapsed_ms(started),
            version,
            lan_enabled,
            error: Some(format!("auth probe returned HTTP {}", resp.status)),
        },
        Err(err) => EndpointProbeResult {
            reachable: true,
            authenticated: false,
            latency_ms: elapsed_ms(started),
            version,
            lan_enabled,
            error: Some(err.to_string()),
        },
    }
}

struct HttpResponse {
    status: u16,
    body: String,
}

fn http_request(
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    bearer_token: Option<&str>,
    json_body: Option<&str>,
    timeout: Duration,
) -> Result<HttpResponse, ProbeError> {
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|source| ProbeError::Resolve {
            host: host.to_owned(),
            port,
            source,
        })?
        .next()
        .ok_or_else(|| ProbeError::NoAddress {
            host: host.to_owned(),
            port,
        })?;

    let mut stream =
        TcpStream::connect_timeout(&addr, timeout).map_err(|source| ProbeError::Connect {
            host: host.to_owned(),
            port,
            source,
        })?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let body = json_body.unwrap_or("");
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    if let Some(tok) = bearer_token {
        req.push_str(&format!("Authorization: Bearer {tok}\r\n"));
    }
    if json_body.is_some() {
        req.push_str("Content-Type: application/json\r\n");
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    req.push_str(body);

    stream.write_all(req.as_bytes())?;
    let mut raw = Vec::new();
    stream.take(65_536).read_to_end(&mut raw)?;
    parse_http_response(&raw)
}

fn parse_http_response(raw: &[u8]) -> Result<HttpResponse, ProbeError> {
    let text = String::from_utf8_lossy(raw);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Err(ProbeError::MalformedHttp);
    };
    let status_line = head.lines().next().ok_or(ProbeError::MalformedHttp)?;
    let status_str = status_line
        .split_whitespace()
        .nth(1)
        .ok_or(ProbeError::MalformedHttp)?;
    let status = status_str
        .parse::<u16>()
        .map_err(|_| ProbeError::MalformedHttp)?;
    Ok(HttpResponse {
        status,
        body: body.trim().to_owned(),
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn probes_info_and_bearer_auth_against_local_mock_server() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = [0u8; 2048];
                let n = sock.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                let resp = if req.starts_with("GET /api/info") {
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"name\":\"strata\",\"version\":\"0.1.0\",\"lanEnabled\":true}"
                } else if req.contains("Authorization: Bearer secret-token-123") {
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"pong\":true}}"
                } else {
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\n\r\n{\"error\":\"unauthorized\"}"
                };
                sock.write_all(resp.as_bytes()).unwrap();
            }
        });

        let res = probe_stratad_endpoint(
            "127.0.0.1",
            port,
            Some("secret-token-123"),
            Duration::from_secs(2),
        );
        server.join().unwrap();

        assert!(res.reachable);
        assert!(res.authenticated);
        assert_eq!(res.version.as_deref(), Some("0.1.0"));
        assert_eq!(res.lan_enabled, Some(true));
        assert!(res.error.is_none());
    }
}
