//! Per-service live state, bounded 200-line log ring buffer, and restart policy evaluation.

use std::collections::VecDeque;
use std::time::Instant;

use strata_protocol::rpc::{
    ContainerServiceRuntime, ContainerServiceSpec, ServiceRestartPolicy, ServiceStatus,
};
use tokio::sync::{Mutex, RwLock};

pub(super) const MAX_SERVICE_LOG_LINES: usize = 200;
pub(super) const MAX_SERVICE_LINE_BYTES: usize = 4096;

#[derive(Debug)]
pub(super) struct ServiceSlot {
    pub(super) lock: Mutex<()>,
    pub(super) state: RwLock<ServiceLiveState>,
}

impl ServiceSlot {
    pub(super) fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            state: RwLock::new(ServiceLiveState::new()),
        }
    }
}

#[derive(Debug)]
pub(super) struct ServiceLiveState {
    pub(super) status: ServiceStatus,
    pub(super) desired_running: bool,
    pub(super) generation: u64,
    pub(super) pid: Option<u32>,
    pub(super) started_at: Option<Instant>,
    pub(super) restart_count: u32,
    pub(super) last_error: Option<String>,
    pub(super) logs: VecDeque<String>,
}

impl ServiceLiveState {
    pub(super) fn new() -> Self {
        Self {
            status: ServiceStatus::Stopped,
            desired_running: false,
            generation: 0,
            pid: None,
            started_at: None,
            restart_count: 0,
            last_error: None,
            logs: VecDeque::with_capacity(MAX_SERVICE_LOG_LINES),
        }
    }

    pub(super) fn push_log(&mut self, line: String) {
        let bounded = truncate_utf8_bytes(line, MAX_SERVICE_LINE_BYTES);
        if self.logs.len() >= MAX_SERVICE_LOG_LINES {
            self.logs.pop_front();
        }
        self.logs.push_back(bounded);
    }

    pub(super) fn snapshot(&self, spec: &ContainerServiceSpec) -> ContainerServiceRuntime {
        let uptime_ms = self
            .started_at
            .filter(|_| self.status == ServiceStatus::Running)
            .map_or(0, |t| t.elapsed().as_millis() as u64);
        ContainerServiceRuntime {
            id: spec.id.clone(),
            name: spec.name.clone(),
            status: self.status,
            pid: self.pid,
            uptime_ms,
            restart_count: self.restart_count,
            last_error: self.last_error.clone(),
            auto_start: spec.auto_start,
        }
    }
}

pub(super) fn should_restart(policy: ServiceRestartPolicy, exit_code: i32) -> bool {
    match policy {
        ServiceRestartPolicy::Always => true,
        ServiceRestartPolicy::OnFailure => exit_code != 0,
        ServiceRestartPolicy::Never => false,
    }
}

fn truncate_utf8_bytes(mut s: String, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str("...");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_bounded_log_ring_and_restart_policy_rules() {
        let mut st = ServiceLiveState::new();
        for i in 0..250 {
            st.push_log(format!("line-{i}"));
        }
        assert_eq!(st.logs.len(), MAX_SERVICE_LOG_LINES);
        assert_eq!(st.logs.front().map(String::as_str), Some("line-50"));

        let huge = "a".repeat(8192);
        st.push_log(huge);
        assert!(st.logs.back().expect("last").len() <= MAX_SERVICE_LINE_BYTES + 3);

        assert!(should_restart(ServiceRestartPolicy::Always, 0));
        assert!(should_restart(ServiceRestartPolicy::OnFailure, 1));
        assert!(!should_restart(ServiceRestartPolicy::OnFailure, 0));
        assert!(!should_restart(ServiceRestartPolicy::Never, 1));
    }
}
