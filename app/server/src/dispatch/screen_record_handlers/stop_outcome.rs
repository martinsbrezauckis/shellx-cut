//! What Stop and polish record about a take for the next start of the app.

use cut_core::{error_codes, CutError};
use std::path::Path;

/// `reason` on screen_record.stop: only `app_exit` (the app is closing during
/// the recording) is accepted. Returns whether the app is closing.
pub(super) fn app_exit(reason: Option<&str>) -> Result<bool, CutError> {
    match reason {
        None => Ok(false),
        Some("app_exit") => Ok(true),
        Some(other) => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown stop reason {other:?}"),
            "reason accepts only \"app_exit\"",
        )
        .with_suggested_action("omit reason, or pass \"app_exit\" when the app is closing")),
    }
}

/// A raw copy that could not be written does not lose the recording: say so,
/// and how to get the copy once the destination is fixed.
pub(super) fn take_kept_in_project(mut error: CutError) -> CutError {
    error.cause = format!(
        "{}; the recording itself is saved in the project and nothing was lost",
        error.cause
    );
    if error.suggested_action.is_none() {
        error.suggested_action = Some(
            "choose a folder you can write to, then run Stop again for this capture_id with that raw_path"
                .into(),
        );
    }
    error
}

/// Stop's wait ended while the recorder is still saving the take. Nothing is
/// lost and nothing about permissions is wrong: say that, and how to follow it.
pub(super) fn still_saving(capture_id: &str, waited_ms: u64, detail: String) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        "the recording is still being saved",
        format!(
            "Stop waited {} s for capture '{capture_id}' and the recorder is still finishing the file; {detail}",
            waited_ms / 1000
        ),
    )
    .with_suggested_action(
        "wait: screen_record.status shows when it is saved, and the take is then offered in Recording Recovery; a new recording can start once it is saved",
    )
}

/// A take saved while the app was closing has now reached the timeline.
pub(super) fn note_added_to_timeline(source: &Path) {
    if let Some(capture_id) = crate::screen_record::pending_recordings::capture_id_of_source(source)
    {
        crate::screen_record::pending_recordings::mark_added_to_timeline(&capture_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_app_exit_is_a_stop_reason() {
        assert!(!app_exit(None).unwrap());
        assert!(app_exit(Some("app_exit")).unwrap());
        assert_eq!(
            app_exit(Some("later")).unwrap_err().code,
            error_codes::INVALID_ARGS
        );
    }

    #[test]
    fn a_stop_that_waited_too_long_says_the_take_is_still_being_saved() {
        let error = still_saving("cap_1", 152_152, "log tail".into());
        assert_eq!(error.message, "the recording is still being saved");
        assert!(error.cause.contains("152 s"));
        let action = error.suggested_action.unwrap();
        assert!(!action.contains("ScreenCast") && !action.contains("System Settings"));
    }

    #[test]
    fn stop_returns_the_notes_of_a_take_saved_with_a_problem() {
        let dir = tempfile::tempdir().unwrap();
        assert!(super::super::stop_files::capture_notes(dir.path()).is_empty());
        std::fs::write(
            dir.path().join(record_capture::CAPTURE_NOTES_FILE),
            serde_json::to_vec(&serde_json::json!({
                "schema": record_capture::CAPTURE_NOTES_SCHEMA,
                "notes": ["The screen from 15.0 s to 30.0 s could not be saved."],
            }))
            .unwrap(),
        )
        .unwrap();
        let notes = super::super::stop_files::capture_notes(dir.path());
        assert_eq!(notes.len(), 1);
        let warnings = super::super::stop_files::notes_as_warnings(&notes);
        assert_eq!(warnings[0].code, "recording_saved_with_problem");
        std::fs::write(
            dir.path().join(record_capture::CAPTURE_NOTES_FILE),
            br#"{"schema":"other","notes":["x"]}"#,
        )
        .unwrap();
        assert!(super::super::stop_files::capture_notes(dir.path()).is_empty());
    }

    #[test]
    fn a_raw_copy_that_failed_says_the_recording_is_kept() {
        let error = take_kept_in_project(CutError::new(error_codes::IO, "x", "denied"));
        assert!(error.cause.contains("nothing was lost"));
        assert!(error.suggested_action.unwrap().contains("raw_path"));
    }
}
