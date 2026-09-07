//! Durable admission for side-effecting project jobs.

use super::{persist, JobManager, JobManagerInner, JobRecord, JobState};
use cut_core::{error_codes, CutError};

impl JobManager {
    /// Admit a new side-effecting job only after its queued record is durable.
    ///
    /// This is intentionally narrower than create: callers that publish output,
    /// consume a one-time confirmation, or retire cache files must not start
    /// work that cannot be recovered from the project job log. A rejected
    /// admission consumes the server-local sequence but is never visible in
    /// memory or written as a partial record.
    pub fn create_durable(&self, kind: &str) -> Result<JobRecord, CutError> {
        let mut inner = self.lock_inner();
        let dir = inner.persist_dir.clone().ok_or_else(|| {
            CutError::new(
                error_codes::CONFLICT,
                "job requires an attached project job store",
                "open the project and restore its durable job history before starting work",
            )
        })?;
        let rec = Self::next_queued_record(&mut inner, kind, None);
        if let Err(error) = persist(&dir.join(format!("{}.json", rec.job_id)), &rec) {
            tracing::error!(
                job_id = %rec.job_id,
                error = %error,
                "refused job admission because the queued record could not persist"
            );
            return Err(CutError::new(
                error_codes::IO,
                format!("could not durably admit {kind} job"),
                error.to_string(),
            )
            .with_suggested_action("resolve project job storage and retry; no work was started"));
        }
        inner.jobs.insert(rec.job_id.clone(), rec.clone());
        Ok(rec)
    }

    pub(super) fn next_queued_record(
        inner: &mut JobManagerInner,
        kind: &str,
        retry: Option<super::JobRetry>,
    ) -> JobRecord {
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
            retry.initialize_root(&rec.job_id);
            if retry.descriptor.is_some() && !retry.descriptor_matches_kind(kind) {
                retry.eligible = false;
                retry.reason = Some("retry descriptor does not match this job owner".into());
                retry.descriptor = None;
            }
        }
        rec
    }
}
