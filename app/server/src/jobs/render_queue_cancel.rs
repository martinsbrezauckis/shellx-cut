//! Parent/child cancellation for the sequential Render Queue orchestrator.

use super::{JobCancellationReason, JobManager, JobState};
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
        let drain_timeout = self.drain_timeout();
        let deadline = tokio::time::Instant::now() + drain_timeout;
        if !control.wait_until(deadline).await {
            self.lock_inner().tasks.insert(job_id.to_string(), control);
            return Err(self.cancel_pending_error(job_id, drain_timeout));
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
                    return Err(self.cancel_pending_error(job_id, drain_timeout));
                }
                Ok(_) => {}
            }
        }

        self.cancel_by_user(job_id);
        Ok(true)
    }
}

/// The answer when a cancelled job's worker has not ended inside the drain
/// window. Plain on purpose: the job was told to stop, nothing new starts, and
/// the record turns cancelled when the step it is in has ended.
pub(super) fn cancel_pending(
    job_id: &str,
    drain_timeout: std::time::Duration,
    step: Option<&str>,
) -> CutError {
    let step = step
        .map(|text| format!(" The step it is in: {text}."))
        .unwrap_or_default();
    CutError::new(
        "job_cancel_pending",
        format!("job '{job_id}' was told to stop and is still ending its current step"),
        format!(
            "its worker did not end within {} s; it starts nothing new and the job is recorded as cancelled as soon as that step ends.{step}",
            drain_timeout.as_secs().max(1)
        ),
    )
    .with_suggested_action(
        "wait a few seconds and call jobs.cancel again (it answers cancelled once the step has ended), or read jobs.status",
    )
}

impl JobManager {
    /// The pending-cancel answer, naming the step the job is in (its last
    /// progress message), so a step that cannot be interrupted says so.
    pub(super) fn cancel_pending_error(
        &self,
        job_id: &str,
        drain_timeout: std::time::Duration,
    ) -> CutError {
        let step = self.get(job_id).and_then(|record| record.message);
        cancel_pending(job_id, drain_timeout, step.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::cancel_pending;
    use std::time::Duration;

    #[test]
    fn a_cancel_that_needs_longer_says_what_is_happening_and_what_to_do() {
        let error = cancel_pending("job-1", Duration::from_millis(20_000), None);
        assert_eq!(error.code, "job_cancel_pending");
        assert!(error.message.contains("told to stop"), "{}", error.message);
        assert!(error.cause.contains("20 s"), "{}", error.cause);
        assert!(!error.cause.contains("blocking worker"), "{}", error.cause);
        let action = error.suggested_action.expect("a next step");
        assert!(action.contains("jobs.cancel again"), "{action}");
    }

    #[test]
    fn a_cancel_inside_an_uninterruptible_step_names_that_step() {
        let error = cancel_pending(
            "job-1",
            Duration::from_millis(20_000),
            Some("perception setup: downloading the installer, about 20 MB; it stops right after"),
        );
        assert!(
            error
                .cause
                .contains("downloading the installer, about 20 MB; it stops right after"),
            "{}",
            error.cause
        );
    }

    #[test]
    fn the_manager_quotes_the_jobs_current_step_in_the_pending_answer() {
        let manager = crate::jobs::JobManager::new(crate::events::EventBus::new());
        let job = manager.create("setup_perception");
        manager.progress(
            &job.job_id,
            0.05,
            Some("perception setup: downloading the installer, about 20 MB; a cancel stops right after it".into()),
        );
        let error = manager.cancel_pending_error(&job.job_id, Duration::from_secs(20));
        assert!(error.cause.contains("about 20 MB"), "{}", error.cause);
        assert!(error.cause.contains("stops right after"), "{}", error.cause);
    }
}
