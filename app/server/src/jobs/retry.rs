//! Persisted retry projection and lineage for jobs with a safe replay owner.

use super::{persist, JobManager, JobManagerInner, JobOutcome, JobRecord, JobState};
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

    /// Normalize creation-time lineage. Callers construct a recipe before the
    /// manager assigns its id, so they must never be able to pre-seed a root
    /// id, retry links, or an attempt counter on a newly created record.
    pub(super) fn initialize_root(&mut self, job_id: &str) {
        self.eligible = false;
        self.root_job_id = job_id.to_string();
        self.attempt = 1;
        self.retry_of = None;
        self.retried_by = None;
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
            // `admit_retry` rejects the final valid attempt before creating a
            // child, preserving the same finite lineage rule as recovery.
            attempt: self
                .attempt
                .checked_add(1)
                .expect("retry admission checked the attempt bound"),
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

    pub(super) fn descriptor_matches_kind(&self, job_kind: &str) -> bool {
        matches!(
            (&self.descriptor, job_kind),
            (
                Some(JobRetryDescriptor::ScreenRecordExport(_)),
                "screen_record_export"
            ) | (Some(JobRetryDescriptor::VerifyRerun(_)), "verify-rerun")
        )
    }
}

impl JobManager {
    /// Create a job with a retry projection owned by its job kind. The record
    /// is persisted before it becomes visible in the in-memory table.
    pub fn create_with_retry(&self, kind: &str, retry: Option<JobRetry>) -> JobRecord {
        let mut inner = self.lock_inner();
        let mut rec = Self::next_queued_record(&mut inner, kind, retry);
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
        let source = Self::retry_candidate_locked(&inner, source_job_id)?;
        let retry = source
            .retry
            .as_ref()
            .expect("retry candidate has a retry projection");
        let dir = inner.persist_dir.clone().ok_or_else(|| {
            CutError::new(
                cut_core::error_codes::CONFLICT,
                "retry requires an attached project job store",
                "open the project and restore its durable job history before retrying",
            )
        })?;

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
        persist(&dir.join(format!("{}.json", child.job_id)), &child).map_err(|error| {
            CutError::new(
                cut_core::error_codes::IO,
                format!("could not persist retry job '{}': {error}", child.job_id),
                "retry was not admitted; resolve project storage and try again",
            )
        })?;
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

    /// Return a source that is still safe to revalidate and admit. Dispatch
    /// calls this before it reserves output paths or re-hashes inputs; admission
    /// repeats the same check under the manager lock.
    pub fn retry_candidate(&self, source_job_id: &str) -> Result<JobRecord, CutError> {
        let inner = self.lock_inner();
        Self::retry_candidate_locked(&inner, source_job_id)
    }

    fn retry_candidate_locked(
        inner: &JobManagerInner,
        source_job_id: &str,
    ) -> Result<JobRecord, CutError> {
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
        if source.persistence_error.is_some() {
            return Err(CutError::new(
                cut_core::error_codes::CONFLICT,
                format!("job '{source_job_id}' has an unresolved persistence error"),
                "resolve project storage and restart the server before retrying this job",
            ));
        }
        if !matches!(source.state, JobState::Failed) || !retry.eligible {
            return Err(CutError::new(
                cut_core::error_codes::CONFLICT,
                format!("job '{source_job_id}' is not eligible for retry"),
                retry.reason.clone().unwrap_or_else(|| {
                    "only failed jobs with an engine-owned retry descriptor can be retried".into()
                }),
            ));
        }
        if !retry.descriptor_matches_kind(&source.kind) {
            return Err(CutError::new(
                cut_core::error_codes::CONFLICT,
                format!("job '{source_job_id}' has a retry descriptor for another job owner"),
                "start a new job; this persisted retry recipe is not safe to replay",
            ));
        }
        if retry.attempt == u32::MAX {
            return Err(CutError::new(
                cut_core::error_codes::CONFLICT,
                format!("job '{source_job_id}' reached the maximum retry attempt"),
                "start a new job; this retry lineage cannot be extended safely",
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
        Ok(source)
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

    #[test]
    fn admission_refuses_unresolved_persistence_or_exhausted_lineage() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = JobManager::new(EventBus::new());
        mgr.attach_project(dir.path()).unwrap();
        let parent = mgr.create_with_retry(
            "screen_record_export",
            Some(JobRetry::screen_record_export(
                ScreenRecordExportRetryDescriptor {
                    project_revision: "op_000001".into(),
                    source: "source.mp4".into(),
                    plan: "edit.json".into(),
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
        {
            let mut inner = mgr.lock_inner();
            inner
                .jobs
                .get_mut(&parent.job_id)
                .unwrap()
                .persistence_error = Some("disk full".into());
        }
        assert_eq!(
            mgr.retry_candidate(&parent.job_id).unwrap_err().code,
            cut_core::error_codes::CONFLICT
        );
        {
            let mut inner = mgr.lock_inner();
            let record = inner.jobs.get_mut(&parent.job_id).unwrap();
            record.persistence_error = None;
            record.retry.as_mut().unwrap().attempt = u32::MAX;
        }
        assert_eq!(
            mgr.admit_retry(&parent.job_id).unwrap_err().code,
            cut_core::error_codes::CONFLICT
        );
        assert_eq!(mgr.list().len(), 1, "unsafe admission must not add a child");
    }

    #[test]
    fn admission_refuses_a_retry_without_an_attached_durable_job_store() {
        let mgr = JobManager::new(EventBus::new());
        let parent = mgr.create_with_retry(
            "screen_record_export",
            Some(JobRetry::screen_record_export(
                ScreenRecordExportRetryDescriptor {
                    project_revision: "op_000001".into(),
                    source: "source.mp4".into(),
                    plan: "edit.json".into(),
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

        let error = mgr.admit_retry(&parent.job_id).unwrap_err();
        assert_eq!(error.code, cut_core::error_codes::CONFLICT);
        assert!(error.message.contains("attached project job store"));
        assert_eq!(
            mgr.list().len(),
            1,
            "non-durable admission must add no child"
        );
        assert!(
            mgr.get(&parent.job_id).unwrap().retry.unwrap().eligible,
            "failed source remains available after a fail-closed refusal"
        );
    }

    #[test]
    fn creation_canonicalizes_root_lineage_and_rejects_wrong_owner_recipe() {
        let mgr = JobManager::new(EventBus::new());
        let mut retry = JobRetry::screen_record_export(ScreenRecordExportRetryDescriptor {
            project_revision: "op_000001".into(),
            source: "source.mp4".into(),
            plan: "edit.json".into(),
            format: ScreenRecordExportRetryFormat::Mp4,
            inputs: Vec::new(),
            system_audio_offset_ms: 0,
        });
        retry.eligible = true;
        retry.root_job_id = "forged-root".into();
        retry.attempt = 42;
        retry.retry_of = Some("forged-parent".into());
        retry.retried_by = Some("forged-child".into());

        let record = mgr.create_with_retry("verify-rerun", Some(retry));
        let retry = record.retry.unwrap();
        assert_eq!(retry.root_job_id, record.job_id);
        assert_eq!(retry.attempt, 1);
        assert!(retry.retry_of.is_none());
        assert!(retry.retried_by.is_none());
        assert!(!retry.eligible);
        assert!(retry.descriptor.is_none());
        assert_eq!(
            retry.reason.as_deref(),
            Some("retry descriptor does not match this job owner")
        );
    }
}
