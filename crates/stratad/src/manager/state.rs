//! Per-container live state, restart backoff transitions, and lifecycle slot synchronization.

use std::time::Instant;

use strata_protocol::{ContainerPhase, RestartBackoff};
use tokio::sync::{Mutex, RwLock};

/// Per-container live state tracked inside `stratad`.
#[derive(Debug)]
pub(crate) struct ContainerLiveState {
    pub(crate) phase: ContainerPhase,
    pub(crate) ns_pid: Option<u32>,
    pub(crate) mnt_ns_inode: Option<u64>,
    pub(crate) started_at: Option<Instant>,
    pub(crate) restart_count: u32,
    pub(crate) restart_backoff: RestartBackoff,
    pub(crate) last_error: Option<String>,
    pub(crate) desired_running: bool,
    pub(crate) generation: u64,
}

impl Default for ContainerLiveState {
    fn default() -> Self {
        Self {
            phase: ContainerPhase::default(),
            ns_pid: None,
            mnt_ns_inode: None,
            started_at: None,
            restart_count: 0,
            restart_backoff: RestartBackoff::standard(),
            last_error: None,
            desired_running: false,
            generation: 0,
        }
    }
}

impl ContainerLiveState {
    pub(crate) fn prepare_start(&mut self, is_restart: bool) -> ContainerPhase {
        let prev = self.phase;
        self.phase = ContainerPhase::Starting;
        self.desired_running = true;
        self.last_error = None;
        self.generation = self.generation.saturating_add(1);
        if !is_restart {
            self.restart_backoff.reset();
        }
        prev
    }

    pub(crate) fn record_unexpected_exit(
        &mut self,
        ran_for_ms: u64,
        reason: String,
    ) -> (ContainerPhase, Option<u32>, Option<u64>, u64) {
        let prev = self.phase;
        let ns_pid = self.ns_pid;
        let mnt_ns = self.mnt_ns_inode;
        self.phase = ContainerPhase::Failed;
        self.ns_pid = None;
        self.mnt_ns_inode = None;
        self.started_at = None;
        self.last_error = Some(reason);
        let delay_ms = self.restart_backoff.record_exit(ran_for_ms);
        (prev, ns_pid, mnt_ns, delay_ms)
    }
}

/// Per-container lock pair obeying the strict `lifecycle_lock -> state` lock order.
#[derive(Debug)]
pub(crate) struct ContainerSlot {
    pub(crate) lifecycle_lock: Mutex<()>,
    pub(crate) state: RwLock<ContainerLiveState>,
}

impl ContainerSlot {
    pub(crate) fn new() -> Self {
        Self {
            lifecycle_lock: Mutex::new(()),
            state: RwLock::new(ContainerLiveState::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_restart_backoff_escalates_across_restarts_and_resets_on_explicit_start() {
        let mut live = ContainerLiveState::default();
        live.prepare_start(false);
        live.phase = ContainerPhase::Running;

        // Consecutive rapid crashes must escalate: 2s -> 4s -> 8s -> 16s -> 30s -> 30s.
        let mut delays = Vec::new();
        for _ in 0..6 {
            let (_, _, _, delay) = live.record_unexpected_exit(250, "crashed".to_owned());
            delays.push(delay);
            live.prepare_start(true);
            live.phase = ContainerPhase::Running;
        }
        assert_eq!(delays, vec![2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);

        // Staying healthy past 60s resets the backoff window to 2s.
        let (_, _, _, post_healthy) = live.record_unexpected_exit(90_000, "late exit".to_owned());
        assert_eq!(post_healthy, 2_000);

        // Explicit user start resets accumulated failures back to 0.
        live.prepare_start(false);
        assert_eq!(live.restart_backoff.failures(), 0);
    }
}
