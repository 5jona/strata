//! Authorized SSH key inspection, deduplicated append, and standard OpenSSH `SHA256:` fingerprint removal.

use std::fmt::Write as _;
use std::io::ErrorKind;
use std::path::Path;

use sha2::{Digest, Sha256};
use strata_sys::rootfs_io::RootDir;

use crate::error::{RuntimeError, RuntimeResult};

const AUTHORIZED_KEYS_REL: &str = "root/.ssh/authorized_keys";

/// Reads authorized SSH public keys from `<mount_dir>/root/.ssh/authorized_keys` safely within root.
pub fn list_ssh_keys(mount_dir: &Path) -> RuntimeResult<Vec<String>> {
    let root = RootDir::open(mount_dir).map_err(RuntimeError::from_sys)?;
    match root.read_file(Path::new(AUTHORIZED_KEYS_REL)) {
        Ok(bytes) => {
            let content = String::from_utf8_lossy(&bytes);
            Ok(parse_ssh_keys(&content))
        }
        Err(strata_sys::SysError::PathIo { source, .. })
            if source.kind() == ErrorKind::NotFound =>
        {
            Ok(Vec::new())
        }
        Err(err) => Err(RuntimeError::from_sys(err)),
    }
}

/// Appends one or more newline-separated SSH public keys deduplicated and persisted atomically.
pub fn add_ssh_keys(mount_dir: &Path, input: &str) -> RuntimeResult<(Vec<String>, usize)> {
    let root = RootDir::open(mount_dir).map_err(RuntimeError::from_sys)?;
    root.ensure_dir(Path::new("root/.ssh"), 0o700)
        .map_err(RuntimeError::from_sys)?;

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
    root.write_atomic(Path::new(AUTHORIZED_KEYS_REL), rendered.as_bytes(), 0o600)
        .map_err(RuntimeError::from_sys)?;
    Ok((keys, added))
}

/// Removes matching SSH public keys by exact line, base64 key blob, or OpenSSH `SHA256:` fingerprint.
pub fn remove_ssh_keys(
    mount_dir: &Path,
    key_or_fingerprint: &str,
) -> RuntimeResult<(Vec<String>, usize)> {
    let needle = key_or_fingerprint.trim();
    if needle.is_empty() {
        return Err(RuntimeError::InvalidArgument(
            "fingerprintOrKey must not be empty".to_owned(),
        ));
    }
    let root = RootDir::open(mount_dir).map_err(RuntimeError::from_sys)?;
    let keys = list_ssh_keys(mount_dir)?;
    let mut kept = Vec::with_capacity(keys.len());
    let mut removed = 0;
    let needle_unpadded = needle.trim_end_matches('=');

    for key in keys {
        let blob = extract_ssh_key_blob(&key);
        let matched = key == needle
            || blob.is_some_and(|b| b == needle)
            || blob
                .and_then(openssh_sha256_fingerprint)
                .is_some_and(|fp| fp == needle || fp == needle_unpadded)
            || blob
                .map(legacy_blob_sha256_hex)
                .is_some_and(|fp| fp.eq_ignore_ascii_case(needle));
        if matched {
            removed += 1;
        } else {
            kept.push(key);
        }
    }
    if removed > 0 {
        let mut rendered = kept.join("\n");
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        root.write_atomic(Path::new(AUTHORIZED_KEYS_REL), rendered.as_bytes(), 0o600)
            .map_err(RuntimeError::from_sys)?;
    }
    Ok((kept, removed))
}

fn parse_ssh_keys(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect()
}

/// Extracts the base64 key-blob field from an `authorized_keys` line, skipping optional leading options.
fn extract_ssh_key_blob(key_line: &str) -> Option<&str> {
    let mut rest = key_line.trim();
    while !rest.is_empty() {
        let (tok, after) = split_authorized_keys_token(rest);
        if is_ssh_key_type(tok) {
            return after.split_whitespace().next();
        }
        rest = after.trim_start();
    }
    None
}

fn split_authorized_keys_token(input: &str) -> (&str, &str) {
    let bytes = input.as_bytes();
    let mut in_quotes = false;
    let mut escaped = false;
    for (idx, &b) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' && in_quotes {
            escaped = true;
            continue;
        }
        if b == b'"' {
            in_quotes = !in_quotes;
            continue;
        }
        if !in_quotes && b.is_ascii_whitespace() {
            return (&input[..idx], &input[idx..]);
        }
    }
    (input, "")
}

fn is_ssh_key_type(tok: &str) -> bool {
    tok.starts_with("ssh-")
        || tok.starts_with("ecdsa-sha2-")
        || tok.starts_with("sk-ssh-")
        || tok.starts_with("sk-ecdsa-")
}

/// Computes the standard OpenSSH `SHA256:<unpadded-base64(sha256(raw_key_blob))>` fingerprint.
pub(super) fn openssh_sha256_fingerprint(blob_b64: &str) -> Option<String> {
    let raw = base64_decode(blob_b64)?;
    let digest = Sha256::digest(&raw);
    Some(format!("SHA256:{}", base64_encode_nopad(&digest)))
}

fn legacy_blob_sha256_hex(blob: &str) -> String {
    let digest = Sha256::digest(blob.as_bytes());
    let mut hex = String::with_capacity(7 + 64);
    hex.push_str("SHA256:");
    for b in digest {
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.is_empty() || !clean.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity((clean.len() / 4) * 3);
    let (chunks, _) = clean.as_chunks::<4>();
    for chunk in chunks {
        let a = b64_val(chunk[0])?;
        let b = b64_val(chunk[1])?;
        out.push((a << 2) | (b >> 4));
        if chunk[2] == b'=' {
            if chunk[3] != b'=' {
                return None;
            }
            break;
        }
        let c = b64_val(chunk[2])?;
        out.push((b << 4) | (c >> 2));
        if chunk[3] == b'=' {
            break;
        }
        let d = b64_val(chunk[3])?;
        out.push((c << 6) | d);
    }
    Some(out)
}

fn b64_val(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn base64_encode_nopad(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() * 4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn matches_standard_openssh_sha256_fingerprint_and_leading_options() {
        let base = std::env::temp_dir().join(format!("strata-ssh-fp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("base");

        // Real ssh-ed25519 key payload: raw bytes = b"\x00\x00\x00\x0bssh-ed25519\x00\x00\x00\x04test"
        let raw_blob = b"\x00\x00\x00\x0bssh-ed25519\x00\x00\x00\x04test";
        let b64_blob = format!("{}=", base64_encode_nopad(raw_blob));
        assert_eq!(base64_decode(&b64_blob).as_deref(), Some(&raw_blob[..]));

        let line = format!("command=\"/bin/echo hi\",no-pty ssh-ed25519 {b64_blob} ops@host");
        let fp = openssh_sha256_fingerprint(&b64_blob).expect("fp");
        assert!(fp.starts_with("SHA256:"));
        assert!(!fp.ends_with('='));

        let (keys, added) = add_ssh_keys(&base, &line).expect("add");
        assert_eq!(added, 1);
        assert_eq!(keys.len(), 1);

        let (remaining, removed) = remove_ssh_keys(&base, &fp).expect("remove by OpenSSH SHA256");
        assert_eq!(removed, 1);
        assert!(remaining.is_empty());

        let _ = fs::remove_dir_all(&base);
    }
}
