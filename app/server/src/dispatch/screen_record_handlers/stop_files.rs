//! Bounded local-file facts used while stopping a capture.

use std::path::Path;

use cut_core::{error_codes, CutError};
use serde_json::Value;

pub(super) fn capture_marker_duration(marker: &Path) -> Result<Option<u64>, CutError> {
    match std::fs::symlink_metadata(marker) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(capture_file_error(marker, "inspect capture marker", error)),
        Ok(_) => {
            if !record_recovery::is_plain_regular_file(marker).map_err(|error| {
                capture_file_error(
                    marker,
                    "validate capture marker",
                    std::io::Error::other(error),
                )
            })? {
                return Err(CutError::new(
                    error_codes::IO,
                    format!("could not read capture marker {}", marker.display()),
                    "the capture marker must be a local regular file",
                ));
            }
            let bytes = std::fs::read(marker)
                .map_err(|error| capture_file_error(marker, "read capture marker", error))?;
            let marker: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            Ok(marker.get("duration_ms").and_then(Value::as_u64))
        }
    }
}

/// A live writer can be between journal appends, so malformed/torn state falls
/// back to the explicit minimum Stop budget.
pub(super) fn capture_started_unix_ms(capture_dir: &Path) -> Option<u64> {
    record_recovery::read_manifest(capture_dir)
        .ok()
        .map(|manifest| manifest.start.started_unix_ms)
        .filter(|started| *started > 0)
}

pub(super) fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

pub(super) fn local_regular_file_nonempty(path: &Path, stage: &str) -> Result<bool, CutError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(capture_file_error(path, stage, error)),
        Ok(metadata) => metadata,
    };
    if !record_recovery::is_plain_regular_file(path)
        .map_err(|error| capture_file_error(path, stage, std::io::Error::other(error)))?
    {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "could not {stage}: {} is not a local regular file",
                path.display()
            ),
            "capture files must remain inside the local capture directory",
        ));
    }
    Ok(metadata.len() > 0)
}

fn capture_file_error(path: &Path, stage: &str, error: std::io::Error) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("could not {stage} at {}: {error}", path.display()),
        "capture files must remain inside the local capture directory",
    )
}
