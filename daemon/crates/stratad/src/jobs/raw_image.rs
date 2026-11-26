//! Streaming raw `.img` / `.img.gz` import with wire-order SHA256 verification,
//! explicit decompressed size limits, and sparse hole preservation.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use strata_protocol::ContainerConfig;
use strata_protocol::constants::SELINUX_IMAGE_CONTEXT;
use strata_runtime::fs_atomic::ensure_dir_mode;
use strata_runtime::provision::provision_container_rootfs;
use strata_runtime::provision::resize::validate_ext4_magic;
use strata_runtime::{RuntimeError, RuntimeResult, storage};
use strata_sys::selinux;

const SPARSE_CHUNK_BYTES: usize = 64 * 1024;

struct WireHashReader<R> {
    inner: R,
    hasher: Sha256,
    wire_bytes: u64,
    cancel: Arc<AtomicBool>,
}

impl<R: Read> WireHashReader<R> {
    fn new(inner: R, cancel: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            wire_bytes: 0,
            cancel,
        }
    }

    fn drain_and_finish_hex(mut self) -> io::Result<(u64, String)> {
        let mut sink = [0u8; 8192];
        while self.read(&mut sink)? > 0 {}
        let mut hex = String::with_capacity(64);
        for b in self.hasher.finalize() {
            let _ = write!(&mut hex, "{b:02x}");
        }
        Ok((self.wire_bytes, hex))
    }
}

impl<R: Read> Read for WireHashReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "import cancelled",
            ));
        }
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
            self.wire_bytes = self.wire_bytes.saturating_add(n as u64);
        }
        Ok(n)
    }
}

/// Streams a raw `.img` or `.img.gz` into `image_path`, hashing wire bytes BEFORE decompression,
/// enforcing `max_decompressed_bytes`, and punching sparse holes for all-zero blocks.
pub(super) fn write_raw_image_stream<R, F>(
    reader: R,
    gzip_compressed: bool,
    image_path: &Path,
    max_decompressed_bytes: u64,
    expected_sha256: Option<&str>,
    cancel: Arc<AtomicBool>,
    on_progress: &mut F,
) -> RuntimeResult<u64>
where
    R: Read,
    F: FnMut(&str, u64),
{
    if let Some(parent) = image_path.parent() {
        ensure_dir_mode(parent, 0o755)?;
    }

    on_progress("writing_image", 0);
    let mut dst = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(image_path)
        .map_err(|source| RuntimeError::Io {
            op: "create imported image",
            source,
        })?;

    let wire_reader = WireHashReader::new(reader, Arc::clone(&cancel));
    let (wire_reader, written_bytes) = if gzip_compressed {
        let mut gz = GzDecoder::new(wire_reader);
        let written = copy_sparse_bounded(
            &mut gz,
            &mut dst,
            max_decompressed_bytes,
            &cancel,
            on_progress,
        )?;
        (gz.into_inner(), written)
    } else {
        let mut wr = wire_reader;
        let written = copy_sparse_bounded(
            &mut wr,
            &mut dst,
            max_decompressed_bytes,
            &cancel,
            on_progress,
        )?;
        (wr, written)
    };

    dst.set_len(written_bytes)
        .map_err(|source| RuntimeError::Io {
            op: "set final imported image length",
            source,
        })?;
    dst.sync_all().map_err(|source| RuntimeError::Io {
        op: "sync imported image",
        source,
    })?;
    drop(dst);

    let (wire_bytes, actual_hex) =
        wire_reader
            .drain_and_finish_hex()
            .map_err(|source| RuntimeError::Io {
                op: "finalize imported image sha256",
                source,
            })?;

    if let Some(expected) = expected_sha256
        && !expected.trim().is_empty()
        && !actual_hex.eq_ignore_ascii_case(expected.trim())
    {
        return Err(RuntimeError::InvalidArgument(format!(
            "SHA256 mismatch: expected {}, got {actual_hex}",
            expected.trim()
        )));
    }
    Ok(wire_bytes.max(written_bytes))
}

fn copy_sparse_bounded<R, F>(
    reader: &mut R,
    dst: &mut File,
    max_bytes: u64,
    cancel: &AtomicBool,
    on_progress: &mut F,
) -> RuntimeResult<u64>
where
    R: Read,
    F: FnMut(&str, u64),
{
    let mut buf = vec![0u8; SPARSE_CHUNK_BYTES];
    let mut total: u64 = 0;
    let mut last_reported: u64 = 0;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(RuntimeError::InvalidArgument("import cancelled".to_owned()));
        }
        let n = reader.read(&mut buf).map_err(|source| RuntimeError::Io {
            op: "read imported image stream",
            source,
        })?;
        if n == 0 {
            break;
        }
        let next_total = total.saturating_add(n as u64);
        if next_total > max_bytes {
            return Err(RuntimeError::InvalidArgument(format!(
                "decompressed image exceeds {max_bytes} byte limit"
            )));
        }
        let slice = &buf[..n];
        if slice.iter().all(|&b| b == 0) {
            dst.seek(SeekFrom::Current(n as i64))
                .map_err(|source| RuntimeError::Io {
                    op: "seek sparse hole in imported image",
                    source,
                })?;
        } else {
            dst.write_all(slice).map_err(|source| RuntimeError::Io {
                op: "write imported image",
                source,
            })?;
        }
        total = next_total;
        if total.saturating_sub(last_reported) >= 512 * 1024 {
            last_reported = total;
            on_progress("writing_image", total);
        }
    }
    Ok(total)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn provision_from_raw_image<R, F>(
    reader: R,
    gzip_compressed: bool,
    expected_sha256: Option<&str>,
    container_id: &str,
    cfg: &ContainerConfig,
    max_decompressed_bytes: u64,
    ssh_keys: &[String],
    module_dir: Option<&Path>,
    cancel: Arc<AtomicBool>,
    on_progress: &mut F,
) -> RuntimeResult<()>
where
    R: Read + Send,
    F: FnMut(&str, u64) + Send,
{
    let image_path = Path::new(&cfg.image_path);
    let bytes_done = write_raw_image_stream(
        reader,
        gzip_compressed,
        image_path,
        max_decompressed_bytes.max(64 * 1024 * 1024),
        expected_sha256,
        cancel,
        on_progress,
    )?;

    if !validate_ext4_magic(image_path)? {
        return Err(RuntimeError::InvalidArgument(
            "imported image is not a valid ext4 filesystem (missing 0xEF53 superblock magic)"
                .to_owned(),
        ));
    }

    let _ = fs::set_permissions(image_path, fs::Permissions::from_mode(0o600));
    let _ = selinux::set_file_context(image_path, SELINUX_IMAGE_CONTEXT);

    on_progress("provisioning", bytes_done);
    storage::mount_rootfs(cfg, module_dir)?;
    let prov_res = provision_container_rootfs(Path::new(&cfg.mount_dir), cfg, ssh_keys, module_dir);
    let dropbear_missing = cfg.ssh.enabled
        && cfg.init_command.is_empty()
        && !Path::new(&cfg.mount_dir)
            .join("usr/sbin/dropbear")
            .is_file();
    let _ = storage::teardown_container(container_id, cfg, None, None);
    prov_res?;
    if dropbear_missing {
        eprintln!(
            "stratad: imported image for '{container_id}' has enableSsh=true but /usr/sbin/dropbear is missing; disable ssh.enabled or install dropbear-bin before starting"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Cursor;

    #[test]
    fn img_gz_hashes_compressed_wire_bytes_enforces_limit_and_preserves_data() {
        let base = std::env::temp_dir().join(format!("strata-imggz-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("dir");
        let out_img = base.join("rootfs.img");

        let mut raw = vec![0u8; 196 * 1024];
        raw[1080] = 0x53;
        raw[1081] = 0xEF;
        raw[131_072..131_080].copy_from_slice(b"STRATA!!");

        let mut gz_bytes = Vec::new();
        {
            let mut enc = GzEncoder::new(&mut gz_bytes, Compression::fast());
            enc.write_all(&raw).expect("gz write");
            enc.finish().expect("gz finish");
        }
        let mut expected_hex = String::new();
        for b in Sha256::digest(&gz_bytes) {
            let _ = write!(&mut expected_hex, "{b:02x}");
        }

        let cancel = Arc::new(AtomicBool::new(false));
        write_raw_image_stream(
            Cursor::new(gz_bytes.clone()),
            true,
            &out_img,
            256 * 1024,
            Some(&expected_hex),
            Arc::clone(&cancel),
            &mut |_, _| {},
        )
        .expect("img.gz with matching wire sha256 must succeed");
        assert_eq!(fs::read(&out_img).expect("read out"), raw);
        assert!(validate_ext4_magic(&out_img).expect("ext4 magic"));

        let bomb_err = write_raw_image_stream(
            Cursor::new(gz_bytes),
            true,
            &out_img,
            64 * 1024,
            None,
            cancel,
            &mut |_, _| {},
        );
        assert!(bomb_err.is_err());

        let _ = fs::remove_dir_all(&base);
    }
}
