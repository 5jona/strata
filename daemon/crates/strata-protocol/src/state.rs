//! Pure container lifecycle state machine, status snapshots, and restart backoff policy.

pub use crate::generated::state::{
    ContainerDiskStats, ContainerPhase, ContainerStatusSnapshot, DaemonSummarySnapshot,
};

#[allow(clippy::derivable_impls)]
impl Default for ContainerPhase {
    fn default() -> Self {
        Self::Stopped
    }
}

impl ContainerPhase {
    /// Checks whether transitioning from `self` to `next` is valid.
    #[must_use]
    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (Self::Stopped | Self::Failed, Self::Starting)
                | (
                    Self::Starting,
                    Self::Running | Self::Stopping | Self::Failed | Self::Stopped
                )
                | (Self::Running, Self::Stopping | Self::Failed | Self::Stopped)
                | (Self::Stopping, Self::Stopped | Self::Failed)
        )
    }

    /// Returns true when the container is actively running.
    #[must_use]
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }
}

/// Deterministic exponential backoff calculator for supervising container init restarts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartBackoff {
    base_ms: u64,
    max_ms: u64,
    healthy_reset_ms: u64,
    consecutive_failures: u32,
}

impl RestartBackoff {
    /// Creates a new restart backoff tracker with explicit bounds.
    #[must_use]
    pub fn new(base_ms: u64, max_ms: u64, healthy_reset_ms: u64) -> Self {
        Self {
            base_ms: base_ms.max(100),
            max_ms: max_ms.max(base_ms.max(100)),
            healthy_reset_ms,
            consecutive_failures: 0,
        }
    }

    /// Standard backoff profile: 2s initial, capped at 30s, resets after 60s uptime.
    #[must_use]
    pub fn standard() -> Self {
        Self::new(2_000, 30_000, 60_000)
    }

    /// Records a process exit after `ran_for_ms` and returns the next delay in milliseconds.
    pub fn record_exit(&mut self, ran_for_ms: u64) -> u64 {
        if ran_for_ms >= self.healthy_reset_ms {
            self.consecutive_failures = 0;
        }
        let shift = self.consecutive_failures.min(10);
        let delay = self.base_ms.saturating_mul(1_u64 << shift).min(self.max_ms);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        delay
    }

    /// Resets failure counter after a manual stop or successful start reset.
    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Returns the number of consecutive rapid failures recorded so far.
    #[must_use]
    pub fn failures(&self) -> u32 {
        self.consecutive_failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_transitions_enforce_lifecycle_rules() {
        assert!(ContainerPhase::Stopped.can_transition_to(ContainerPhase::Starting));
        assert!(!ContainerPhase::Stopped.can_transition_to(ContainerPhase::Running));
        assert!(ContainerPhase::Starting.can_transition_to(ContainerPhase::Running));
        assert!(ContainerPhase::Running.can_transition_to(ContainerPhase::Stopping));
        assert!(ContainerPhase::Stopping.can_transition_to(ContainerPhase::Stopped));
        assert!(ContainerPhase::Failed.can_transition_to(ContainerPhase::Starting));
    }

    #[test]
    fn restart_backoff_grows_caps_and_resets_after_healthy_run() {
        let mut backoff = RestartBackoff::standard();
        assert_eq!(backoff.record_exit(500), 2_000);
        assert_eq!(backoff.record_exit(500), 4_000);
        assert_eq!(backoff.record_exit(500), 8_000);
        assert_eq!(backoff.record_exit(500), 16_000);
        assert_eq!(backoff.record_exit(500), 30_000);
        assert_eq!(backoff.record_exit(500), 30_000);
        assert_eq!(backoff.record_exit(120_000), 2_000);
        assert_eq!(backoff.failures(), 1);
    }
}
