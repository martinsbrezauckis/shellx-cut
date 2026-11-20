//! Terminal job outcome taxonomy.
//!
//! `JobState` remains the stable queued/running/done/failed API. These fields
//! explain *why* a terminal job ended without asking callers to infer it from
//! an implementation-specific error string.

use super::{JobCancellationReason, JobCompletion, JobManager, JobRecord, JobState};
use cut_core::CutError;
use serde::{Deserialize, Serialize};

mod terminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOutcome {
    /// Ran to its end and did what was asked (possibly with warnings).
    Succeeded,
    /// Ran to its end but did NOT fully deliver: a written file failed its own
    /// checks (`checks_failed`) or some items of a batch failed
    /// (`items_failed`). Never read as success.
    Partial,
    Failed,
    Cancelled,
    Interrupted,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOutcomeReason {
    Completed,
    CompletedWithWarnings,
    /// The job wrote its output, but the output did not pass its own checks
    /// (a render whose receipt verdict is `problems`). Usable, not correct.
    ChecksFailed,
    /// A batch job (a render queue) finished, but some of its items failed:
    /// the delivered items are real, the failed ones are named in the result.
    ItemsFailed,
    TrueFailure,
    UserCancelled,
    ProjectSwitchCancelled,
    RestartInterrupted,
    Superseded,
}

pub(super) fn restart_interrupted(record: &mut JobRecord) {
    record.state = JobState::Failed;
    record.queue = None;
    record.waiting_on = None;
    record.completion = None;
    // Keep the progress the job really reached; an interrupted job is not 100%.
    record.outcome = Some(JobOutcome::Interrupted);
    record.outcome_reason = Some(JobOutcomeReason::RestartInterrupted);
    record.message = Some("interrupted by restart".into());
    record.error = Some(CutError::new(
        "job_failed",
        format!("job '{}' was interrupted", record.job_id),
        "server restarted while the job was running",
    ));
    if let Some(retry) = record.retry.as_mut() {
        retry.terminal(JobOutcome::Interrupted);
    }
}

impl JobManager {
    /// Mark done with a result payload.
    pub fn finish(&self, job_id: &str, result: serde_json::Value) {
        self.complete(
            job_id,
            result,
            JobCompletion::Success,
            JobOutcomeReason::Completed,
            "done",
        );
    }

    /// Mark done with a usable but partial result and explicit warnings.
    pub fn finish_with_warnings(&self, job_id: &str, result: serde_json::Value) {
        self.complete(
            job_id,
            result,
            JobCompletion::DoneWithWarnings,
            JobOutcomeReason::CompletedWithWarnings,
            "done with warnings",
        );
    }

    /// Mark done with an output that was written but did not pass its own
    /// checks: "completed with problems" (completion done_with_warnings,
    /// outcome_reason checks_failed). The output is kept; the result names the
    /// failing checks and the way forward, and the status bar lists the job
    /// like a failed one until dismissed.
    pub fn finish_with_problems(&self, job_id: &str, result: serde_json::Value) {
        self.complete(
            job_id,
            result,
            JobCompletion::DoneWithWarnings,
            JobOutcomeReason::ChecksFailed,
            "done, but it did not pass its checks",
        );
    }

    /// Mark done when some, not all, items of a batch failed: "done, but N of
    /// M failed" (completion done_with_warnings, outcome_reason items_failed).
    /// The result names each failed item; the status bar lists the job like a
    /// failed one until dismissed. All items failed is a failure, not this.
    pub fn finish_with_failed_items(
        &self,
        job_id: &str,
        result: serde_json::Value,
        failed: usize,
        count: usize,
    ) {
        self.complete(
            job_id,
            result,
            JobCompletion::DoneWithWarnings,
            JobOutcomeReason::ItemsFailed,
            &format!("done, but {failed} of {count} failed"),
        );
    }

    /// Mark a job as a true execution failure. Cancellations and restart
    /// interruptions use their dedicated lifecycle methods instead. Motion can
    /// report a caller cancellation from inside a worker, so preserve that too.
    pub fn fail(&self, job_id: &str, error: CutError) {
        let error = crate::requirements::complete(error);
        if let Some(doctor) = &self.doctor {
            doctor.invalidate_on_missing(&error);
        }
        let (outcome, reason, message) = match error.code.as_str() {
            "job_cancelled" | cut_core::error::codes::RENDER_CANCELLED => (
                JobOutcome::Cancelled,
                JobOutcomeReason::UserCancelled,
                "cancelled",
            ),
            "job_superseded" => (
                JobOutcome::Superseded,
                JobOutcomeReason::Superseded,
                "superseded",
            ),
            _ => (JobOutcome::Failed, JobOutcomeReason::TrueFailure, "failed"),
        };
        self.terminate(job_id, outcome, reason, error, message);
    }

    /// Preserve a path-free accounting payload when a true failure happens
    /// after a worker has already made a known change. The terminal state stays
    /// Failed with its original error; callers must not treat the result as a
    /// successful completion.
    pub(crate) fn fail_with_result(
        &self,
        job_id: &str,
        error: CutError,
        result: serde_json::Value,
    ) {
        let error = crate::requirements::complete(error);
        if let Some(doctor) = &self.doctor {
            doctor.invalidate_on_missing(&error);
        }
        let (outcome, reason, message) = match error.code.as_str() {
            "job_cancelled" | cut_core::error::codes::RENDER_CANCELLED => (
                JobOutcome::Cancelled,
                JobOutcomeReason::UserCancelled,
                "cancelled",
            ),
            "job_superseded" => (
                JobOutcome::Superseded,
                JobOutcomeReason::Superseded,
                "superseded",
            ),
            _ => (JobOutcome::Failed, JobOutcomeReason::TrueFailure, "failed"),
        };
        self.terminate_with_result(job_id, outcome, reason, error, message, result);
    }

    pub(crate) fn cancel_by_user(&self, job_id: &str) {
        self.terminate(
            job_id,
            JobOutcome::Cancelled,
            JobOutcomeReason::UserCancelled,
            CutError::new(
                "job_cancelled",
                format!("job '{job_id}' was cancelled"),
                "the active background task was aborted",
            ),
            "cancelled",
        );
    }

    /// Record the reason already captured by a cooperative worker. This avoids
    /// flattening a project switch or supersession into a user cancellation when
    /// the worker stops quickly before the caller's drain method returns.
    pub(crate) fn cancel_from_worker(&self, job_id: &str, reason: JobCancellationReason) {
        match reason {
            JobCancellationReason::CancelledByUser => self.cancel_by_user(job_id),
            JobCancellationReason::ProjectSwitch => self.cancel_for_project_switch(job_id),
            JobCancellationReason::Superseded => self.supersede(job_id),
            JobCancellationReason::Restart => self.terminate(
                job_id,
                JobOutcome::Interrupted,
                JobOutcomeReason::RestartInterrupted,
                CutError::new(
                    "job_failed",
                    format!("job '{job_id}' was interrupted"),
                    "the server restarted while its owned worker was stopping",
                ),
                "interrupted by restart",
            ),
            JobCancellationReason::None => {}
        }
    }

    /// Preserve a path-free progress reconciliation when a cooperative worker
    /// has already made a reversible, partially completed change. The job
    /// remains a normal cancelled/interrupted/superseded terminal record; this
    /// only gives its owner a truthful result payload instead of implying that
    /// cancellation happened before all work.
    pub(crate) fn cancel_from_worker_with_result(
        &self,
        job_id: &str,
        reason: JobCancellationReason,
        result: serde_json::Value,
    ) {
        let (outcome, outcome_reason, error, message) = match reason {
            JobCancellationReason::CancelledByUser => (
                JobOutcome::Cancelled,
                JobOutcomeReason::UserCancelled,
                CutError::new(
                    "job_cancelled",
                    format!("job '{job_id}' was cancelled"),
                    "the active background task was aborted",
                ),
                "cancelled",
            ),
            JobCancellationReason::ProjectSwitch => (
                JobOutcome::Cancelled,
                JobOutcomeReason::ProjectSwitchCancelled,
                CutError::new(
                    "job_cancelled",
                    format!("job '{job_id}' was cancelled because the project changed"),
                    "background work cannot continue after its owning project is closed",
                ),
                "cancelled for project switch",
            ),
            JobCancellationReason::Superseded => (
                JobOutcome::Superseded,
                JobOutcomeReason::Superseded,
                CutError::new(
                    "job_superseded",
                    format!("job '{job_id}' was superseded"),
                    "a newer request replaced this job before it could commit",
                ),
                "superseded",
            ),
            JobCancellationReason::Restart => (
                JobOutcome::Interrupted,
                JobOutcomeReason::RestartInterrupted,
                CutError::new(
                    "job_failed",
                    format!("job '{job_id}' was interrupted"),
                    "the server restarted while its owned worker was stopping",
                ),
                "interrupted by restart",
            ),
            JobCancellationReason::None => return,
        };
        self.terminate_with_result(job_id, outcome, outcome_reason, error, message, result);
    }

    pub(crate) fn cancel_for_project_switch(&self, job_id: &str) {
        self.terminate(
            job_id,
            JobOutcome::Cancelled,
            JobOutcomeReason::ProjectSwitchCancelled,
            CutError::new(
                "job_cancelled",
                format!("job '{job_id}' was cancelled because the project changed"),
                "background work cannot continue after its owning project is closed",
            ),
            "cancelled for project switch",
        );
    }

    /// Mark a worker whose output a newer request made obsolete. Call this from
    /// the worker's own replacement path after it has stopped its subprocesses.
    pub(crate) fn supersede(&self, job_id: &str) {
        self.terminate(
            job_id,
            JobOutcome::Superseded,
            JobOutcomeReason::Superseded,
            CutError::new(
                "job_superseded",
                format!("job '{job_id}' was superseded"),
                "a newer request replaced this job before it could commit",
            ),
            "superseded",
        );
    }
}

#[cfg(test)]
mod tests;
