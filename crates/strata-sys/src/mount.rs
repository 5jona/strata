//! Mount, unmount, `pivot_root`, `/proc/mounts` parsing, immutable flag clearing, and `FITRIM`.

use std::ffi::{CString, OsStr};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::error::{SysError, SysResult};

const FITRIM_IOCTL: u64 = 0xC018_5879;
const FS_IOC_GETFLAGS: u64 = 0x8008_6601;
const FS_IOC_SETFLAGS: u64 = 0x4008_6602;
const FS_IMMUTABLE_FL: libc::c_long = 0x0000_0010;

#[repr(C)]
struct FsTrimRange {
    start: u64,
    len: u64,
    minlen: u64,
}

/// Parsed line from `/proc/mounts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub source: String,
    pub target: String,
    pub fstype: String,
    pub options: String,
}

/// Parses `/proc/mounts` text into structured entries, unescaping standard octal sequences.
#[must_use]
pub fn parse_proc_mounts(content: &str) -> Vec<MountEntry> {
    let mut entries = Vec::new();
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let Some(source) = parts.next() else {
            continue;
        };
        let Some(target) = parts.next() else {
            continue;
        };
        let Some(fstype) = parts.next() else {
            continue;
        };
        let Some(options) = parts.next() else {
            continue;
        };
        entries.push(MountEntry {
            source: unescape_mount_field(source),
            target: unescape_mount_field(target),
            fstype: fstype.to_owned(),
            options: options.to_owned(),
        });
    }
    entries
}

/// Reads and parses `/proc/mounts`.
pub fn read_proc_mounts() -> SysResult<Vec<MountEntry>> {
    let raw = std::fs::read_to_string("/proc/mounts")
        .map_err(|e| SysError::path_io("read", "/proc/mounts", e))?;
    Ok(parse_proc_mounts(&raw))
}

/// Returns true if `mount_dir` currently has an `ext4` mount in `/proc/mounts`.
pub fn is_ext4_mounted(mount_dir: &Path) -> SysResult<bool> {
    let mounts = read_proc_mounts()?;
    let target = mount_dir.to_string_lossy();
    Ok(mounts
        .iter()
        .any(|m| m.target == target.as_ref() && m.fstype == "ext4"))
}

/// Returns all mount targets equal to or nested beneath `root_dir`, sorted deepest-first.
#[must_use]
pub fn select_mount_tree(entries: &[MountEntry], root_dir: &Path) -> Vec<PathBuf> {
    let root = root_dir.to_string_lossy();
    let trimmed = root.trim_end_matches('/');
    let prefix = format!("{trimmed}/");
    let mut targets: Vec<PathBuf> = entries
        .iter()
        .filter(|m| m.target == trimmed || m.target.starts_with(&prefix))
        .map(|m| PathBuf::from(&m.target))
        .collect();
    targets.sort_by(|a, b| b.as_os_str().len().cmp(&a.as_os_str().len()).then(b.cmp(a)));
    targets.dedup();
    targets
}

/// Returns all live mount targets under `root_dir`, sorted deepest-first.
pub fn mount_points_under(root_dir: &Path) -> SysResult<Vec<PathBuf>> {
    let mounts = read_proc_mounts()?;
    Ok(select_mount_tree(&mounts, root_dir))
}

/// Calls `mount(2)` with optional source, fstype, flags, and data strings.
pub fn mount(
    source: Option<&Path>,
    target: &Path,
    fstype: Option<&str>,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> SysResult<()> {
    let c_src = source.map(|p| os_to_cstring(p.as_os_str())).transpose()?;
    let c_tgt = os_to_cstring(target.as_os_str())?;
    let c_fstype = fstype
        .map(|s| CString::new(s).map_err(|_| SysError::InvalidCString(s.to_owned())))
        .transpose()?;
    let c_data = data
        .map(|s| CString::new(s).map_err(|_| SysError::InvalidCString(s.to_owned())))
        .transpose()?;

    // SAFETY: All non-null pointers reference NUL-terminated CStrings kept alive across `mount(2)`.
    let rc = unsafe {
        libc::mount(
            c_src.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            c_tgt.as_ptr(),
            c_fstype.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            flags,
            c_data
                .as_ref()
                .map_or(std::ptr::null(), |c| c.as_ptr().cast::<libc::c_void>()),
        )
    };
    if rc < 0 {
        return Err(SysError::path_os("mount", target));
    }
    Ok(())
}

/// Marks `/` (or `target`) recursively private (`MS_REC | MS_PRIVATE`) so mounts never leak.
pub fn make_rprivate(target: &Path) -> SysResult<()> {
    mount(None, target, None, libc::MS_REC | libc::MS_PRIVATE, None)
}

/// Marks `target` recursively slave (`MS_REC | MS_SLAVE`) prior to detaching old root.
pub fn make_rslave(target: &Path) -> SysResult<()> {
    mount(None, target, None, libc::MS_REC | libc::MS_SLAVE, None)
}

/// Calls `umount2(2)` with explicit flags.
pub fn umount2(target: &Path, flags: libc::c_int) -> SysResult<()> {
    let c_tgt = os_to_cstring(target.as_os_str())?;
    // SAFETY: `c_tgt` is a valid NUL-terminated path string.
    let rc = unsafe { libc::umount2(c_tgt.as_ptr(), flags) };
    if rc < 0 {
        return Err(SysError::path_os("umount2", target));
    }
    Ok(())
}

/// Unmounts all mounts at or beneath `root_dir` deepest-first, falling back to `MNT_DETACH`.
pub fn unmount_tree(root_dir: &Path) -> SysResult<usize> {
    let mut unmounted = 0;
    for _pass in 0..4 {
        let targets = mount_points_under(root_dir)?;
        if targets.is_empty() {
            break;
        }
        let mut progress = false;
        for target in targets {
            if umount2(&target, 0).is_ok() || umount2(&target, libc::MNT_DETACH).is_ok() {
                unmounted += 1;
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    Ok(unmounted)
}

/// Executes `pivot_root(".", ".")` followed by `make_rslave(".")`, `umount2(".", MNT_DETACH)`,
/// and `chdir("/")`. Caller must have already `chdir`'d into the new root mountpoint.
pub fn pivot_root_in_place() -> SysResult<()> {
    let dot = CString::new(".").map_err(|_| SysError::InvalidCString(".".to_owned()))?;
    // SAFETY: Both arguments are valid NUL-terminated `"."` strings for Linux `SYS_pivot_root`.
    let rc = unsafe { libc::syscall(libc::SYS_pivot_root, dot.as_ptr(), dot.as_ptr()) };
    if rc < 0 {
        return Err(SysError::last_os("pivot_root"));
    }
    let _ = make_rslave(Path::new("."));
    umount2(Path::new("."), libc::MNT_DETACH)?;
    std::env::set_current_dir("/").map_err(|e| SysError::path_io("chdir", "/", e))?;
    Ok(())
}

/// Clears the ext4 `FS_IMMUTABLE_FL` attribute without following symlinks (e.g. stale `.rurienv`).
pub fn clear_immutable_flag(path: &Path) -> SysResult<()> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };
    if !meta.is_file() {
        return Ok(());
    }
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| SysError::path_io("open for getflags", path, e))?;
    clear_immutable_flag_fd(file.as_raw_fd(), path)
}

/// Clears `FS_IMMUTABLE_FL` directly on an already-opened non-symlink regular file descriptor.
pub fn clear_immutable_flag_fd(raw_fd: std::os::fd::RawFd, path: &Path) -> SysResult<()> {
    let mut flags: libc::c_long = 0;
    // SAFETY: `raw_fd` is a valid fd and `flags` is writable storage for `FS_IOC_GETFLAGS`.
    let rc = unsafe {
        libc::ioctl(
            raw_fd,
            FS_IOC_GETFLAGS as libc::Ioctl,
            &mut flags as *mut libc::c_long,
        )
    };
    if rc < 0 {
        return Err(SysError::path_os("FS_IOC_GETFLAGS", path));
    }
    if (flags & FS_IMMUTABLE_FL) != 0 {
        flags &= !FS_IMMUTABLE_FL;
        // SAFETY: `raw_fd` is a valid fd and `flags` points to updated flags for `FS_IOC_SETFLAGS`.
        let set_rc = unsafe {
            libc::ioctl(
                raw_fd,
                FS_IOC_SETFLAGS as libc::Ioctl,
                &flags as *const libc::c_long,
            )
        };
        if set_rc < 0 {
            return Err(SysError::path_os("FS_IOC_SETFLAGS", path));
        }
    }
    Ok(())
}

/// Runs Linux `FITRIM` ioctl on a mounted directory to reclaim sparse ext4 image blocks.
pub fn fstrim_path(mount_dir: &Path) -> SysResult<u64> {
    let dir =
        File::open(mount_dir).map_err(|e| SysError::path_io("open for fstrim", mount_dir, e))?;
    let mut range = FsTrimRange {
        start: 0,
        len: u64::MAX,
        minlen: 0,
    };
    // SAFETY: `dir` is an open directory fd on the target mount and `range` is valid `FsTrimRange`.
    let rc = unsafe {
        libc::ioctl(
            dir.as_raw_fd(),
            FITRIM_IOCTL as libc::Ioctl,
            &mut range as *mut FsTrimRange,
        )
    };
    if rc < 0 {
        return Err(SysError::path_os("FITRIM", mount_dir));
    }
    Ok(range.len)
}

fn os_to_cstring(s: &OsStr) -> SysResult<CString> {
    CString::new(s.as_bytes())
        .map_err(|_| SysError::InvalidCString(s.to_string_lossy().into_owned()))
}

fn unescape_mount_field(raw: &str) -> String {
    raw.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_mounts_and_orders_deepest_first() {
        let sample = "\
/dev/block/loop9 /data/local/strata/containers/main/mnt ext4 rw,relatime 0 0\n\
tmpfs /data/local/strata/containers/main/mnt/dev tmpfs rw 0 0\n\
tmpfs /data/local/strata/containers/main/mnt/dev/shm tmpfs rw 0 0\n\
/dev/block/loop2 /apex/other ext4 ro 0 0\n";
        let mounts = parse_proc_mounts(sample);
        assert_eq!(mounts.len(), 4);

        let tree = select_mount_tree(&mounts, Path::new("/data/local/strata/containers/main/mnt"));
        assert_eq!(
            tree,
            vec![
                PathBuf::from("/data/local/strata/containers/main/mnt/dev/shm"),
                PathBuf::from("/data/local/strata/containers/main/mnt/dev"),
                PathBuf::from("/data/local/strata/containers/main/mnt"),
            ]
        );
    }
}
