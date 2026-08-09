//! Atomic timing-receipt publication for system-loopback captures.

use super::{timing_path, SystemAudioTiming};
use cut_core::{error_codes, CutError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SYSTEM_AUDIO_TIMING_PENDING_FILE: &str = "system-audio.json.pending";

pub(super) fn pending_timing_path(capture_dir: &Path) -> PathBuf {
    capture_dir.join(SYSTEM_AUDIO_TIMING_PENDING_FILE)
}

pub(super) fn timing_publication_is_pending(capture_dir: &Path) -> Result<bool, CutError> {
    ensure_capture_dir(capture_dir)?;
    local_regular_file_or_absent(&pending_timing_path(capture_dir), "inspect timing marker")
}

pub(super) fn begin_timing_publication(capture_dir: &Path) -> Result<(), CutError> {
    ensure_capture_dir(capture_dir)?;
    let pending = pending_timing_path(capture_dir);
    remove_plain_file_if_present(&pending, "replace prior system-audio timing marker")?;
    write_new_synced(
        &pending,
        b"shellx-cut/system-audio-timing-pending/1\n",
        "begin system-audio timing publication",
    )?;
    remove_plain_file_if_present(&timing_path(capture_dir), "clear prior system-audio timing")
}

pub(super) fn clear_timing_publication(capture_dir: &Path) -> Result<(), CutError> {
    ensure_capture_dir(capture_dir)?;
    remove_plain_file_if_present(
        &pending_timing_path(capture_dir),
        "finish system-audio timing publication",
    )
}

pub(super) fn discard_incomplete_timing_publication(out: &Path, capture_dir: &Path) {
    if ensure_capture_dir(capture_dir).is_ok() {
        let _ = remove_plain_file_if_present(out, "discard incomplete system audio");
        let _ = remove_plain_file_if_present(&timing_path(capture_dir), "discard timing sidecar");
        let _ = remove_plain_file_if_present(
            &pending_timing_path(capture_dir),
            "discard timing marker",
        );
    }
}

pub(super) fn write_timing(capture_dir: &Path, timing: &SystemAudioTiming) -> Result<(), CutError> {
    ensure_capture_dir(capture_dir)?;
    let path = timing_path(capture_dir);
    let bytes = serde_json::to_vec_pretty(timing).map_err(|error| {
        CutError::new(
            error_codes::IO,
            format!("could not serialize system-audio timing: {error}"),
            "the system-audio timing sidecar could not be written",
        )
    })?;
    let temporary = unique_staging_path(capture_dir)?;
    write_new_synced(&temporary, &bytes, "write partial system-audio timing")?;
    if let Err(error) = record_recovery::publish_new_synced(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(io_error(&path, "publish system-audio timing", error));
    }
    Ok(())
}

fn ensure_capture_dir(capture_dir: &Path) -> Result<(), CutError> {
    record_recovery::is_plain_dir(capture_dir)
        .map_err(|error| {
            io_error(
                capture_dir,
                "inspect system-audio capture directory",
                std::io::Error::other(error),
            )
        })?
        .then_some(())
        .ok_or_else(|| unsafe_path(capture_dir, "system-audio capture directory"))
}

fn unique_staging_path(capture_dir: &Path) -> Result<PathBuf, CutError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    for attempt in 0..32 {
        let path = capture_dir.join(format!(
            ".system-audio-timing-{}-{nonce}-{attempt}.part",
            std::process::id()
        ));
        if std::fs::symlink_metadata(&path)
            .map(|_| false)
            .unwrap_or(true)
        {
            return Ok(path);
        }
    }
    Err(CutError::new(
        error_codes::IO,
        "could not reserve system-audio timing staging",
        "the capture directory has no safe local staging name",
    ))
}

fn write_new_synced(path: &Path, bytes: &[u8], stage: &str) -> Result<(), CutError> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error(path, stage, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(path, stage, error))
}

fn remove_plain_file_if_present(path: &Path, stage: &str) -> Result<(), CutError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, stage, error)),
        Ok(_) if record_recovery::is_plain_regular_file(path).unwrap_or(false) => {
            std::fs::remove_file(path).map_err(|error| io_error(path, stage, error))
        }
        Ok(_) => Err(unsafe_path(path, stage)),
    }
}

pub(super) fn local_regular_file_or_absent(path: &Path, stage: &str) -> Result<bool, CutError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(path, stage, error)),
        Ok(_) if record_recovery::is_plain_regular_file(path).unwrap_or(false) => Ok(true),
        Ok(_) => Err(unsafe_path(path, stage)),
    }
}

fn unsafe_path(path: &Path, stage: &str) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("could not {stage}: unsafe local path {}", path.display()),
        "check that the capture directory contains only local regular timing files",
    )
}

fn io_error(path: &Path, stage: &str, error: std::io::Error) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("could not {stage} at {}: {error}", path.display()),
        "check that the capture directory is writable, then retry recording",
    )
}
