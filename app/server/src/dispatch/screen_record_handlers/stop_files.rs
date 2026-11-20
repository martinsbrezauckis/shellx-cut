//! Bounded local-file facts used while stopping a capture.

use std::path::Path;

use cut_core::{error_codes, CutError};
use serde_json::Value;

/// The engine's capture marker holds a few fields.
const MAX_CAPTURE_MARKER_BYTES: u64 = 1024 * 1024;

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
            let bytes = match cut_core::store::read_bounded_cache(marker, MAX_CAPTURE_MARKER_BYTES)
            {
                cut_core::store::BoundedCacheRead::Bytes(bytes) => bytes,
                cut_core::store::BoundedCacheRead::Oversized => {
                    return Err(CutError::new(
                        error_codes::IO,
                        format!("could not read capture marker {}", marker.display()),
                        "the capture marker is larger than 1 MiB, so it is not the engine's",
                    ))
                }
                cut_core::store::BoundedCacheRead::Unavailable => {
                    return Err(CutError::new(
                        error_codes::IO,
                        format!("could not read capture marker {}", marker.display()),
                        "the capture marker must be a local regular file",
                    ))
                }
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_capture_marker_is_refused_unread() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(".capture.json");
        let mut body = br#"{"duration_ms":5000}"#.to_vec();
        std::fs::write(&marker, &body).unwrap();
        assert_eq!(capture_marker_duration(&marker).unwrap(), Some(5000));
        // The same marker followed by whitespace, one byte over the limit.
        body.resize(MAX_CAPTURE_MARKER_BYTES as usize + 1, b' ');
        std::fs::write(&marker, &body).unwrap();
        let error = capture_marker_duration(&marker).unwrap_err();
        assert!(error.cause.contains("1 MiB"), "{error:?}");
    }
}

/// What the recorder wrote about a take saved with a problem
/// (`capture-notes.json`); empty for an exact take or an unreadable file.
pub(super) fn capture_notes(capture_dir: &Path) -> Vec<String> {
    let path = capture_dir.join(record_capture::CAPTURE_NOTES_FILE);
    if !record_recovery::is_plain_regular_file(&path).unwrap_or(false) {
        return Vec::new();
    }
    let cut_core::store::BoundedCacheRead::Bytes(bytes) =
        cut_core::store::read_bounded_cache(&path, 64 * 1024)
    else {
        return Vec::new();
    };
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if value.get("schema").and_then(Value::as_str) != Some(record_capture::CAPTURE_NOTES_SCHEMA) {
        return Vec::new();
    }
    value
        .get("notes")
        .and_then(Value::as_array)
        .map(|notes| {
            notes
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The same notes for an agent, as the envelope's non-fatal warnings.
pub(super) fn notes_as_warnings(notes: &[String]) -> Vec<cut_core::VerbWarning> {
    notes
        .iter()
        .map(|note| cut_core::VerbWarning {
            code: "recording_saved_with_problem".into(),
            message: note.clone(),
            detail: serde_json::Map::new(),
        })
        .collect()
}
