//! Persisted retry projection and lineage for jobs with a safe replay owner.

use super::{persist, JobManager, JobOutcome, JobRecord, JobState};
use cut_core::CutError;
use serde::{Deserialize, Serialize};

const RETRIED_REASON: &str = "a retry attempt has already been admitted";

/// Durable retry projection. Public job status exposes its eligibility and
/// lineage while the persisted descriptor remains engine-private; the
/// dispatcher never replays raw historical verb arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRetry {
    pub eligible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub root_job_id: String,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retried_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<JobRetryDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobRetryDescriptor {
    ScreenRecordExport(ScreenRecordExportRetryDescriptor),
    VerifyRerun(VerifyRerunRetryDescriptor),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRecordExportRetryDescriptor {
    pub project_revision: String,
    pub source: String,
    pub plan: String,
    pub format: ScreenRecordExportRetryFormat,
    pub inputs: Vec<JobInputFingerprint>,
    pub system_audio_offset_ms: u64,
}

/// Exact output-only verification inputs. The source render receipt and its
/// output artifact are fingerprinted before a retry can be admitted; no render
/// request or project mutation is replayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyRerunRetryDescriptor {
    pub project_revision: String,
    pub render_id: String,
    pub output_hash: String,
    pub duration_ms: u64,
    pub footage_profile: String,
    pub inputs: Vec<JobInputFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
pub enum ScreenRecordExportRetryFormat {
    Mp4,
    Gif { fps: u32, width: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobInputFingerprint {
    pub role: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

impl JobRetry {
    pub fn screen_record_export(descriptor: ScreenRecordExportRetryDescriptor) -> Self {
        Self {
            eligible: false,
            reason: Some("job is still active".into()),
            root_job_id: String::new(),
            attempt: 1,
            retry_of: None,
            retried_by: None,
            descriptor: Some(JobRetryDescriptor::ScreenRecordExport(descriptor)),
        }
    }

    pub fn verify_rerun(descriptor: VerifyRerunRetryDescriptor) -> Self {
        Self {
            eligible: false,
            reason: Some("job is still active".into()),
            root_job_id: String::new(),
            attempt: 1,
            retry_of: None,
            retried_by: None,
            descriptor: Some(JobRetryDescriptor::VerifyRerun(descriptor)),
        }
    }

    pub fn ineligible(reason: impl Into<String>) -> Self {
        Self {
            eligible: false,
            reason: Some(reason.into()),
            root_job_id: String::new(),
            attempt: 1,
            retry_of: None,
            retried_by: None,
            descriptor: None,
        }
    }

    pub(super) fn initialize(&mut self, job_id: &str) {
        if self.root_job_id.is_empty() {
            self.root_job_id = job_id.to_string();
        }
    }

    pub(super) fn terminal(&mut self, outcome: JobOutcome) {
        if self.descriptor.is_none() {
            return;
        }
        match outcome {
            JobOutcome::Failed | JobOutcome::Interrupted => {
                self.eligible = true;
                self.reason = None;
            }
            JobOutcome::Cancelled => {
                self.eligible = false;
                self.reason = Some("cancelled jobs are not retried automatically".into());
            }
            JobOutcome::Succeeded | JobOutcome::Superseded => {
                self.eligible = false;
                self.reason = Some("only failed jobs are eligible for retry".into());
            }
        }
    }

    pub(super) fn admitted_child(&self, parent_id: &str) -> Self {
        Self {
            eligible: false,
            reason: Some("retry attempt is queued".into()),
            root_job_id: self.root_job_id.clone(),
            attempt: self.attempt.saturating_add(1),
            retry_of: Some(parent_id.to_string()),
            retried_by: None,
            descriptor: self.descriptor.clone(),
        }
    }

    pub(super) fn mark_retried(&mut self, child_id: &str) {
        self.eligible = false;
        self.reason = Some(RETRIED_REASON.into());
        self.retried_by = Some(child_id.to_string());
    }

    pub(super) fn is_marked_retried_by(&self, child_id: &str) -> bool {
        !self.eligible
            && self.reason.as_deref() == Some(RETRIED_REASON)
            && self.retried_by.as_deref() == Some(child_id)
    }
}

impl JobManager {
    /// Create a job with a retry projection owned by its job kind. The record
    /// is persisted before it becomes visible in the in-memory table.
    pub fn create_with_retry(&self, kind: &str, retry: Option<JobRetry>) -> JobRecord {
        let mut inner = self.lock_inner();
        inner.next_seq += 1;
        let now = cut_core::OpRecord::now_ts();
        let mut rec = JobRecord {
            job_id: format!("job_{:03}", inner.next_seq),
            kind: kind.to_string(),
            state: JobState::Queued,
            completion: None,
            outcome: None,
            outcome_reason: None,
            progress: 0.0,
            message: None,
            queue: None,
            waiting_on: None,
            retry,
            created_ts: now.clone(),
            updated_ts: now,
            result: None,
            error: None,
            persistence_error: None,
        };
        if let Some(retry) = rec.retry.as_mut() {
            retry.initialize(&rec.job_id);
        }
        if let Some(dir) = inner.persist_dir.clone() {
            if let Err(error) = persist(&dir.join(format!("{}.json", rec.job_id)), &rec) {
                tracing::error!(
                    job_id = %rec.job_id,
                    error = %error,
                    "failed to persist queued job"
                );
                rec.persistence_error = Some(error.to_string());
            }
        }
        inner.jobs.insert(rec.job_id.clone(), rec.clone());
        rec
    }

    /// Atomically admit the one allowed retry child for an eligible failed
    /// record. The child is persisted before it is visible, and the source is
    /// marked as consumed while the same manager lock is held.
    pub fn admit_retry(&self, source_job_id: &str) -> Result<JobRecord, CutError> {
        let mut inner = self.lock_inner();
        let source = inner.jobs.get(source_job_id).cloned().ok_or_else(|| {
            CutError::new(
                cut_core::error_codes::NOT_FOUND,
                format!("job '{source_job_id}' was not found"),
                "refresh jobs.list and select an existing failed export",
            )
        })?;
        let retry = source.retry.as_ref().ok_or_else(|| {
            CutError::new(
                cut_core::error_codes::CONFLICT,
                format!("job '{source_job_id}' has no retry descriptor"),
                "only jobs with an engine-owned retry descriptor can be retried",
            )
        })?;
        if !matches!(source.state, JobState::Failed) || !retry.eligible {
            return Err(CutError::new(
                cut_core::error_codes::CONFLICT,
                format!("job '{source_job_id}' is not eligible for retry"),
                retry.reason.clone().unwrap_or_else(|| {
                    "only failed jobs with an engine-owned retry descriptor can be retried".into()
                }),
            ));
        }
        if inner.jobs.values().any(|record| {
            record
                .retry
                .as_ref()
                .and_then(|retry| retry.retry_of.as_deref())
                == Some(source_job_id)
        }) {
            return Err(CutError::new(
                cut_core::error_codes::CONFLICT,
                format!("job '{source_job_id}' already has a retry child"),
                "refresh jobs.status to inspect the admitted retry attempt",
            ));
        }

        inner.next_seq += 1;
        let now = cut_core::OpRecord::now_ts();
        let child_id = format!("job_{:03}", inner.next_seq);
        let child = JobRecord {
            job_id: child_id.clone(),
            kind: source.kind,
            state: JobState::Queued,
            completion: None,
            outcome: None,
            outcome_reason: None,
            progress: 0.0,
            message: None,
            queue: None,
            waiting_on: None,
            retry: Some(retry.admitted_child(source_job_id)),
            created_ts: now.clone(),
            updated_ts: now,
            result: None,
            error: None,
            persistence_error: None,
        };
        if let Some(dir) = inner.persist_dir.clone() {
            persist(&dir.join(format!("{}.json", child.job_id)), &child).map_err(|error| {
                CutError::new(
                    cut_core::error_codes::IO,
                    format!("could not persist retry job '{}': {error}", child.job_id),
                    "retry was not admitted; resolve project storage and try again",
                )
            })?;
        }
        inner.jobs.insert(child_id.clone(), child.clone());
        let persist_dir = inner.persist_dir.clone();
        if let Some(source) = inner.jobs.get_mut(source_job_id) {
            if let Some(retry) = source.retry.as_mut() {
                retry.mark_retried(&child_id);
            }
            source.updated_ts = cut_core::OpRecord::now_ts();
            if let Some(dir) = persist_dir {
                if let Err(error) = persist(&dir.join(format!("{}.json", source.job_id)), source) {
                    source.persistence_error = Some(error.to_string());
                }
            }
        }
        Ok(child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;

    #[test]
    fn admission_persists_one_linked_child_and_consumes_the_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = JobManager::new(EventBus::new());
        mgr.attach_project(dir.path()).unwrap();
        let parent = mgr.create_with_retry(
            "screen_record_export",
            Some(JobRetry::screen_record_export(
                ScreenRecordExportRetryDescriptor {
                    project_revision: "op_000001".into(),
                    source: "cache/screen_record/capture/source.mp4".into(),
                    plan: "plans/edit.json".into(),
                    format: ScreenRecordExportRetryFormat::Mp4,
                    inputs: Vec::new(),
                    system_audio_offset_ms: 0,
                },
            )),
        );
        mgr.fail(
            &parent.job_id,
            CutError::new("job_failed", "fixture export failure", "fixture"),
        );
        assert!(mgr.get(&parent.job_id).unwrap().retry.unwrap().eligible);

        let child = mgr.admit_retry(&parent.job_id).unwrap();
        let retry = child.retry.as_ref().unwrap();
        assert_eq!(retry.retry_of.as_deref(), Some(parent.job_id.as_str()));
        assert_eq!(retry.root_job_id, parent.job_id);
        assert_eq!(retry.attempt, 2);
        assert!(dir
            .path()
            .join("jobs")
            .join(format!("{}.json", child.job_id))
            .exists());
        assert_eq!(
            mgr.get(&parent.job_id)
                .unwrap()
                .retry
                .unwrap()
                .retried_by
                .as_deref(),
            Some(child.job_id.as_str())
        );
        assert_eq!(
            mgr.admit_retry(&parent.job_id).unwrap_err().code,
            cut_core::error_codes::CONFLICT
        );
    }

    #[test]
    fn verify_rerun_retry_is_durable_duplicate_safe_and_recovers_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = JobManager::new(EventBus::new());
        mgr.attach_project(dir.path()).unwrap();
        let parent = mgr.create_with_retry(
            "verify-rerun",
            Some(JobRetry::verify_rerun(VerifyRerunRetryDescriptor {
                project_revision: "op_000001".into(),
                render_id: "render_001".into(),
                output_hash: "sha256:fixture".into(),
                duration_ms: 1_000,
                footage_profile: "talking_head".into(),
                inputs: vec![JobInputFingerprint {
                    role: "rendered_output".into(),
                    path: "exports/render_001.mp4".into(),
                    bytes: 42,
                    sha256: "fixture".into(),
                }],
            })),
        );
        mgr.fail(
            &parent.job_id,
            CutError::new("job_failed", "fixture verification failure", "fixture"),
        );

        let child = mgr.admit_retry(&parent.job_id).unwrap();
        assert_eq!(child.kind, "verify-rerun");
        assert_eq!(child.retry.as_ref().unwrap().attempt, 2);
        assert_eq!(
            mgr.admit_retry(&parent.job_id).unwrap_err().code,
            cut_core::error_codes::CONFLICT
        );
        drop(mgr);

        let reopened = JobManager::new(EventBus::new());
        reopened.attach_project(dir.path()).unwrap();
        let recovered_parent = reopened.get(&parent.job_id).unwrap();
        let recovered_child = reopened.get(&child.job_id).unwrap();
        assert_eq!(
            recovered_parent.retry.unwrap().retried_by.as_deref(),
            Some(child.job_id.as_str())
        );
        assert_eq!(recovered_child.state, JobState::Failed);
        assert!(recovered_child.retry.unwrap().eligible);
        assert_eq!(
            reopened
                .admit_retry(&child.job_id)
                .unwrap()
                .retry
                .unwrap()
                .attempt,
            3
        );
    }
}
