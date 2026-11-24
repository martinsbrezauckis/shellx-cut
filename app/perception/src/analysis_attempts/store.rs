//! Bounded, authenticated attempt history. The producer must already own/admit
//! the project's parent directory chain. Never recreates a deleted receipts dir.
use super::AnalysisAttempt;
use cut_core::{error_codes, CutError};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_HISTORY: usize = 32;
// Failed writes and explicitly ephemeral runs live here; readers can report
// their evidence without writing a source cache. Keyed by exact path, asset
// hash and attempt id; no evidence leaks between projects/revisions.
// At most 4 MiB across retained session records, except that the newest record
// is always kept in full. One large error must not multiply by 128 jobs.
static FAILED_WRITES: Mutex<Vec<(PathBuf, AnalysisAttempt, usize)>> = Mutex::new(Vec::new());

struct EncodedSize(usize);
impl std::io::Write for EncodedSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encoded_size(record: &AnalysisAttempt) -> usize {
    let mut size = EncodedSize(0);
    if serde_json::to_writer(&mut size, record).is_err() {
        usize::MAX
    } else {
        size.0
    }
}

fn problem(message: &str, cause: impl ToString) -> CutError {
    CutError::new(error_codes::IO, message, cause.to_string()).with_suggested_action(
        "Check the project drive's free space and permissions, then check again.",
    )
}

fn path(dir: &Path, asset: &str) -> Result<PathBuf, CutError> {
    if asset.is_empty()
        || asset.len() > 160
        || !asset
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err(problem(
            "Analysis attempt record has an invalid file id",
            "A file id cannot name a path outside the project.",
        ));
    }
    Ok(dir.join(format!("{asset}.analysis-attempts.json")))
}

fn admitted_directory(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|m| {
        m.is_dir() && !m.file_type().is_symlink() && !cut_core::reparse::names_another_entry(&m)
    })
}

fn disk(dir: &Path, asset: &str) -> Result<Vec<AnalysisAttempt>, CutError> {
    let path = path(dir, asset)?;
    if !admitted_directory(dir) {
        return if !dir.exists() {
            Ok(Vec::new())
        } else {
            Err(problem(
                "Analysis attempt folder is unavailable",
                dir.display(),
            ))
        };
    }
    if std::fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return Ok(Vec::new());
    }
    let bytes = match cut_core::store::read_bounded_cache(&path, MAX_BYTES) {
        cut_core::store::BoundedCacheRead::Bytes(bytes) => bytes,
        _ => {
            return Err(problem(
                "Analysis attempt record could not be read safely",
                path.display(),
            ))
        }
    };
    if !cut_media::cache_trust::trusted_bytes(&path, &bytes) {
        return Ok(Vec::new()); // received or changed data is not execution evidence
    }
    let records: Vec<AnalysisAttempt> = serde_json::from_slice(&bytes)
        .map_err(|e| problem("Analysis attempt record could not be read", e))?;
    Ok(records
        .into_iter()
        .filter(|r| r.schema_version == 1 && r.asset_id == asset)
        .collect())
}

/// Missing, received and legacy histories return no evidence, not a synthetic
/// not-run or successful attempt. Ephemeral runs and failed writes remain
/// readable in-session within the same retention budget.
pub fn load_attempts(
    dir: &Path,
    asset: &str,
    hash: &str,
) -> Result<Vec<AnalysisAttempt>, CutError> {
    path(dir, asset)?;
    let failed = FAILED_WRITES.lock().unwrap_or_else(|p| p.into_inner());
    let fallback: Vec<_> = failed
        .iter()
        .filter(|(d, r, _)| d == dir && r.asset_id == asset && r.asset_hash == hash)
        .map(|(_, r, _)| r.clone())
        .collect();
    let mut records = match disk(dir, asset) {
        Ok(records) => records,
        Err(_) if !fallback.is_empty() => Vec::new(),
        Err(error) => return Err(error),
    };
    for record in fallback {
        merge(&mut records, record);
    }
    records.retain(|r| r.asset_hash == hash);
    Ok(records)
}

fn merge(records: &mut Vec<AnalysisAttempt>, record: AnalysisAttempt) {
    records.retain(|r| r.attempt_id != record.attempt_id);
    records.push(record);
    records.sort_by(|a, b| {
        (a.asked_at_unix_ms, &a.attempt_id).cmp(&(b.asked_at_unix_ms, &b.attempt_id))
    });
    if records.len() > MAX_HISTORY {
        records.drain(..records.len() - MAX_HISTORY);
    }
}

pub(super) fn remember_session(dir: &Path, record: &AnalysisAttempt) {
    let mut records = FAILED_WRITES.lock().unwrap_or_else(|p| p.into_inner());
    records.retain(|(d, r, _)| !(d == dir && r.attempt_id == record.attempt_id));
    retain_session(&mut records, dir, record);
}

fn retain_session(
    records: &mut Vec<(PathBuf, AnalysisAttempt, usize)>,
    dir: &Path,
    record: &AnalysisAttempt,
) {
    records.push((dir.into(), record.clone(), encoded_size(record)));
    while records.len() > 1
        && (records.len() > 128
            || records
                .iter()
                .fold(0usize, |sum, (_, _, size)| sum.saturating_add(*size))
                > MAX_BYTES as usize)
    {
        records.remove(0);
    }
}

pub(super) fn save(dir: &Path, record: &mut AnalysisAttempt) {
    // Serialize each read/merge/replace, including concurrent instrument jobs.
    let mut failed = FAILED_WRITES.lock().unwrap_or_else(|p| p.into_inner());
    record.storage_error = None;
    let written = (|| {
        let path = path(dir, &record.asset_id)?;
        if !admitted_directory(dir) {
            return Err(problem(
                "Analysis attempt folder disappeared",
                dir.display(),
            ));
        }
        let mut history = disk(dir, &record.asset_id)?;
        merge(&mut history, record.clone());
        let bytes = serde_json::to_vec(&history)
            .map_err(|e| problem("Analysis attempt record could not be encoded", e))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(problem(
                "Analysis attempt record is too large",
                "Attempt evidence exceeded its bounded storage budget.",
            ));
        }
        cut_core::store::replace_project_file(&path, &bytes)
            .map_err(|e| problem("Analysis attempt record could not be saved", e))?;
        cut_media::cache_trust::remember(&path);
        if !cut_media::cache_trust::trusted_bytes(&path, &bytes) {
            return Err(problem(
                "Analysis attempt record could not be authenticated",
                path.display(),
            ));
        }
        Ok(())
    })();
    failed.retain(|(d, r, _)| !(d == dir && r.attempt_id == record.attempt_id));
    if let Err(error) = written {
        tracing::warn!(error = %error, "analysis attempt evidence was not persisted");
        record.storage_error = Some(error);
        retain_session(&mut failed, dir, record);
    }
}
