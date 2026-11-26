//! Direct `/dev/loop-control` and loop block-device ioctl management.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::error::{SysError, SysResult};
use crate::mount::read_proc_mounts;

const LOOP_CTL_GET_FREE: u64 = 0x4C82;
const LOOP_SET_FD: u64 = 0x4C00;
const LOOP_CLR_FD: u64 = 0x4C01;
const LOOP_SET_STATUS64: u64 = 0x4C04;
const LOOP_GET_STATUS64: u64 = 0x4C05;
const LO_FLAGS_AUTOCLEAR: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy)]
struct LoopInfo64 {
    lo_device: u64,
    lo_inode: u64,
    lo_rdevice: u64,
    lo_offset: u64,
    lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; 64],
    lo_crypt_name: [u8; 64],
    lo_encrypt_key: [u8; 32],
    lo_init: [u64; 2],
}

impl Default for LoopInfo64 {
    fn default() -> Self {
        Self {
            lo_device: 0,
            lo_inode: 0,
            lo_rdevice: 0,
            lo_offset: 0,
            lo_sizelimit: 0,
            lo_number: 0,
            lo_encrypt_type: 0,
            lo_encrypt_key_size: 0,
            lo_flags: 0,
            lo_file_name: [0; 64],
            lo_crypt_name: [0; 64],
            lo_encrypt_key: [0; 32],
            lo_init: [0; 2],
        }
    }
}

/// Active loop device attachment held open across `mount(2)`.
#[derive(Debug)]
pub struct LoopAttachment {
    pub device_path: PathBuf,
    _loop_fd: OwnedFd,
}

/// Resolves the Android (`/dev/block/loopN`) or standard Linux (`/dev/loopN`) device node path.
#[must_use]
pub fn loop_device_path(index: u32) -> PathBuf {
    let android_path = PathBuf::from(format!("/dev/block/loop{index}"));
    if android_path.exists() {
        return android_path;
    }
    PathBuf::from(format!("/dev/loop{index}"))
}

/// Returns `(st_dev, st_ino)` for the backing image file.
pub fn file_dev_inode(path: &Path) -> SysResult<(u64, u64)> {
    let meta = std::fs::metadata(path).map_err(|e| SysError::path_io("stat", path, e))?;
    Ok((meta.dev(), meta.ino()))
}

/// Scans active loop devices once and returns a `(st_dev, st_ino) -> Vec<PathBuf>` backing map.
#[must_use]
pub fn scan_loop_backing_map() -> HashMap<(u64, u64), Vec<PathBuf>> {
    let mut map: HashMap<(u64, u64), Vec<PathBuf>> = HashMap::new();
    for dev_path in enumerate_loop_nodes() {
        let Ok(file) = OpenOptions::new().read(true).open(&dev_path) else {
            continue;
        };
        if let Ok(info) = get_loop_status(&file)
            && info.lo_inode != 0
        {
            map.entry((info.lo_device, info.lo_inode))
                .or_default()
                .push(dev_path);
        }
    }
    for paths in map.values_mut() {
        paths.sort();
        paths.dedup();
    }
    map
}

/// Scans active loop devices and returns all device paths currently backed by `image_path`.
pub fn find_image_loops(image_path: &Path) -> SysResult<Vec<PathBuf>> {
    if !image_path.exists() {
        return Ok(Vec::new());
    }
    let (target_dev, target_ino) = file_dev_inode(image_path)?;
    let mut matches = Vec::new();
    for dev_path in enumerate_loop_nodes() {
        let Ok(file) = OpenOptions::new().read(true).open(&dev_path) else {
            continue;
        };
        if let Ok(info) = get_loop_status(&file)
            && info.lo_inode == target_ino
            && info.lo_device == target_dev
        {
            matches.push(dev_path);
        }
    }
    matches.sort();
    matches.dedup();
    Ok(matches)
}

/// Rejects mounting `image_path` if any loop device backing the same `(st_dev, st_ino)`
/// is already mounted at a directory other than `allowed_mount_dir`.
pub fn ensure_image_not_mounted_elsewhere(
    image_path: &Path,
    allowed_mount_dir: &Path,
) -> SysResult<()> {
    let loops = find_image_loops(image_path)?;
    if loops.is_empty() {
        return Ok(());
    }
    let mounts = read_proc_mounts()?;
    let allowed = allowed_mount_dir.to_string_lossy();
    for entry in &mounts {
        if loops.iter().any(|l| l.to_string_lossy() == entry.source)
            && entry.target != allowed.as_ref()
        {
            return Err(SysError::ImageBusy {
                image: image_path.to_path_buf(),
                existing_mount: PathBuf::from(&entry.target),
                loop_dev: PathBuf::from(&entry.source),
            });
        }
    }
    Ok(())
}

/// Allocates a free loop device via `/dev/loop-control` and binds `image_path` read-write.
pub fn attach_loop(image_path: &Path) -> SysResult<LoopAttachment> {
    let backing = OpenOptions::new()
        .read(true)
        .write(true)
        .open(image_path)
        .map_err(|e| SysError::path_io("open image", image_path, e))?;

    let control = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/loop-control")
        .map_err(|e| SysError::path_io("open loop-control", "/dev/loop-control", e))?;

    for _attempt in 0..8 {
        // SAFETY: `control` is a valid open fd for `/dev/loop-control`; `LOOP_CTL_GET_FREE`
        // takes no extra pointer arguments and returns the non-negative loop index or -1.
        let idx = unsafe { libc::ioctl(control.as_raw_fd(), LOOP_CTL_GET_FREE as libc::Ioctl) };
        if idx < 0 {
            return Err(SysError::last_os("LOOP_CTL_GET_FREE"));
        }
        let dev_path = loop_device_path(idx as u32);
        let loop_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&dev_path)
            .map_err(|e| SysError::path_io("open loop device", &dev_path, e))?;

        // SAFETY: Both `loop_file` and `backing` are valid open file descriptors.
        let rc = unsafe {
            libc::ioctl(
                loop_file.as_raw_fd(),
                LOOP_SET_FD as libc::Ioctl,
                backing.as_raw_fd(),
            )
        };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EBUSY) {
                continue;
            }
            return Err(SysError::path_io("LOOP_SET_FD", &dev_path, err));
        }

        let mut info = LoopInfo64 {
            lo_flags: LO_FLAGS_AUTOCLEAR,
            ..LoopInfo64::default()
        };
        let name_bytes = image_path.as_os_str().as_encoded_bytes();
        let copy_len = name_bytes.len().min(info.lo_file_name.len() - 1);
        info.lo_file_name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

        // SAFETY: `loop_file` is bound to `backing` and `info` points to initialized `LoopInfo64`.
        let status_rc = unsafe {
            libc::ioctl(
                loop_file.as_raw_fd(),
                LOOP_SET_STATUS64 as libc::Ioctl,
                &info as *const LoopInfo64,
            )
        };
        if status_rc < 0 {
            let err = io::Error::last_os_error();
            let _ = detach_loop_fd(loop_file.as_raw_fd());
            return Err(SysError::path_io("LOOP_SET_STATUS64", &dev_path, err));
        }

        return Ok(LoopAttachment {
            device_path: dev_path,
            _loop_fd: OwnedFd::from(loop_file),
        });
    }

    Err(SysError::Io {
        op: "attach_loop",
        source: io::Error::from_raw_os_error(libc::EBUSY),
    })
}

/// Detaches a single loop device path via `LOOP_CLR_FD`.
pub fn detach_loop(dev_path: &Path) -> SysResult<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dev_path)
        .or_else(|_| OpenOptions::new().read(true).open(dev_path))
        .map_err(|e| SysError::path_io("open loop for detach", dev_path, e))?;
    detach_loop_fd(file.as_raw_fd()).map_err(|e| SysError::path_io("LOOP_CLR_FD", dev_path, e))
}

/// Detaches all loop devices currently backing `image_path`, returning the count detached.
pub fn detach_image_loops(image_path: &Path) -> SysResult<usize> {
    let loops = find_image_loops(image_path)?;
    let mut detached = 0;
    for dev in loops {
        if detach_loop(&dev).is_ok() {
            detached += 1;
        }
    }
    Ok(detached)
}

fn detach_loop_fd(fd: libc::c_int) -> io::Result<()> {
    // SAFETY: `fd` is an open loop block device descriptor; `LOOP_CLR_FD` takes no pointer args.
    let rc = unsafe { libc::ioctl(fd, LOOP_CLR_FD as libc::Ioctl, 0) };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENXIO) {
            return Ok(());
        }
        return Err(err);
    }
    Ok(())
}

fn get_loop_status(file: &File) -> io::Result<LoopInfo64> {
    let mut info = LoopInfo64::default();
    // SAFETY: `file` is a valid loop device descriptor and `info` is a writable `LoopInfo64`.
    let rc = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            LOOP_GET_STATUS64 as libc::Ioctl,
            &mut info as *mut LoopInfo64,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

fn enumerate_loop_nodes() -> Vec<PathBuf> {
    let mut nodes = Vec::new();
    for dir in ["/dev/block", "/dev"] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            if let Some(rest) = name_str.strip_prefix("loop")
                && !rest.is_empty()
                && rest.bytes().all(|b| b.is_ascii_digit())
            {
                nodes.push(entry.path());
            }
        }
        if !nodes.is_empty() {
            break;
        }
    }
    nodes
}
