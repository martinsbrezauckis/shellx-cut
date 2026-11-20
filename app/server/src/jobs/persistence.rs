//! Durable job-record storage and crash recovery.
//!
//! Job JSON is a recovery trail, so every replacement is staged in the jobs
//! directory, synced, and atomically promoted. Invalid records are moved aside
//! instead of being silently ignored; `jobs.list` exposes the recovery notice.

mod cleanup;
mod lineage;
mod paths;
mod retry;

use super::{outcome::restart_interrupted, JobRecord};
use cut_core::CutError;
use lineage::reconcile_retry_lineage;
use retry::write_atomically_with;
#[cfg(test)]
use retry::{
    is_transient_windows_replace_error, platform_replace_retry_delays, promote_temporary,
    write_atomically_with_retry, write_atomically_with_retry_and_cleanup, PromotionFailure,
};
use serde::Serialize;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct JobPersistenceNotice {
    pub code: String,
    pub record: String,
    pub message: String,
    pub quarantine: String,
}

pub(super) struct RecoveredJobs {
    pub(super) dir: PathBuf,
    pub(super) records: Vec<JobRecord>,
    pub(super) next_seq: u64,
    pub(super) notices: Vec<JobPersistenceNotice>,
}

/// Largest job record read back from a project. Records the engine writes are
/// far smaller; the limit matches the other project report ceilings.
const MAX_JOB_RECORD_BYTES: u64 = 64 * 1024 * 1024;

/// Prepare a project's job directory and recover its valid history. A bad
/// `.json` record is retained under `jobs/quarantine/` and reported to callers
/// through `jobs.list`; a failed quarantine aborts attach rather than hiding it.
pub(super) fn recover(project_dir: &Path) -> Result<RecoveredJobs, CutError> {
    recover_within(project_dir, LoadBudget::default())
}

fn recover_within(project_dir: &Path, mut history: LoadBudget) -> Result<RecoveredJobs, CutError> {
    cut_core::store::validate_job_output_roots(project_dir)?;
    let dir = project_dir.join("jobs");
    paths::prepare_jobs_dir(&dir)?;
    let mut records = Vec::new();
    let mut record_ids = HashSet::new();
    let mut next_seq = 0;

    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .map(|extension| extension != "json")
            .unwrap_or(true)
        {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().into_owned();
        next_seq = next_seq.max(sequence_from_filename(&filename).unwrap_or(0));

        if history.full(records.len()) {
            // Left on disk unread; its sequence number above still counts.
            history.left += 1;
            continue;
        }
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            history.set_aside(&dir, &path, &filename, "job record is not a regular file")?;
            continue;
        }
        // A received project may hold a huge or linked file at a record name;
        // it is set aside unread instead of being loaded.
        let text = match cut_core::store::read_bounded_cache(&path, MAX_JOB_RECORD_BYTES) {
            cut_core::store::BoundedCacheRead::Bytes(bytes) => bytes,
            cut_core::store::BoundedCacheRead::Oversized => {
                history.set_aside(
                    &dir,
                    &path,
                    &filename,
                    "job record is larger than 64 MiB and was not read",
                )?;
                continue;
            }
            cut_core::store::BoundedCacheRead::Unavailable => {
                history.set_aside(&dir, &path, &filename, "job record could not be read: it is a link, a file shared with another path, or not an ordinary file")?;
                continue;
            }
        };
        history.bytes += text.len() as u64;
        let record = match serde_json::from_slice::<JobRecord>(&text) {
            Ok(record) => record,
            Err(error) => {
                history.set_aside(
                    &dir,
                    &path,
                    &filename,
                    &format!("job record is invalid JSON: {error}"),
                )?;
                continue;
            }
        };
        next_seq = next_seq.max(sequence_from_job_id(&record.job_id).unwrap_or(0));
        if filename != format!("{}.json", record.job_id) {
            history.set_aside(
                &dir,
                &path,
                &filename,
                "job record id does not match its filename",
            )?;
            continue;
        }
        if !record_ids.insert(record.job_id.clone()) {
            history.set_aside(&dir, &path, &filename, "duplicate job record id")?;
            continue;
        }
        records.push(record);
    }

    // Retry admission writes the child before consuming the source record. If
    // the process stops in that window, repair only a fully validated pair.
    // Validate the complete recovered graph before changing any record, so an
    // inconsistent lineage cannot cause recovery to rewrite related history.
    reconcile_retry_lineage(&dir, &mut records)?;

    for record in &mut records {
        if matches!(
            record.state,
            super::JobState::Queued | super::JobState::Running
        ) {
            restart_interrupted(record);
            record.updated_ts = cut_core::OpRecord::now_ts();
            persist(&dir.join(format!("{}.json", record.job_id)), record)?;
        }
    }

    Ok(RecoveredJobs {
        dir,
        records,
        next_seq,
        notices: history.into_notices(),
    })
}

/// Most job records loaded when a project opens, and most bytes they may add
/// up to. A received project can hold any number of files in `jobs/`; past
/// these the rest stay on disk unread and one notice says so.
const MAX_LOADED_JOB_RECORDS: usize = 50_000;
const MAX_LOADED_JOB_BYTES: u64 = 512 * 1024 * 1024;
/// Most bad records moved to quarantine on one open; the rest stay in place.
const MAX_QUARANTINED_PER_OPEN: usize = 1_000;
/// Highest job sequence number taken from a name: a planted
/// `job_18446744073709551615.json` must not wrap the counter.
const MAX_JOB_SEQUENCE: u64 = 1_000_000_000_000;

struct LoadBudget {
    max_records: usize,
    max_bytes: u64,
    max_quarantined: usize,
    notices: Vec<JobPersistenceNotice>,
    bytes: u64,
    left: usize,
}

impl Default for LoadBudget {
    fn default() -> Self {
        Self::new(
            MAX_LOADED_JOB_RECORDS,
            MAX_LOADED_JOB_BYTES,
            MAX_QUARANTINED_PER_OPEN,
        )
    }
}

impl LoadBudget {
    fn new(max_records: usize, max_bytes: u64, max_quarantined: usize) -> Self {
        Self {
            max_records,
            max_bytes,
            max_quarantined,
            notices: Vec::new(),
            bytes: 0,
            left: 0,
        }
    }

    fn full(&self, loaded: usize) -> bool {
        loaded >= self.max_records || self.bytes >= self.max_bytes
    }

    fn set_aside(
        &mut self,
        dir: &Path,
        path: &Path,
        filename: &str,
        reason: &str,
    ) -> Result<(), CutError> {
        if self.notices.len() >= self.max_quarantined {
            self.left += 1;
            return Ok(());
        }
        self.notices.push(quarantine(dir, path, filename, reason)?);
        Ok(())
    }

    fn into_notices(mut self) -> Vec<JobPersistenceNotice> {
        if self.left > 0 {
            self.notices.push(JobPersistenceNotice {
                code: "job_history_not_loaded".into(),
                record: String::new(),
                message: format!(
                    "{} job records were left unread in jobs/: the project holds more job history than is loaded at once ({MAX_LOADED_JOB_RECORDS} records, 512 MiB, {MAX_QUARANTINED_PER_OPEN} set aside)",
                    self.left
                ),
                quarantine: String::new(),
            });
        }
        self.notices
    }
}

pub(super) fn persist(path: &Path, record: &JobRecord) -> Result<(), CutError> {
    let json = serde_json::to_string_pretty(record).map_err(|error| {
        CutError::new(
            cut_core::error::codes::IO,
            format!("could not serialize job '{}'", record.job_id),
            error.to_string(),
        )
    })?;
    write_atomically(path, json.as_bytes()).map_err(|error| {
        CutError::new(
            cut_core::error::codes::IO,
            format!("could not persist job '{}'", record.job_id),
            format!("job record write failed: {error}"),
        )
    })
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomically_with(path, bytes, |_| Ok(()))
}

fn sequence_from_filename(filename: &str) -> Option<u64> {
    filename
        .strip_suffix(".json")
        .and_then(sequence_from_job_id)
}

fn sequence_from_job_id(job_id: &str) -> Option<u64> {
    let suffix = job_id.strip_prefix("job_")?;
    (!suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| suffix.parse().ok())
        .flatten()
        .filter(|sequence| *sequence <= MAX_JOB_SEQUENCE)
}

fn quarantine(
    jobs_dir: &Path,
    source: &Path,
    filename: &str,
    reason: &str,
) -> Result<JobPersistenceNotice, CutError> {
    let quarantine_dir = paths::prepare_quarantine_dir(jobs_dir)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let destination = quarantine_dir.join(format!(
        "{filename}.corrupt.{}.{}",
        std::process::id(),
        nonce
    ));
    paths::validate_jobs_dir(jobs_dir)?;
    std::fs::rename(source, &destination).map_err(|error| {
        CutError::new(
            cut_core::error::codes::IO,
            format!("could not quarantine corrupt job record '{filename}'"),
            format!("{}: {error}", source.display()),
        )
    })?;
    let quarantine = destination
        .strip_prefix(jobs_dir)
        .unwrap_or(&destination)
        .to_string_lossy()
        .into_owned();
    tracing::warn!(record = %filename, quarantine = %quarantine, reason, "quarantined corrupt job record");
    Ok(JobPersistenceNotice {
        code: "job_record_quarantined".into(),
        record: filename.into(),
        message: reason.into(),
        quarantine,
    })
}

#[cfg(test)]
mod cleanup_tests;
#[cfg(test)]
mod lineage_tests;
#[cfg(test)]
mod paths_tests;
#[cfg(test)]
mod tests;
