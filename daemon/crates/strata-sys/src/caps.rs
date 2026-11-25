//! Linux v3 capability syscalls (`SYS_capget`, `SYS_capset`, and `PR_CAPBSET_DROP`).

use std::fs;

use strata_protocol::capabilities::{CapMask, CapProfile, cap};

use crate::error::{SysError, SysResult};

const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;

#[repr(C)]
struct CapUserHeader {
    version: u32,
    pid: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CapUserData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// Reads the kernel's highest capability index from `/proc/sys/kernel/cap_last_cap`.
#[must_use]
pub fn read_last_cap() -> u32 {
    fs::read_to_string("/proc/sys/kernel/cap_last_cap")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map_or(cap::FALLBACK_LAST_CAP, |v| v.min(63))
}

/// Enforces `profile` on the calling thread's bounding, effective, permitted, and inheritable sets.
/// Dropping from the bounding set ensures capabilities remain bounded across `execve` as root.
pub fn apply_cap_profile(profile: CapProfile) -> SysResult<()> {
    let last_cap = read_last_cap();
    let desired = profile.desired_mask(last_cap);
    let drops = profile.bounding_drops(last_cap);

    // Drop undesired bounding capabilities first while CAP_SETPCAP is still held.
    for bit in drops {
        // SAFETY: `PR_CAPBSET_DROP` takes a single capability number and zero trailing arguments.
        let rc = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, bit as libc::c_ulong, 0, 0, 0) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if !matches!(err.raw_os_error(), Some(libc::EINVAL | libc::EPERM)) {
                return Err(SysError::Io {
                    op: "prctl(PR_CAPBSET_DROP)",
                    source: err,
                });
            }
        }
    }

    let current = capget_self()?;
    let current_permitted = CapMask {
        low: current[0].permitted,
        high: current[1].permitted,
    };
    let final_mask = desired.intersect(current_permitted);

    let next = [
        CapUserData {
            effective: final_mask.low,
            permitted: final_mask.low,
            inheritable: 0,
        },
        CapUserData {
            effective: final_mask.high,
            permitted: final_mask.high,
            inheritable: 0,
        },
    ];
    capset_self(&next)
}

fn capget_self() -> SysResult<[CapUserData; 2]> {
    let mut header = CapUserHeader {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };
    let mut data = [CapUserData::default(); 2];
    // SAFETY: `header` uses v3 format and `data` has the required two `CapUserData` elements.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_capget,
            &mut header as *mut CapUserHeader,
            data.as_mut_ptr(),
        )
    };
    if rc < 0 {
        return Err(SysError::last_os("capget"));
    }
    Ok(data)
}

fn capset_self(data: &[CapUserData; 2]) -> SysResult<()> {
    let mut header = CapUserHeader {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };
    // SAFETY: `header` uses v3 format and `data` provides two valid `CapUserData` elements.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_capset,
            &mut header as *mut CapUserHeader,
            data.as_ptr(),
        )
    };
    if rc < 0 {
        return Err(SysError::last_os("capset"));
    }
    Ok(())
}
