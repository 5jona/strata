//! Blocking single-pass rootfs provisioning pipeline (tar extraction or raw ext4 image import).

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use strata_protocol::{ContainerConfig, StrataConfig};
use strata_runtime::fs_atomic::ensure_dir_mode;
use strata_runtime::provision::archive::{ArchiveFormat, extract_archive_streaming};
use strata_runtime::provision::provision_container_rootfs;
use strata_runtime::provision::resize::create_sparse_ext4;
use strata_runtime::{RuntimeError, RuntimeResult, storage};
use strata_sys::{loopdev, mount};

use crate::jobs::raw_image::provision_from_raw_image;

pub(crate) const MAX_CONTAINER_SIZE_MB: u64 = 524_288; // 512 GiB
const MAX_RAW_IMAGE_BYTES: u64 = MAX_CONTAINER_SIZE_MB * 1024 * 1024;

/// Allocates a free SSH port for a new or cloned container avoiding both configured and reserved ports.
#[must_use]
pub(crate) fn allocate_ssh_port(
    cfg: &StrataConfig,
    reserved_ports: &HashSet<u16>,
    requested: Option<u16>,
) -> u16 {
    if let Some(p) = requested
        && p > 0
    {
        return p;
    }
    let mut candidate: u16 = 2222;
    while reserved_ports.contains(&candidate)
        || cfg
            .containers
            .values()
            .any(|c| c.ssh.enabled && c.ssh.port == candidate)
    {
        candidate = candidate.saturating_add(1);
    }
    candidate
}

/// Executes single-pass streaming provisioning from an archive or raw image reader into `cfg.image_path`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn provision_from_reader<R, F>(
    reader: R,
    format: ArchiveFormat,
    expected_sha256: Option<&str>,
    container_id: &str,
    cfg: &ContainerConfig,
    size_bytes: u64,
    ssh_keys: &[String],
    module_dir: Option<&Path>,
    cancel: Arc<AtomicBool>,
    mut on_progress: F,
) -> RuntimeResult<()>
where
    R: Read + Send,
    F: FnMut(&str, u64) + Send,
{
    let image_path = Path::new(&cfg.image_path);
    let mount_dir = Path::new(&cfg.mount_dir);

    let outcome = match format {
        ArchiveFormat::TarGz
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst
        | ArchiveFormat::Tar => provision_from_tar(
            reader,
            format,
            expected_sha256,
            container_id,
            cfg,
            size_bytes,
            ssh_keys,
            module_dir,
            cancel,
            &mut on_progress,
        ),
        ArchiveFormat::RawImg | ArchiveFormat::RawImgGz => provision_from_raw_image(
            reader,
            format == ArchiveFormat::RawImgGz,
            expected_sha256,
            container_id,
            cfg,
            size_bytes.min(MAX_RAW_IMAGE_BYTES),
            ssh_keys,
            module_dir,
            cancel,
            &mut on_progress,
        ),
    };

    if outcome.is_err() {
        let _ = storage::teardown_container(container_id, cfg, None, None);
        let _ = fs::remove_file(image_path);
        let _ = fs::remove_dir(mount_dir);
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
fn provision_from_tar<R, F>(
    reader: R,
    format: ArchiveFormat,
    expected_sha256: Option<&str>,
    container_id: &str,
    cfg: &ContainerConfig,
    size_bytes: u64,
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
    let mount_dir = Path::new(&cfg.mount_dir);

    on_progress("formatting_ext4", 0);
    create_sparse_ext4(image_path, size_bytes)?;
    ensure_dir_mode(mount_dir, 0o755)?;

    let attachment = loopdev::attach_loop(image_path).map_err(RuntimeError::from_sys)?;
    let dev_path = attachment.device_path.clone();
    let mount_flags = if cfg.noatime {
        libc::MS_NOATIME | libc::MS_NODIRATIME
    } else {
        libc::MS_RELATIME
    };
    if let Err(err) = mount::mount(Some(&dev_path), mount_dir, Some("ext4"), mount_flags, None) {
        let _ = loopdev::detach_loop(&dev_path);
        return Err(RuntimeError::from_sys(err));
    }
    drop(attachment);

    on_progress("extracting", 0);
    let extract_res =
        extract_archive_streaming(reader, format, mount_dir, expected_sha256, cancel, |done| {
            on_progress("extracting", done);
        });
    if let Err(err) = extract_res {
        let _ = storage::teardown_container(container_id, cfg, None, None);
        return Err(err);
    }

    on_progress(
        "provisioning",
        extract_res.as_ref().map_or(0, |r| r.bytes_read),
    );
    let prov_res = provision_container_rootfs(mount_dir, cfg, ssh_keys, module_dir);
    let _ = storage::teardown_container(container_id, cfg, None, None);
    prov_res
}
