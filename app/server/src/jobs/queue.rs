//! Durable admission facts for jobs waiting on shared local capacity.

use super::{JobManager, JobState};
use serde::{Deserialize, Serialize};
use std::future::Future;

/// Why a queued job has not started yet. Optional and backward-compatible so
/// older records remain readable and non-limited jobs keep the compact shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobQueueInfo {
    /// Shared limiter key, such as `render`, `analysis`, or `proxy`.
    pub resource: String,
    /// Maximum jobs that may hold this resource concurrently.
    pub max_running: usize,
}

impl JobManager {
    /// Spawn behind a per-key limiter. The durable record names the constrained
    /// resource until its task actually acquires a slot.
    pub fn spawn_limited<F>(&self, job_id: &str, key: &str, max_running: usize, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let capacity = max_running.max(1);
        let resource = key.to_string();
        self.update(job_id, |record| {
            record.state = JobState::Queued;
            record.queue = Some(JobQueueInfo {
                resource: resource.clone(),
                max_running: capacity,
            });
        });
        let limiter = self.limiter(key, capacity);
        let manager = self.clone();
        let owned_job_id = job_id.to_string();
        self.spawn(job_id, async move {
            let _permit = limiter.acquire_owned().await.expect("job limiter closed");
            manager.update(&owned_job_id, |record| {
                record.state = JobState::Running;
                record.queue = None;
            });
            future.await;
        });
    }
}
