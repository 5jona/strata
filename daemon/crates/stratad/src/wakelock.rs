//! Reference-counted optional Android Partial WakeLock (`/sys/power/wake_lock`).
//! Serializes counter transitions with kernel sysfs writes under a mutex (never held across `.await`).

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};

const WAKE_LOCK_PATH: &str = "/sys/power/wake_lock";
const WAKE_UNLOCK_PATH: &str = "/sys/power/wake_unlock";
const WAKE_LOCK_NAME: &str = "strata_active";

/// Manages a reference-counted partial wakelock held only during active PTY sessions or jobs.
#[derive(Debug, Default)]
pub(crate) struct WakeLockManager {
    active_refs: Mutex<usize>,
}

impl WakeLockManager {
    #[must_use]
    pub(crate) fn new() -> Self {
        // Clear any stale lock left behind by a previously crashed daemon instance.
        write_sysfs_wakelock(WAKE_UNLOCK_PATH, WAKE_LOCK_NAME);
        Self {
            active_refs: Mutex::new(0),
        }
    }

    /// Acquires a RAII wakelock guard, asserting the kernel lock on the `0 -> 1` transition.
    #[must_use]
    pub(crate) fn acquire(self: &Arc<Self>) -> WakeLockGuard {
        let mut count = self
            .active_refs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *count == 0 {
            write_sysfs_wakelock(WAKE_LOCK_PATH, WAKE_LOCK_NAME);
        }
        *count = count.saturating_add(1);
        drop(count);
        WakeLockGuard {
            manager: Arc::clone(self),
        }
    }

    /// Returns the number of active wakelock references.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn active_count(&self) -> usize {
        *self
            .active_refs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn release(&self) {
        let mut count = self
            .active_refs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *count = count.saturating_sub(1);
        if *count == 0 {
            write_sysfs_wakelock(WAKE_UNLOCK_PATH, WAKE_LOCK_NAME);
        }
    }
}

/// RAII guard releasing the partial wakelock when the last active session/job finishes.
#[derive(Debug)]
pub(crate) struct WakeLockGuard {
    manager: Arc<WakeLockManager>,
}

impl Drop for WakeLockGuard {
    fn drop(&mut self) {
        self.manager.release();
    }
}

fn write_sysfs_wakelock(path: &str, name: &str) {
    if let Ok(mut file) = OpenOptions::new().write(true).open(path) {
        let _ = file.write_all(name.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_counts_wakelock_guards_cleanly() {
        let mgr = Arc::new(WakeLockManager::new());
        assert_eq!(mgr.active_count(), 0);
        let g1 = mgr.acquire();
        assert_eq!(mgr.active_count(), 1);
        let g2 = mgr.acquire();
        assert_eq!(mgr.active_count(), 2);
        drop(g1);
        assert_eq!(mgr.active_count(), 1);
        drop(g2);
        assert_eq!(mgr.active_count(), 0);
    }
}
