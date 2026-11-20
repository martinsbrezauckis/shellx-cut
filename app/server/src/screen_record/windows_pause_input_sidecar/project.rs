//! Read-only Cut project identity and durable op-log binding.

use record_recovery::{
    CaptureRoot, RecordingProjectBinding, RecordingProjectIdentity, RECORDING_PROJECT_ID_SCHEMA,
};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The operation journal's own size limit (`cut_core` journal), applied here
/// too because this reader opens the log itself.
const MAX_PROJECT_LOG_BYTES: u64 = 128 * 1024 * 1024;

pub(crate) fn admit_project_binding(root: &CaptureRoot) -> Result<RecordingProjectBinding, String> {
    let (origin_path_sha256, records) = derive(root)?;
    let accepted_revision = records
        .last()
        .map(|record| record.op_id.clone())
        .ok_or_else(|| "project durable log has no accepted revision".to_string())?;
    Ok(RecordingProjectBinding {
        identity: identity_at(&origin_path_sha256, &records, &accepted_revision)?,
        accepted_revision,
    })
}

pub(crate) fn verify_project_binding(
    root: &CaptureRoot,
    binding: &RecordingProjectBinding,
) -> Result<(), String> {
    let (origin_path_sha256, records) = derive(root)?;
    let identity = identity_at(&origin_path_sha256, &records, &binding.accepted_revision)?;
    if identity != binding.identity {
        return Err("project identity or accepted durable revision is stale".into());
    }
    Ok(())
}

fn derive(root: &CaptureRoot) -> Result<(String, Vec<cut_core::OpRecord>), String> {
    let project_dir = project_dir(root)?;
    let canonical = project_dir
        .canonicalize()
        .map_err(|_| "canonicalize project identity origin".to_string())?;
    let records = read_log(&canonical.join("ops.jsonl"))?;
    Ok((
        format!(
            "sha256:{:x}",
            Sha256::digest(canonical.to_string_lossy().as_bytes())
        ),
        records,
    ))
}

fn identity_at(
    origin_path_sha256: &str,
    records: &[cut_core::OpRecord],
    revision: &str,
) -> Result<RecordingProjectIdentity, String> {
    let end = records
        .iter()
        .position(|record| record.op_id == revision)
        .ok_or_else(|| "accepted revision is not in the current durable project log".to_string())?;
    Ok(RecordingProjectIdentity {
        schema: RECORDING_PROJECT_ID_SCHEMA.into(),
        origin_path_sha256: origin_path_sha256.into(),
        project_name: durable_project_name(&records[..=end])?,
    })
}

fn project_dir(root: &CaptureRoot) -> Result<PathBuf, String> {
    root.cache_dir()
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "derive project root from capture root".to_string())
}

fn read_log(path: &Path) -> Result<Vec<cut_core::OpRecord>, String> {
    read_log_within(path, MAX_PROJECT_LOG_BYTES)
}

fn read_log_within(path: &Path, limit: u64) -> Result<Vec<cut_core::OpRecord>, String> {
    if !record_recovery::is_plain_regular_file(path)
        .map_err(|_| "inspect project durable log".to_string())?
    {
        return Err("project durable log is not a local regular file".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "open project durable log".to_string())?;
    ensure_open_regular(&file)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "read project durable log".to_string())?;
    if bytes.len() as u64 > limit {
        return Err("project durable log is larger than the journal size limit".into());
    }
    if bytes.is_empty() || bytes.last() != Some(&b'\n') {
        return Err("project durable log has no complete terminal revision".into());
    }
    let mut records = Vec::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let record: cut_core::OpRecord = serde_json::from_slice(line)
            .map_err(|_| "project durable log record is malformed".to_string())?;
        if record.op_id != cut_core::OpRecord::format_id(records.len() as u64) {
            return Err("project durable log revision sequence is invalid".into());
        }
        records.push(record);
    }
    Ok(records)
}

/// Mirror the journal reader's post-open local-file check. A pre-open path
/// check alone cannot prove a Windows reparse leaf was not swapped in before
/// the no-follow handle was obtained.
fn ensure_open_regular(file: &File) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|_| "read project log metadata".to_string())?;
    (metadata.file_type().is_file() && !is_reparse(&metadata))
        .then_some(())
        .ok_or_else(|| "opened project durable log is not regular".into())
}

/// Only a name-surrogate reparse point (symbolic link, junction) names another
/// entry. Cloud placeholders, compressed and deduplicated files are the file
/// itself, so they are not refused here (the shared project-file rule).
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    cut_core::reparse::names_another_entry(metadata)
}

fn durable_project_name(records: &[cut_core::OpRecord]) -> Result<String, String> {
    let first = records
        .first()
        .ok_or_else(|| "project durable log is empty".to_string())?;
    if first.verb != "project.create" {
        return Err("project durable log has no creation identity".into());
    }
    let mut name = first
        .args
        .get("name")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128 && !value.contains(['/', '\\']))
        .map(str::to_string)
        .ok_or_else(|| "project creation identity is invalid".to_string())?;
    for record in records
        .iter()
        .skip(1)
        .filter(|record| record.verb == "project.rename")
    {
        name = record
            .args
            .get("name")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 128 && !value.contains(['/', '\\']))
            .map(str::to_string)
            .ok_or_else(|| "project rename identity is invalid".to_string())?;
    }
    Ok(name)
}

#[cfg(test)]
mod size_limit_tests {
    #[test]
    fn a_log_past_the_limit_is_refused_after_reading_only_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ops.jsonl");
        std::fs::write(&path, vec![b' '; 4097]).unwrap();
        let error = super::read_log_within(&path, 4096).unwrap_err();
        assert!(error.contains("size limit"), "{error}");
        assert_eq!(super::MAX_PROJECT_LOG_BYTES, 128 * 1024 * 1024);
    }
}
