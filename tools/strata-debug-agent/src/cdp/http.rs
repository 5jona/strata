//! Lightweight HTTP/1.1 response framing (`Content-Length` and `chunked`) for CDP over UDS.

use tokio::io::{AsyncRead, AsyncReadExt};

/// Reads one complete HTTP/1.1 response from `stream`, stopping immediately once
/// `Content-Length` bytes or the terminal `0\r\n\r\n` chunk arrive without waiting for EOF.
pub(crate) async fn read_http_response<R>(stream: &mut R) -> Result<(u16, Vec<u8>), String>
where
    R: AsyncRead + Unpin,
{
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    let mut framing: Option<(usize, Option<usize>, bool)> = None;

    loop {
        if let Some((sep, content_len, chunked)) = framing {
            let body = &raw[sep + 4..];
            if chunked {
                if decode_chunked(body).is_ok() {
                    break;
                }
            } else if let Some(cl) = content_len
                && body.len() >= cl
            {
                break;
            }
        }
        let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        if framing.is_none()
            && let Some(sep) = raw.windows(4).position(|w| w == b"\r\n\r\n")
        {
            let (_, cl, chunked) = inspect_headers(&raw[..sep])?;
            framing = Some((sep, cl, chunked));
        }
    }
    parse_http_response(&raw)
}

fn inspect_headers(header_bytes: &[u8]) -> Result<(u16, Option<usize>, bool), String> {
    let head = String::from_utf8_lossy(header_bytes);
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| "empty HTTP response".to_owned())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("malformed status line: {status_line}"))?;

    let mut content_len = None;
    let mut chunked = false;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_len = v.trim().parse::<usize>().ok();
            } else if k.trim().eq_ignore_ascii_case("transfer-encoding")
                && v.to_ascii_lowercase().contains("chunked")
            {
                chunked = true;
            }
        }
    }
    Ok((status, content_len, chunked))
}

/// Parses a raw HTTP/1.1 response supporting `Content-Length`, `chunked`, and EOF framing.
pub(crate) fn parse_http_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "malformed HTTP response: missing header terminator".to_owned())?;
    let (status, content_len, chunked) = inspect_headers(&raw[..sep])?;
    let body_raw = &raw[sep + 4..];
    let body = if chunked {
        decode_chunked(body_raw)?
    } else if let Some(cl) = content_len {
        if body_raw.len() < cl {
            return Err(format!(
                "truncated HTTP body: got {} of {cl} bytes",
                body_raw.len()
            ));
        }
        body_raw[..cl].to_vec()
    } else {
        body_raw.to_vec()
    };
    Ok((status, body))
}

pub(crate) fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let Some(rel) = data[pos..].windows(2).position(|w| w == b"\r\n") else {
            return Err("incomplete chunk size line".to_owned());
        };
        let size_line = String::from_utf8_lossy(&data[pos..pos + rel]);
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|e| format!("invalid chunk size '{size_hex}': {e}"))?;
        pos += rel + 2;
        if size == 0 {
            let rem = &data[pos..];
            if rem.starts_with(b"\r\n") || rem.windows(4).any(|w| w == b"\r\n\r\n") {
                return Ok(out);
            }
            return Err("missing terminal chunk CRLF".to_owned());
        }
        if pos + size + 2 > data.len() {
            return Err("truncated chunk payload".to_owned());
        }
        if &data[pos + size..pos + size + 2] != b"\r\n" {
            return Err("malformed chunk payload terminator".to_owned());
        }
        out.extend_from_slice(&data[pos..pos + size]);
        pos += size + 2;
    }
    Err("missing terminal zero chunk".to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    use super::*;

    #[tokio::test]
    async fn reads_content_length_response_without_eof() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let writer = tokio::spawn(async move {
            server
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\n[{\"id\":")
                .await
                .expect("write head");
            server.write_all(b"\"p1\"}]").await.expect("write tail");
            // Keep `server` alive so no EOF is ever delivered to `client`.
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let (code, body) =
            tokio::time::timeout(Duration::from_millis(500), read_http_response(&mut client))
                .await
                .expect("must not wait for EOF")
                .expect("parse HTTP response");
        writer.abort();
        assert_eq!(code, 200);
        assert_eq!(body, br#"[{"id":"p1"}]"#);
    }

    #[test]
    fn decodes_chunked_and_rejects_missing_terminal_zero_block() {
        let ok_raw =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n[1,2\r\n2\r\n,3\r\n1\r\n]\r\n0\r\n\r\n";
        let (code, body) = parse_http_response(ok_raw).expect("valid chunked");
        assert_eq!(code, 200);
        assert_eq!(body, b"[1,2,3]");

        assert!(decode_chunked(b"4\r\n[1,2\r\n").is_err());
        assert!(decode_chunked(b"4\r\n[1,2\r\n0\r\n").is_err());
        let bad_raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n[1,2\r\n";
        assert!(parse_http_response(bad_raw).is_err());
    }
}
