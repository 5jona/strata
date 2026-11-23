//! Minimal safe `tmpfs /dev` provisioning (no `DEVTMPFS`) and explicit full host-runtime mode.

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, symlink};
use std::path::Path;

use crate::error::{SysError, SysResult};
use crate::mount;

struct UmaskGuard {
    prev: libc::mode_t,
}

impl UmaskGuard {
    fn zero() -> Self {
        // SAFETY: `umask(0)` is always safe in single-threaded stage-2 setup and restored on drop.
        let prev = unsafe { libc::umask(0) };
        Self { prev }
    }
}

impl Drop for UmaskGuard {
    fn drop(&mut self) {
        // SAFETY: Restores the process umask saved in `UmaskGuard::zero`.
        unsafe {
            libc::umask(self.prev);
        }
    }
}

/// Core safe character device nodes created inside container `tmpfs /dev`.
/// Raw storage block devices (`/dev/block/*`) are intentionally excluded by default.
const SAFE_CHAR_DEVS: &[(&str, u32, u32, u32)] = &[
    ("null", 1, 3, 0o666),
    ("zero", 1, 5, 0o666),
    ("full", 1, 7, 0o666),
    ("random", 1, 8, 0o666),
    ("urandom", 1, 9, 0o666),
    ("tty", 5, 0, 0o666),
    ("ptmx", 5, 2, 0o666),
];

/// Standard `/dev` stdio symlinks pointing to `/proc/self/fd`.
const DEV_SYMLINKS: &[(&str, &str)] = &[
    ("fd", "/proc/self/fd"),
    ("stdin", "/proc/self/fd/0"),
    ("stdout", "/proc/self/fd/1"),
    ("stderr", "/proc/self/fd/2"),
];

/// Sets up standard isolated `/proc`, `/sys`, minimal `tmpfs /dev`, `mode=1777` `tmpfs /dev/shm`,
/// bind-mounted `/dev/pts`, and any validated `extra_char_devs`.
pub fn setup_isolated_virtual_fs(rootfs: &Path, extra_char_devs: &[String]) -> SysResult<()> {
    let proc_dir = rootfs.join("proc");
    let sys_dir = rootfs.join("sys");
    let dev_dir = rootfs.join("dev");
    ensure_dir(&proc_dir)?;
    ensure_dir(&sys_dir)?;
    ensure_dir(&dev_dir)?;

    mount::mount(
        Some(Path::new("proc")),
        &proc_dir,
        Some("proc"),
        libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV,
        None,
    )?;

    // Mount fresh sysfs when allowed; fall back to read-only bind of host /sys on restrictive kernels.
    if mount::mount(
        Some(Path::new("sysfs")),
        &sys_dir,
        Some("sysfs"),
        libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV,
        None,
    )
    .is_err()
    {
        mount::mount(
            Some(Path::new("/sys")),
            &sys_dir,
            None,
            libc::MS_BIND | libc::MS_REC,
            None,
        )?;
        let existing_flags = crate::procfs::read_mount_flags(&sys_dir).unwrap_or(0);
        mount::mount(
            None,
            &sys_dir,
            None,
            existing_flags | libc::MS_REMOUNT | libc::MS_BIND | libc::MS_RDONLY,
            None,
        )?;
    }

    // Minimal tmpfs /dev (must NOT pass MS_NODEV so created character devices remain usable).
    mount::mount(
        Some(Path::new("tmpfs")),
        &dev_dir,
        Some("tmpfs"),
        libc::MS_NOSUID,
        Some("mode=0755,size=16m"),
    )?;

    {
        let _umask = UmaskGuard::zero();
        for &(name, major, minor, mode) in SAFE_CHAR_DEVS {
            let dev_path = dev_dir.join(name);
            let rdev = libc::makedev(major, minor);
            mknod_char(&dev_path, mode, rdev)?;
        }

        for &(link_name, target) in DEV_SYMLINKS {
            let link_path = dev_dir.join(link_name);
            let _ = fs::remove_file(&link_path);
            symlink(target, &link_path).map_err(|e| SysError::path_io("symlink", &link_path, e))?;
        }

        for extra in extra_char_devs {
            if !expose_host_char_dev_optional(&dev_dir, extra)? {
                eprintln!("strata: optional host char dev '/dev/{extra}' not found, skipping");
            }
        }
    }

    setup_dev_shm_and_pts(&dev_dir)?;
    Ok(())
}

/// Sets up explicit full host-runtime mode (`debug` or `hostRuntime=true`), bind-mounting
/// host `/proc`, `/sys`, and `/dev` recursively while ensuring writable `mode=1777` `/dev/shm`.
pub fn setup_host_runtime_virtual_fs(rootfs: &Path) -> SysResult<()> {
    for name in ["proc", "sys", "dev"] {
        let host = Path::new("/").join(name);
        let dst = rootfs.join(name);
        ensure_dir(&dst)?;
        mount::mount(Some(&host), &dst, None, libc::MS_BIND | libc::MS_REC, None)?;
    }

    let shm_dir = rootfs.join("dev/shm");
    ensure_dir(&shm_dir)?;
    let _ = mount::mount(
        Some(Path::new("tmpfs")),
        &shm_dir,
        Some("tmpfs"),
        libc::MS_NOSUID | libc::MS_NODEV,
        Some("mode=1777"),
    );
    Ok(())
}

fn setup_dev_shm_and_pts(dev_dir: &Path) -> SysResult<()> {
    let shm_dir = dev_dir.join("shm");
    ensure_dir(&shm_dir)?;
    mount::mount(
        Some(Path::new("tmpfs")),
        &shm_dir,
        Some("tmpfs"),
        libc::MS_NOSUID | libc::MS_NODEV,
        Some("mode=1777"),
    )?;

    let pts_dir = dev_dir.join("pts");
    ensure_dir(&pts_dir)?;
    if Path::new("/dev/pts").is_dir() {
        mount::mount(
            Some(Path::new("/dev/pts")),
            &pts_dir,
            None,
            libc::MS_BIND,
            None,
        )?;
    }
    Ok(())
}

/// Exposes a host character device node inside `container_dev_dir`.
///
/// Returns `Ok(true)` when created, `Ok(false)` when the host node does not exist
/// (`ErrorKind::NotFound`), and `Err` on non-character-device targets or real syscall failures.
pub fn expose_host_char_dev_optional(
    container_dev_dir: &Path,
    relative_name: &str,
) -> SysResult<bool> {
    expose_char_dev_from_root(Path::new("/dev"), container_dev_dir, relative_name)
}

fn expose_char_dev_from_root(
    host_dev_root: &Path,
    container_dev_dir: &Path,
    relative_name: &str,
) -> SysResult<bool> {
    let host_dev = host_dev_root.join(relative_name);
    let meta = match fs::metadata(&host_dev) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(SysError::path_io("stat char dev", &host_dev, e)),
    };
    if !meta.file_type().is_char_device() {
        return Err(SysError::InvalidArgument {
            op: "expose_host_char_dev",
            detail: format!("'{}' is not a character device", host_dev.display()),
        });
    }
    let dst = container_dev_dir.join(relative_name);
    if let Some(parent) = dst.parent() {
        ensure_dir(parent)?;
    }
    let perms = meta.mode() & 0o666;
    mknod_char(&dst, if perms == 0 { 0o660 } else { perms }, meta.rdev())?;
    Ok(true)
}

fn mknod_char(path: &Path, perms: u32, rdev: libc::dev_t) -> SysResult<()> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| SysError::InvalidCString(path.to_string_lossy().into_owned()))?;
    let mode = (libc::S_IFCHR | (perms as libc::mode_t)) as libc::mode_t;
    // SAFETY: `c_path` is a valid NUL-terminated path and `mode` specifies `S_IFCHR`.
    let rc = unsafe { libc::mknod(c_path.as_ptr(), mode, rdev) };
    if rc < 0 {
        return Err(SysError::path_os("mknod", path));
    }
    Ok(())
}

fn ensure_dir(path: &Path) -> SysResult<()> {
    fs::create_dir_all(path).map_err(|e| SysError::path_io("mkdir", path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_char_dev_skips_missing_and_rejects_regular_files() {
        let base = std::env::temp_dir().join(format!("strata-devfs-test-{}", std::process::id()));
        let host_root = base.join("host_dev");
        let dst_root = base.join("dst_dev");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&host_root).expect("create host_dev");
        fs::create_dir_all(&dst_root).expect("create dst_dev");

        let missing = expose_char_dev_from_root(&host_root, &dst_root, "uinput")
            .expect("missing optional dev should return Ok(false)");
        assert!(!missing);

        let reg_file = host_root.join("not_a_chardev");
        fs::write(&reg_file, b"plain").expect("write regular file");
        let err = expose_char_dev_from_root(&host_root, &dst_root, "not_a_chardev");
        assert!(matches!(
            err,
            Err(SysError::InvalidArgument {
                op: "expose_host_char_dev",
                ..
            })
        ));

        let _ = fs::remove_dir_all(&base);
    }
}
