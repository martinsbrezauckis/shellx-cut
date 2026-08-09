//! cutd-side lifecycle for durable recording checkpoint manifests.

use std::path::Path;

use crate::dispatch::{parse_args, snapshot};
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use record_recovery::{
    is_plain_dir, is_plain_regular_file, owner_state, read_manifest, recover_interrupted,
    recovery_status, seal_torn_receipt, CaptureRecoveryStatus, CaptureStart, ManifestOwner,
    RecoveryReceipt, RecoveryState, MANIFEST_FILE,
};
use serde::Serialize;

pub(crate) const CHECKPOINT_INTERVAL_MS: u64 = 15_000;

pub(crate) fn validate_capture_id(capture_id: &str) -> Result<(), CutError> {
    let valid = !capture_id.is_empty()
        && capture_id.len() <= 128
        && capture_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        });
    valid.then_some(()).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "capture_id is not valid",
            "capture_id must be the filesystem-safe id returned by screen_record.start",
        )
        .with_suggested_action("pass the exact capture_id from screen_record.start")
    })
}

pub(crate) fn scan_recovery_for_project(project_dir: &Path) -> Result<RecoveryScan, CutError> {
    let Some(cache) = crate::screen_record::containment::existing_cache_dir(project_dir)? else {
        return Ok(RecoveryScan::default());
    };
    let ffmpeg = cut_media::toolpath::ffmpeg();
    let ffprobe = cut_media::toolpath::ffprobe();
    Ok(scan(
        &cache,
        &ffmpeg.to_string_lossy(),
        &ffprobe.to_string_lossy(),
    ))
}

pub(crate) fn startup(project_dir: &Path) -> Result<(), CutError> {
    let scan = scan_recovery_for_project(project_dir)?;
    if !scan.recovered.is_empty() || !scan.failed_closed.is_empty() {
        tracing::info!(recovered = ?scan.recovered, failed_closed = ?scan.failed_closed, "screen-record checkpoint startup scan completed");
    }
    Ok(())
}

#[derive(Debug, Default)]
pub(crate) struct RecoveryScan {
    pub recovered: Vec<String>,
    pub deferred: Vec<String>,
    pub failed_closed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecoveryStatusPage {
    pub captures: Vec<RecoveryStatusItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecoveryStatusItem {
    pub capture_id: String,
    #[serde(flatten)]
    pub status: CaptureRecoveryStatus,
}

pub(crate) fn begin(capture_dir: &Path, capture_id: &str) -> Result<(), CutError> {
    ManifestOwner::begin(
        capture_dir,
        CaptureStart::new(capture_id, CHECKPOINT_INTERVAL_MS),
    )
    .map(|_| ())
    .map_err(|error| checkpoint_error("begin checkpoint manifest", error))
}

pub(crate) fn complete(capture_dir: &Path, source: &Path) -> Result<(), CutError> {
    let manifest = read_manifest(capture_dir)
        .map_err(|error| checkpoint_error("read checkpoint manifest", error))?;
    let receipt = RecoveryReceipt {
        state: RecoveryState::Complete,
        recovered_segments: manifest.checkpoints.len() as u64,
        lost_tail_ms: Some(0),
        lost_tail_lower_bound_ms: 0,
        lost_tail_upper_bound_ms: Some(0),
        audio_first_packet_offset_ms: crate::screen_record::system_audio::read_timing(capture_dir)
            .ok()
            .flatten()
            .and_then(|timing| timing.first_packet_offset_ms),
        source: source
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string),
        note: "recording source and project finalized normally".into(),
    };
    if manifest.has_torn_tail() {
        return seal_torn_receipt(capture_dir, &receipt)
            .map_err(|error| checkpoint_error("repair torn normal completion receipt", error));
    }
    let mut owner = ManifestOwner::open(capture_dir)
        .map_err(|error| checkpoint_error("open checkpoint manifest", error))?;
    owner
        .publish_receipt(receipt)
        .map_err(|error| checkpoint_error("publish normal recording receipt", error))
}
pub(crate) fn scan(cache: &Path, ffmpeg: &str, ffprobe: &str) -> RecoveryScan {
    let mut scan = RecoveryScan::default();
    let Ok(entries) = std::fs::read_dir(cache) else {
        return scan;
    };
    for entry in entries.flatten() {
        let root = entry.path();
        if !is_plain_dir(&root).unwrap_or(false) {
            continue;
        }
        let Ok(id) = entry.file_name().into_string() else {
            scan.failed_closed
                .push("capture-id-unavailable: invalid_capture_id".into());
            continue;
        };
        if validate_capture_id(&id).is_err() {
            scan.failed_closed
                .push("capture-id-unavailable: invalid_capture_id".into());
            continue;
        }
        if !is_plain_regular_file(&root.join(MANIFEST_FILE)).unwrap_or(false) {
            scan.failed_closed.push(format!("{id}: manifest_invalid"));
            continue;
        }
        let manifest = match read_manifest(&root) {
            Ok(manifest) => manifest,
            Err(error) => {
                let quarantine = quarantine_invalid_manifest(&root);
                tracing::warn!(
                    capture_id = %id,
                    error = %error,
                    quarantine_error = ?quarantine.as_ref().err(),
                    "screen-record manifest rejected during recovery scan"
                );
                scan.failed_closed.push(format!("{id}: manifest_invalid"));
                continue;
            }
        };
        // `project.json` is atomically written only after the backend has
        // verified/published normal source.mp4, immediately before Complete is
        // appended to the manifest. Repair that one crash window first: it is a
        // normal completed recording, not an interrupted capture that should be
        // remuxed to a second recovered.mp4.
        if manifest.receipt.is_none()
            && !manifest.has_open_segment()
            && has_sealed_normal_project(&root)
        {
            if let Err(error) = complete(&root, &root.join("source.mp4")) {
                failed_closed(&mut scan, &id, "normal_completion_seal_failed", &error);
            }
            continue;
        }
        match recover_interrupted(&root, ffmpeg, ffprobe, owner_state(&manifest.start)) {
            Ok(Some(_)) => scan.recovered.push(id),
            Ok(None) if manifest.receipt.is_none() => scan.deferred.push(id),
            Ok(None) => {}
            Err(error) => failed_closed(&mut scan, &id, "recovery_failed", &error),
        }
    }
    scan
}
/// A normal completion publishes `project.json` atomically before its manifest
/// receipt. This deliberately recognizes only the local conventional source
/// name; project metadata cannot redirect recovery to an arbitrary path.
fn has_sealed_normal_project(root: &Path) -> bool {
    let project = root.join("project.json");
    let source = root.join("source.mp4");
    if !is_plain_regular_file(&project).unwrap_or(false)
        || !is_plain_regular_file(&source).unwrap_or(false)
    {
        return false;
    }
    let Ok(bytes) = std::fs::read(project) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    let source_video = value
        .get("source_video")
        .and_then(serde_json::Value::as_str);
    let names_source = source_video
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        == Some("source.mp4");
    names_source
}

/// Enumerate the capture-cache status without calling a media tool or changing
/// anything on disk. Cursor values and source paths never escape this owner.
pub(crate) fn status_page(
    cache: &Path,
    after: Option<&str>,
    limit: usize,
) -> Result<RecoveryStatusPage, CutError> {
    if !cache.exists() {
        return Ok(RecoveryStatusPage {
            captures: Vec::new(),
            next_cursor: None,
        });
    }
    let entries = std::fs::read_dir(cache).map_err(|_error| {
        CutError::new(
            error_codes::IO,
            "could not read screen-record recovery cache",
            "check that the project cache directory is readable",
        )
    })?;
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .filter(|_| is_plain_dir(&entry.path()).unwrap_or(false))
                .and_then(|_| entry.file_name().into_string().ok())
        })
        .filter(|id| validate_capture_id(id).is_ok())
        .collect();
    ids.sort_unstable();
    let start = match after {
        None => 0,
        Some(cursor) => ids
            .iter()
            .position(|id| id == cursor)
            .map(|index| index + 1)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "recovery-status cursor was not emitted for this project",
                    "request the first page, then pass its exact next_cursor value",
                )
                .with_suggested_action("pass an exact next_cursor from the previous response")
            })?,
    };
    let limit = limit.clamp(1, 100);
    let selected = ids.iter().skip(start).take(limit);
    let captures: Vec<_> = selected
        .map(|capture_id| RecoveryStatusItem {
            capture_id: capture_id.clone(),
            status: recovery_status(&cache.join(capture_id)),
        })
        .collect();
    let next_cursor = (start.saturating_add(captures.len()) < ids.len())
        .then(|| captures.last().map(|item| item.capture_id.clone()))
        .flatten();
    Ok(RecoveryStatusPage {
        captures,
        next_cursor,
    })
}

/// `screen_record.recovery_status{after?, limit?}` — expose only the safe,
/// project-scoped recovery receipt projection.  This is deliberately separate
/// from `scan`: asking for status cannot invoke ffmpeg, repair a journal, or
/// change a capture's owner state.
pub(crate) async fn recovery_status_handler(
    state: &AppState,
    args: serde_json::Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        after: Option<String>,
        limit: Option<usize>,
    }
    let args: Args = parse_args(args)?;
    if let Some(after) = args.after.as_deref() {
        validate_capture_id(after)?;
    }
    let (_project, _edl, project_dir, _at) = snapshot(state).await?;
    let page = status::for_project(
        &project_dir,
        args.after.as_deref(),
        args.limit.unwrap_or(50),
    )?;
    let result = serde_json::to_value(page).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not serialize recovery status",
            error.to_string(),
        )
    })?;
    Ok(VerbResult::ok(result))
}

fn quarantine_invalid_manifest(root: &Path) -> Result<std::path::PathBuf, std::io::Error> {
    let source = root.join(MANIFEST_FILE);
    let quarantine = root.join("quarantine");
    if !is_plain_dir(root).unwrap_or(false) || !is_plain_regular_file(&source).unwrap_or(false) {
        return Err(std::io::Error::other(
            "manifest root or source is not a local regular path",
        ));
    }
    std::fs::create_dir_all(&quarantine)?;
    if !is_plain_dir(&quarantine).unwrap_or(false) {
        return Err(std::io::Error::other("quarantine directory is not local"));
    }
    let target = quarantine.join("capture.manifest.invalid.jsonl");
    match std::fs::symlink_metadata(&target) {
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "quarantine target already exists",
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::rename(source, &target)?;
    Ok(target)
}

fn checkpoint_error(stage: &str, error: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("could not {stage}: {error}"),
        "recording checkpoints were not safely published",
    )
    .with_suggested_action("check that the project cache is writable, then retry recording")
}

fn failed_closed(scan: &mut RecoveryScan, id: &str, code: &str, error: &impl std::fmt::Display) {
    tracing::warn!(capture_id = %id, failure_code = code, error = %error, "screen-record recovery failed closed");
    scan.failed_closed.push(format!("{id}: {code}"));
}

mod status;
#[cfg(test)]
mod tests;
