//! Bounded provisioning job registry, progress updates, and `container.job.progress` emission.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use strata_protocol::EventEnvelope;
use strata_protocol::rpc::{
    ContainerJobCancelResult, ContainerJobKind, ContainerJobProgressPayload, ContainerJobState,
    events,
};
use tokio::sync::Mutex;

use crate::error::{DaemonError, DaemonResult};
use crate::sideveil_seam::{EventHub, PluginEventSink};

const MAX_RETAINED_JOBS: usize = 32;

#[derive(Debug)]
struct TrackedJob {
    progress: ContainerJobProgressPayload,
    cancel: Arc<AtomicBool>,
}

/// Bounded in-memory registry of recent and active container provisioning jobs.
#[derive(Debug)]
pub(super) struct JobTracker {
    event_hub: Arc<EventHub>,
    next_id: AtomicU64,
    jobs: Mutex<VecDeque<TrackedJob>>,
}

impl JobTracker {
    pub(super) fn new(event_hub: Arc<EventHub>) -> Self {
        Self {
            event_hub,
            next_id: AtomicU64::new(1),
            jobs: Mutex::new(VecDeque::new()),
        }
    }

    pub(super) async fn active_count(&self) -> usize {
        self.jobs
            .lock()
            .await
            .iter()
            .filter(|j| {
                matches!(
                    j.progress.state,
                    ContainerJobState::Pending | ContainerJobState::Running
                )
            })
            .count()
    }

    pub(super) async fn status(&self, job_id: &str) -> DaemonResult<ContainerJobProgressPayload> {
        let guard = self.jobs.lock().await;
        guard
            .iter()
            .find(|j| j.progress.job_id == job_id)
            .map(|j| j.progress.clone())
            .ok_or_else(|| DaemonError::JobNotFound(job_id.to_owned()))
    }

    pub(super) async fn cancel(&self, job_id: &str) -> DaemonResult<ContainerJobCancelResult> {
        let guard = self.jobs.lock().await;
        let Some(job) = guard.iter().find(|j| j.progress.job_id == job_id) else {
            return Err(DaemonError::JobNotFound(job_id.to_owned()));
        };
        let active = matches!(
            job.progress.state,
            ContainerJobState::Pending | ContainerJobState::Running
        );
        if active {
            job.cancel.store(true, Ordering::Relaxed);
        }
        Ok(ContainerJobCancelResult {
            job_id: job_id.to_owned(),
            cancelled: active,
        })
    }

    pub(super) async fn register(
        &self,
        container: &str,
        kind: ContainerJobKind,
        stage: &str,
    ) -> (String, Arc<AtomicBool>) {
        let job_id = format!("job-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let now = now_unix_ms();
        let progress = ContainerJobProgressPayload {
            job_id: job_id.clone(),
            container: container.to_owned(),
            kind,
            state: ContainerJobState::Running,
            stage: stage.to_owned(),
            bytes_done: 0,
            bytes_total: None,
            percent: Some(0),
            message: None,
            error: None,
            started_at_ms: now,
            updated_at_ms: now,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut guard = self.jobs.lock().await;
            guard.push_back(TrackedJob {
                progress: progress.clone(),
                cancel: Arc::clone(&cancel),
            });
            while guard.len() > MAX_RETAINED_JOBS {
                guard.pop_front();
            }
        }
        self.emit(&progress);
        (job_id, cancel)
    }

    pub(super) async fn update(
        &self,
        job_id: &str,
        stage: &str,
        bytes_done: u64,
        bytes_total: Option<u64>,
        message: Option<String>,
    ) {
        let mut guard = self.jobs.lock().await;
        if let Some(job) = guard.iter_mut().find(|j| j.progress.job_id == job_id) {
            job.progress.stage = stage.to_owned();
            job.progress.bytes_done = bytes_done;
            if bytes_total.is_some() {
                job.progress.bytes_total = bytes_total;
            }
            job.progress.percent = job
                .progress
                .bytes_total
                .and_then(|tot| (tot > 0).then_some(((bytes_done.min(tot) * 100) / tot) as u8));
            if message.is_some() {
                job.progress.message = message;
            }
            job.progress.updated_at_ms = now_unix_ms();
            let snap = job.progress.clone();
            drop(guard);
            self.emit(&snap);
        }
    }

    pub(super) async fn finalize(
        &self,
        job_id: &str,
        outcome: DaemonResult<()>,
        cancel: &AtomicBool,
    ) {
        let mut guard = self.jobs.lock().await;
        if let Some(job) = guard.iter_mut().find(|j| j.progress.job_id == job_id) {
            job.progress.updated_at_ms = now_unix_ms();
            match outcome {
                Ok(()) => {
                    job.progress.state = ContainerJobState::Completed;
                    job.progress.stage = "completed".to_owned();
                    job.progress.percent = Some(100);
                    job.progress.error = None;
                }
                Err(err) => {
                    job.progress.state = if cancel.load(Ordering::Relaxed) {
                        ContainerJobState::Cancelled
                    } else {
                        ContainerJobState::Failed
                    };
                    job.progress.stage = "failed".to_owned();
                    job.progress.error = Some(err.to_string());
                }
            }
            let snap = job.progress.clone();
            drop(guard);
            self.emit(&snap);
        }
    }

    fn emit(&self, progress: &ContainerJobProgressPayload) {
        self.event_hub.emit(EventEnvelope::new(
            events::CONTAINER_JOB_PROGRESS,
            serde_json::to_value(progress).ok(),
        ));
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
