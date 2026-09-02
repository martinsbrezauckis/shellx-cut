//! Shared terminal-state mutation and persistence.
//!
//! Public outcome methods choose the lifecycle reason in their owner; this
//! private sibling applies one terminal record shape and preserves a late
//! worker reconciliation only when project detachment already recorded the
//! matching terminal cancellation.

use super::super::persistence::persist;
use super::{JobCompletion, JobManager, JobOutcome, JobOutcomeReason, JobState};
use cut_core::CutError;

impl JobManager {
    pub(super) fn complete(
        &self,
        job_id: &str,
        result: serde_json::Value,
        completion: JobCompletion,
        reason: JobOutcomeReason,
        message: &str,
    ) {
        let kind = self.update(job_id, |record| {
            record.state = JobState::Done;
            record.queue = None;
            record.waiting_on = None;
            record.completion = Some(completion);
            record.progress = 1.0;
            record.outcome = Some(JobOutcome::Succeeded);
            record.outcome_reason = Some(reason);
            record.message = Some(message.to_string());
            record.result = Some(result);
            if let Some(retry) = record.retry.as_mut() {
                retry.terminal(JobOutcome::Succeeded);
            }
        });
        self.publish_terminal_progress(job_id, kind, message);
    }

    pub(super) fn terminate(
        &self,
        job_id: &str,
        outcome: JobOutcome,
        reason: JobOutcomeReason,
        error: CutError,
        message: &str,
    ) {
        self.terminate_inner(job_id, outcome, reason, error, message, None);
    }

    pub(super) fn terminate_with_result(
        &self,
        job_id: &str,
        outcome: JobOutcome,
        reason: JobOutcomeReason,
        error: CutError,
        message: &str,
        result: serde_json::Value,
    ) {
        self.terminate_inner(
            job_id,
            outcome,
            reason,
            error,
            message,
            Some(result.clone()),
        );
        // Project detachment records its cancellation before waiting for its
        // worker to stop. If that worker removed an owned cache file just
        // before observing cancellation, retain its reconciliation without
        // reviving or changing the already-terminal cancellation semantics.
        self.store_terminal_cancellation_result(job_id, outcome, reason, result);
    }

    fn store_terminal_cancellation_result(
        &self,
        job_id: &str,
        outcome: JobOutcome,
        reason: JobOutcomeReason,
        result: serde_json::Value,
    ) {
        let mut inner = self.lock_inner();
        let persist_dir = inner.persist_dir.clone();
        let Some(record) = inner.jobs.get_mut(job_id) else {
            return;
        };
        if record.result.is_some()
            || record.state != JobState::Failed
            || record.outcome != Some(outcome)
            || record.outcome_reason != Some(reason)
        {
            return;
        }
        record.result = Some(result);
        record.updated_ts = cut_core::OpRecord::now_ts();
        record.persistence_error = None;
        if let Some(dir) = persist_dir {
            if let Err(error) = persist(&dir.join(format!("{job_id}.json")), record) {
                tracing::error!(job_id, error = %error, "failed to persist terminal job result");
                record.persistence_error = Some(error.to_string());
            }
        }
    }

    fn terminate_inner(
        &self,
        job_id: &str,
        outcome: JobOutcome,
        reason: JobOutcomeReason,
        error: CutError,
        message: &str,
        result: Option<serde_json::Value>,
    ) {
        let kind = self.update(job_id, |record| {
            record.state = JobState::Failed;
            record.queue = None;
            record.waiting_on = None;
            record.completion = None;
            record.progress = 1.0;
            record.outcome = Some(outcome);
            record.outcome_reason = Some(reason);
            record.message = Some(message.to_string());
            record.error = Some(error);
            if let Some(result) = result {
                record.result = Some(result);
            }
            if let Some(retry) = record.retry.as_mut() {
                retry.terminal(outcome);
            }
        });
        self.publish_terminal_progress(job_id, kind, message);
    }

    fn publish_terminal_progress(&self, job_id: &str, kind: Option<String>, message: &str) {
        if let Some(kind) = kind {
            self.events.publish(crate::events::Event::JobProgress {
                job_id: job_id.to_string(),
                kind,
                progress: 1.0,
                message: Some(message.to_string()),
            });
        }
    }
}
