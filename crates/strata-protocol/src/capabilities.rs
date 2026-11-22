//! Linux v3 capability bitmask calculations for container privilege profiles.

use serde::{Deserialize, Serialize};

/// Standard Linux capability numbers (from `<linux/capability.h>`).
pub mod cap {
    pub const CHOWN: u32 = 0;
    pub const DAC_OVERRIDE: u32 = 1;
    pub const DAC_READ_SEARCH: u32 = 2;
    pub const FOWNER: u32 = 3;
    pub const FSETID: u32 = 4;
    pub const KILL: u32 = 5;
    pub const SETGID: u32 = 6;
    pub const SETUID: u32 = 7;
    pub const SETPCAP: u32 = 8;
    pub const NET_BIND_SERVICE: u32 = 10;
    pub const NET_ADMIN: u32 = 12;
    pub const NET_RAW: u32 = 13;
    pub const SYS_MODULE: u32 = 16;
    pub const SYS_RAWIO: u32 = 17;
    pub const SYS_CHROOT: u32 = 18;
    pub const SYS_PTRACE: u32 = 19;
    pub const SYS_ADMIN: u32 = 21;
    pub const SYS_NICE: u32 = 23;
    pub const SYS_RESOURCE: u32 = 24;
    pub const MKNOD: u32 = 27;
    pub const AUDIT_WRITE: u32 = 29;
    pub const SETFCAP: u32 = 31;
    pub const SYSLOG: u32 = 34;
    pub const PERFMON: u32 = 38;
    pub const BPF: u32 = 39;

    /// Fallback highest capability number if `/proc/sys/kernel/cap_last_cap` is unreadable.
    pub const FALLBACK_LAST_CAP: u32 = 40;
}

/// Container Linux capability policy derived from configuration or debug override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapProfile {
    /// Standard developer container without Android PID 1 `nsenter` bridge privileges.
    Isolated,
    /// Developer container with host-exec bridge privileges (matching `host_bridge=1`).
    HostBridge,
    /// Full privileged runtime mode (`debug` or `hostRuntime=true`).
    FullHostRuntime,
}

/// 64-bit Linux capability bitmask split into v3 low/high 32-bit words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapMask {
    pub low: u32,
    pub high: u32,
}

impl CapMask {
    /// Constructs a mask from a slice of capability bit indices.
    #[must_use]
    pub fn from_caps(caps: &[u32]) -> Self {
        let mut mask = Self { low: 0, high: 0 };
        for &bit in caps {
            mask.set(bit);
        }
        mask
    }

    /// Constructs a mask with all capability bits `0..=last_cap` set.
    #[must_use]
    pub fn all_up_to(last_cap: u32) -> Self {
        let mut mask = Self { low: 0, high: 0 };
        let capped = last_cap.min(63);
        for bit in 0..=capped {
            mask.set(bit);
        }
        mask
    }

    /// Returns whether capability bit `bit` is enabled in this mask.
    #[must_use]
    pub fn contains(self, bit: u32) -> bool {
        if bit < 32 {
            (self.low & (1_u32 << bit)) != 0
        } else if bit < 64 {
            (self.high & (1_u32 << (bit - 32))) != 0
        } else {
            false
        }
    }

    /// Intersects two capability masks.
    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        Self {
            low: self.low & other.low,
            high: self.high & other.high,
        }
    }

    fn set(&mut self, bit: u32) {
        if bit < 32 {
            self.low |= 1_u32 << bit;
        } else if bit < 64 {
            self.high |= 1_u32 << (bit - 32);
        }
    }
}

/// Base capabilities needed by an interactive Ubuntu/Dropbear/APT container.
const ISOLATED_CAPS: &[u32] = &[
    cap::CHOWN,
    cap::DAC_OVERRIDE,
    cap::FOWNER,
    cap::FSETID,
    cap::KILL,
    cap::SETGID,
    cap::SETUID,
    cap::SETPCAP,
    cap::NET_BIND_SERVICE,
    cap::NET_ADMIN,
    cap::NET_RAW,
    cap::SYS_CHROOT,
    cap::SYS_NICE,
    cap::SYS_RESOURCE,
    cap::MKNOD,
    cap::AUDIT_WRITE,
    cap::SETFCAP,
];

/// Additional capabilities enabling `host-exec`, `strata-debug`, `dmesg`, `gdb`/`strace`, `perf`/`bpf`, and `insmod`.
const HOST_BRIDGE_EXTRA_CAPS: &[u32] = &[
    cap::DAC_READ_SEARCH,
    cap::SYS_ADMIN,
    cap::SYS_MODULE,
    cap::SYS_PTRACE,
    cap::SYSLOG,
    cap::PERFMON,
    cap::BPF,
];

impl CapProfile {
    /// Computes the desired effective/permitted capability mask for this profile.
    #[must_use]
    pub fn desired_mask(self, last_cap: u32) -> CapMask {
        let valid = CapMask::all_up_to(last_cap);
        match self {
            Self::FullHostRuntime => valid,
            Self::Isolated => CapMask::from_caps(ISOLATED_CAPS).intersect(valid),
            Self::HostBridge => {
                let mut combined =
                    Vec::with_capacity(ISOLATED_CAPS.len() + HOST_BRIDGE_EXTRA_CAPS.len());
                combined.extend_from_slice(ISOLATED_CAPS);
                combined.extend_from_slice(HOST_BRIDGE_EXTRA_CAPS);
                CapMask::from_caps(&combined).intersect(valid)
            }
        }
    }

    /// Returns capability numbers to drop from the bounding set via `PR_CAPBSET_DROP`.
    #[must_use]
    pub fn bounding_drops(self, last_cap: u32) -> Vec<u32> {
        let keep = self.desired_mask(last_cap);
        let capped = last_cap.min(63);
        (0..=capped).filter(|&bit| !keep.contains(bit)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_bridge_retains_bridge_caps_while_dropping_rawio() {
        let mask = CapProfile::HostBridge.desired_mask(40);
        assert!(mask.contains(cap::SYS_ADMIN));
        assert!(mask.contains(cap::SYS_CHROOT));
        assert!(mask.contains(cap::SYS_MODULE));
        assert!(mask.contains(cap::SYS_PTRACE));
        assert!(mask.contains(cap::SYSLOG));
        assert!(mask.contains(cap::PERFMON));
        assert!(mask.contains(cap::BPF));
        assert!(mask.contains(cap::DAC_OVERRIDE));
        assert!(mask.contains(cap::DAC_READ_SEARCH));
        assert!(!mask.contains(cap::SYS_RAWIO));

        // Older kernels with cap_last_cap < 38 must cleanly gate PERFMON/BPF out.
        let older_mask = CapProfile::HostBridge.desired_mask(37);
        assert!(!older_mask.contains(cap::PERFMON));
        assert!(!older_mask.contains(cap::BPF));

        let drops = CapProfile::HostBridge.bounding_drops(40);
        assert!(drops.contains(&cap::SYS_RAWIO));
        assert!(!drops.contains(&cap::SYS_ADMIN));
        assert!(!drops.contains(&cap::PERFMON));
        assert!(!drops.contains(&cap::BPF));
    }

    #[test]
    fn isolated_drops_sys_admin_and_ptrace() {
        let mask = CapProfile::Isolated.desired_mask(40);
        assert!(!mask.contains(cap::SYS_ADMIN));
        assert!(!mask.contains(cap::SYS_PTRACE));
        assert!(!mask.contains(cap::PERFMON));
        assert!(!mask.contains(cap::BPF));
        assert!(mask.contains(cap::NET_BIND_SERVICE));
        assert!(mask.contains(cap::SETUID));
    }

    #[test]
    fn full_host_runtime_drops_nothing() {
        assert!(CapProfile::FullHostRuntime.bounding_drops(40).is_empty());
    }
}
