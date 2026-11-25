//! Pure-Rust streaming archive extraction (`.tar.gz`, `.tar.xz`, `.tar.zst`, `.tar`)
//! with SHA256 verification and zero double disk copies.

mod unpack;

use std::fmt::Write as _;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

use crate::error::{RuntimeError, RuntimeResult};
use crate::fs_atomic::ensure_dir_mode;
use unpack::unpack_tar_safe;

/// Supported rootfs archive or raw image formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    TarGz,
    TarXz,
    TarZst,
    Tar,
    RawImg,
    RawImgGz,
}

/// Summary returned after streaming archive extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractReport {
    pub bytes_read: u64,
    pub entries_extracted: u64,
    pub sha256_hex: String,
}

/// Detects archive/image format from filename hint and optional leading magic bytes.
#[must_use]
pub fn detect_archive_format(filename: &str, magic: &[u8]) -> Option<ArchiveFormat> {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".img.gz") {
        return Some(ArchiveFormat::RawImgGz);
    }
    if lower.ends_with(".img") || lower.ends_with(".ext4") {
        return Some(ArchiveFormat::RawImg);
    }
    if magic.len() >= 6 && magic[..6] == [0xFD, b'7', b'z', b'X', b'Z', 0x00] {
        return Some(ArchiveFormat::TarXz);
    }
    if magic.len() >= 4 && magic[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        return Some(ArchiveFormat::TarZst);
    }
    if magic.len() >= 2 && magic[..2] == [0x1F, 0x8B] {
        return Some(ArchiveFormat::TarGz);
    }
    if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        Some(ArchiveFormat::TarXz)
    } else if lower.ends_with(".tar.zst") || lower.ends_with(".tzst") {
        Some(ArchiveFormat::TarZst)
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(ArchiveFormat::TarGz)
    } else if lower.ends_with(".tar") {
        Some(ArchiveFormat::Tar)
    } else {
        None
    }
}

struct HashingReader<R, F> {
    inner: R,
    hasher: Sha256,
    bytes_read: u64,
    last_reported: u64,
    cancel: Arc<AtomicBool>,
    on_progress: F,
}

impl<R: Read, F: FnMut(u64)> HashingReader<R, F> {
    fn new(inner: R, cancel: Arc<AtomicBool>, on_progress: F) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes_read: 0,
            last_reported: 0,
            cancel,
            on_progress,
        }
    }

    fn drain_and_finish(mut self) -> io::Result<(u64, String)> {
        let mut sink = [0u8; 8192];
        while self.read(&mut sink)? > 0 {}
        (self.on_progress)(self.bytes_read);
        let digest = self.hasher.finalize();
        let mut hex = String::with_capacity(64);
        for b in digest {
            let _ = write!(&mut hex, "{b:02x}");
        }
        Ok((self.bytes_read, hex))
    }
}

impl<R: Read, F: FnMut(u64)> Read for HashingReader<R, F> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "archive job cancelled",
            ));
        }
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
            self.bytes_read = self.bytes_read.saturating_add(n as u64);
            if self.bytes_read.saturating_sub(self.last_reported) >= 256 * 1024 {
                self.last_reported = self.bytes_read;
                (self.on_progress)(self.bytes_read);
            }
        }
        Ok(n)
    }
}

/// Streams a compressed or plain tar archive directly into `dest_dir` without double disk copies.
pub fn extract_archive_streaming<R, F>(
    reader: R,
    format: ArchiveFormat,
    dest_dir: &Path,
    expected_sha256: Option<&str>,
    cancel: Arc<AtomicBool>,
    on_progress: F,
) -> RuntimeResult<ExtractReport>
where
    R: Read + Send,
    F: FnMut(u64) + Send,
{
    ensure_dir_mode(dest_dir, 0o755)?;
    let hashing = HashingReader::new(reader, Arc::clone(&cancel), on_progress);

    let (entries_extracted, bytes_read, sha256_hex) = match format {
        ArchiveFormat::Tar => {
            let mut archive = tar::Archive::new(hashing);
            let count = unpack_tar_safe(&mut archive, dest_dir, &cancel)?;
            let (bytes, sha) =
                archive
                    .into_inner()
                    .drain_and_finish()
                    .map_err(|source| RuntimeError::Io {
                        op: "finalize tar stream",
                        source,
                    })?;
            (count, bytes, sha)
        }
        ArchiveFormat::TarGz => {
            let mut archive = tar::Archive::new(GzDecoder::new(hashing));
            let count = unpack_tar_safe(&mut archive, dest_dir, &cancel)?;
            let (bytes, sha) = archive
                .into_inner()
                .into_inner()
                .drain_and_finish()
                .map_err(|source| RuntimeError::Io {
                    op: "finalize gzip stream",
                    source,
                })?;
            (count, bytes, sha)
        }
        ArchiveFormat::TarZst => {
            let decoder = ruzstd::StreamingDecoder::new(hashing)
                .map_err(|e| RuntimeError::InvalidArgument(format!("init zstd decoder: {e}")))?;
            let mut archive = tar::Archive::new(decoder);
            let count = unpack_tar_safe(&mut archive, dest_dir, &cancel)?;
            let (bytes, sha) = archive
                .into_inner()
                .into_inner()
                .drain_and_finish()
                .map_err(|source| RuntimeError::Io {
                    op: "finalize zstd stream",
                    source,
                })?;
            (count, bytes, sha)
        }
        ArchiveFormat::TarXz => extract_tar_xz_streaming(hashing, dest_dir, &cancel)?,
        ArchiveFormat::RawImg | ArchiveFormat::RawImgGz => {
            return Err(RuntimeError::InvalidArgument(
                "raw disk image format cannot be extracted as tar archive".to_owned(),
            ));
        }
    };

    if let Some(expected) = expected_sha256
        && !expected.trim().is_empty()
        && !sha256_hex.eq_ignore_ascii_case(expected.trim())
    {
        return Err(RuntimeError::InvalidArgument(format!(
            "SHA256 mismatch: expected {}, got {sha256_hex}",
            expected.trim()
        )));
    }

    Ok(ExtractReport {
        bytes_read,
        entries_extracted,
        sha256_hex,
    })
}

fn extract_tar_xz_streaming<R, F>(
    hashing: HashingReader<R, F>,
    dest_dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> RuntimeResult<(u64, u64, String)>
where
    R: Read + Send,
    F: FnMut(u64) + Send,
{
    let (pipe_r, pipe_w) =
        std::os::unix::net::UnixStream::pair().map_err(|source| RuntimeError::Io {
            op: "create xz stream pipe",
            source,
        })?;

    std::thread::scope(|scope| {
        let decomp_handle = scope.spawn(move || -> RuntimeResult<(u64, String)> {
            let mut buf_reader = BufReader::with_capacity(64 * 1024, hashing);
            let mut writer = io::BufWriter::with_capacity(64 * 1024, pipe_w);
            lzma_rs::xz_decompress(&mut buf_reader, &mut writer)
                .map_err(|e| RuntimeError::InvalidArgument(format!("xz decompress failed: {e}")))?;
            writer.flush().map_err(|source| RuntimeError::Io {
                op: "flush xz stream",
                source,
            })?;
            let pipe_w = writer.into_inner().map_err(|e| RuntimeError::Io {
                op: "unwrap xz writer",
                source: e.into_error(),
            })?;
            let _ = pipe_w.shutdown(std::net::Shutdown::Write);
            buf_reader
                .into_inner()
                .drain_and_finish()
                .map_err(|source| RuntimeError::Io {
                    op: "finalize xz sha256",
                    source,
                })
        });

        let mut archive = tar::Archive::new(pipe_r);
        let unpack_res = unpack_tar_safe(&mut archive, dest_dir, cancel);
        drop(archive);
        let decomp_res = decomp_handle
            .join()
            .map_err(|_| RuntimeError::InvalidArgument("xz decoder thread panicked".to_owned()))?;
        let count = unpack_res?;
        let (bytes, sha) = decomp_res?;
        Ok((count, bytes, sha))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::fs;

    fn build_tar_bytes(entries: &[(&str, tar::EntryType, &[u8], Option<&str>)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut b = tar::Builder::new(&mut buf);
            for &(path, kind, data, link) in entries {
                let mut hdr = tar::Header::new_gnu();
                hdr.set_entry_type(kind);
                hdr.set_size(data.len() as u64);
                hdr.set_mode(0o644);
                if let Some(target) = link {
                    hdr.set_link_name(target).expect("set link");
                }
                if path.contains("..") {
                    let bytes = path.as_bytes();
                    hdr.as_gnu_mut().expect("gnu").name[..bytes.len()].copy_from_slice(bytes);
                    hdr.set_cksum();
                    b.append(&hdr, data).expect("append raw");
                } else {
                    hdr.set_cksum();
                    b.append_data(&mut hdr, path, data).expect("append");
                }
            }
            b.finish().expect("finish tar");
        }
        buf
    }

    #[test]
    fn extracts_tar_gz_and_rejects_zip_slip_and_symlink_escape() {
        let base = std::env::temp_dir().join(format!("strata-arch-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let good_dir = base.join("good");

        let raw_tar = build_tar_bytes(&[
            ("bin/sh", tar::EntryType::Regular, b"#!/bin/sh\n", None),
            ("dev/null", tar::EntryType::Regular, b"ignored", None),
        ]);
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(&raw_tar).expect("gz write");
        let gz_bytes = gz.finish().expect("gz finish");

        let report = extract_archive_streaming(
            &gz_bytes[..],
            ArchiveFormat::TarGz,
            &good_dir,
            None,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .expect("extract tar.gz");
        assert!(good_dir.join("bin/sh").is_file());
        assert!(!good_dir.join("dev/null").exists());
        assert_eq!(report.entries_extracted, 1);

        let evil_zipslip = build_tar_bytes(&[("../evil.txt", tar::EntryType::Regular, b"x", None)]);
        assert!(
            extract_archive_streaming(
                &evil_zipslip[..],
                ArchiveFormat::Tar,
                &base.join("zipslip"),
                None,
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .is_err()
        );

        let evil_sym = build_tar_bytes(&[
            ("esc", tar::EntryType::Symlink, b"", Some("/tmp")),
            ("esc/pwned", tar::EntryType::Regular, b"bad", None),
        ]);
        assert!(
            extract_archive_streaming(
                &evil_sym[..],
                ArchiveFormat::Tar,
                &base.join("sym"),
                None,
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .is_err()
        );

        let _ = fs::remove_dir_all(&base);
    }
}
