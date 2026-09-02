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
    Succeeded,
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
    record.progress = 1.0;
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

    /// Mark a job as a true execution failure. Cancellations and restart
    /// interruptions use their dedicated lifecycle methods instead. Motion can
    /// report a caller cancellation from inside a worker, so preserve that too.
    pub fn fail(&self, job_id: &str, error: CutError) {
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
