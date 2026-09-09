//! Parent/child cancellation for the sequential Render Queue orchestrator.

use super::{JobCancellationReason, JobManager, JobState, JOB_DRAIN_TIMEOUT};
use cut_core::CutError;

impl JobManager {
    /// Cancel a Render Queue and its actual current child before the parent can
    /// advance to another delivery. `waiting_on` is descriptive generally; for
    /// this orchestrator it is a parent-owned cancellation relationship.
    pub async fn abort_render_queue(&self, job_id: &str) -> Result<bool, CutError> {
        let control = self.lock_inner().tasks.remove(job_id);
        let Some(mut control) = control else {
            return Ok(false);
        };

        // Stop and drain the parent before reading waiting_on. Its final drop
        // can replace the dependency while a completed child transitions, so a
        // pre-cancellation snapshot could leave the actual render alive.
        control.request_cancel(JobCancellationReason::CancelledByUser);
        let deadline = tokio::time::Instant::now() + JOB_DRAIN_TIMEOUT;
        if !control.wait_until(deadline).await {
            self.lock_inner().tasks.insert(job_id.to_string(), control);
            return Err(cancel_pending(job_id));
        }
        if self
            .get(job_id)
            .is_some_and(|record| matches!(record.state, JobState::Done))
        {
            return Ok(false);
        }

        let child_job_id = self
            .get(job_id)
            .filter(|record| record.kind == "render_queue")
            .and_then(|record| record.waiting_on)
            .filter(|child| child.kind == "render")
            .map(|child| child.job_id);
        if let Some(child_job_id) = child_job_id {
            // Retain this drained parent control if child cancellation stays
            // pending. A retry must retain authority to reap the child.
            match self.abort(&child_job_id).await {
                Err(error) => {
                    self.lock_inner().tasks.insert(job_id.to_string(), control);
                    return Err(error);
                }
                Ok(false)
                    if self.get(&child_job_id).is_some_and(|record| {
                        matches!(record.state, JobState::Queued | JobState::Running)
                    }) =>
                {
                    // Another jobs.cancel currently owns this child. It may still
                    // be draining and reinsert its control as pending, so do not
                    // report the parent cancelled before that child is terminal.
                    self.lock_inner().tasks.insert(job_id.to_string(), control);
                    return Err(cancel_pending(job_id));
                }
                Ok(_) => {}
            }
        }

        self.cancel_by_user(job_id);
        Ok(true)
    }
}

fn cancel_pending(job_id: &str) -> CutError {
    CutError::new(
        "job_cancel_pending",
        format!("job '{job_id}' is still stopping"),
        format!(
            "a blocking worker did not finish within {} ms",
            JOB_DRAIN_TIMEOUT.as_millis()
        ),
    )
    .with_suggested_action("wait for the worker to finish shutting down, then retry jobs.cancel")
}
