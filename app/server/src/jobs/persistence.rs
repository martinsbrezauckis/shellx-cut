//! Durable job-record storage and crash recovery.
//!
//! Job JSON is a recovery trail, so every replacement is staged in the jobs
//! directory, synced, and atomically promoted. Invalid records are moved aside
//! instead of being silently ignored; `jobs.list` exposes the recovery notice.

use super::{outcome::restart_interrupted, JobRecord};
use cut_core::CutError;
use serde::Serialize;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

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

/// Prepare a project's job directory and recover its valid history. A bad
/// `.json` record is retained under `jobs/quarantine/` and reported to callers
/// through `jobs.list`; a failed quarantine aborts attach rather than hiding it.
pub(super) fn recover(project_dir: &Path) -> Result<RecoveredJobs, CutError> {
    let dir = project_dir.join("jobs");
    std::fs::create_dir_all(&dir)?;
    let mut records = Vec::new();
    let mut record_ids = HashSet::new();
    let mut next_seq = 0;
    let mut notices = Vec::new();

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

        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            notices.push(quarantine(
                &dir,
                &path,
                &filename,
                "job record is not a regular file",
            )?);
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                notices.push(quarantine(
                    &dir,
                    &path,
                    &filename,
                    &format!("job record could not be read: {error}"),
                )?);
                continue;
            }
        };
        let mut record = match serde_json::from_str::<JobRecord>(&text) {
            Ok(record) => record,
            Err(error) => {
                notices.push(quarantine(
                    &dir,
                    &path,
                    &filename,
                    &format!("job record is invalid JSON: {error}"),
                )?);
                continue;
            }
        };
        next_seq = next_seq.max(sequence_from_job_id(&record.job_id).unwrap_or(0));
        if filename != format!("{}.json", record.job_id) {
            notices.push(quarantine(
                &dir,
                &path,
                &filename,
                "job record id does not match its filename",
            )?);
            continue;
        }
        if !record_ids.insert(record.job_id.clone()) {
            notices.push(quarantine(
                &dir,
                &path,
                &filename,
                "duplicate job record id",
            )?);
            continue;
        }
        if matches!(
            record.state,
            super::JobState::Queued | super::JobState::Running
        ) {
            restart_interrupted(&mut record);
            record.updated_ts = cut_core::OpRecord::now_ts();
            persist(&path, &record)?;
        }
        records.push(record);
    }

    Ok(RecoveredJobs {
        dir,
        records,
        next_seq,
        notices,
    })
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
            format!("{}: {error}", path.display()),
        )
    })
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomically_with(path, bytes, |_| Ok(()))
}

fn write_atomically_with(
    path: &Path,
    bytes: &[u8],
    before_replace: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "job record has no parent"))?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    before_replace(temporary.path())?;
    temporary.persist(path).map_err(|error| error.error)?;
    // Directory sync is unavailable on some Windows filesystems. The replace
    // remains atomic; this is best-effort extra crash durability there.
    if let Ok(directory) = std::fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
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
}

fn quarantine(
    jobs_dir: &Path,
    source: &Path,
    filename: &str,
    reason: &str,
) -> Result<JobPersistenceNotice, CutError> {
    let quarantine_dir = jobs_dir.join("quarantine");
    std::fs::create_dir_all(&quarantine_dir)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let destination = quarantine_dir.join(format!(
        "{filename}.corrupt.{}.{}",
        std::process::id(),
        nonce
    ));
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
mod tests;
