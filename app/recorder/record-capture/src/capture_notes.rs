//! What is wrong with a saved take, for the person and the agent.
//!
//! A recorder that saved a take with a problem (a part of the screen that could
//! not be read back, a duration that does not match the recording) keeps the
//! take and writes these sentences to `capture-notes.json` in the capture
//! folder. Stop returns them and the app shows them with the take.
#![cfg_attr(
    not(any(
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos"),
        all(target_os = "linux", feature = "capture-linux")
    )),
    allow(dead_code)
)]

use std::path::Path;

pub const CAPTURE_NOTES_FILE: &str = "capture-notes.json";
pub const CAPTURE_NOTES_SCHEMA: &str = "shellx-cut/capture-notes/1";

/// Write the notes; nothing is written for an exact take. A failure to write
/// them is logged in the capture folder's debug record, never fatal: the take
/// itself is already saved.
pub(crate) fn publish(out_dir: &Path, notes: &[String]) {
    if notes.is_empty() {
        return;
    }
    let body = serde_json::json!({ "schema": CAPTURE_NOTES_SCHEMA, "notes": notes });
    let written = serde_json::to_vec_pretty(&body)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            record_recovery::replace_synced(&out_dir.join(CAPTURE_NOTES_FILE), &bytes)
                .map_err(|error| error.to_string())
        });
    if let Err(error) = written {
        eprintln!("warning: could not write the capture notes: {error}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn notes_are_written_only_for_a_take_with_a_problem() {
        let dir = tempfile::tempdir().unwrap();
        super::publish(dir.path(), &[]);
        assert!(!dir.path().join(super::CAPTURE_NOTES_FILE).exists());
        super::publish(
            dir.path(),
            &["The screen from 1.0 s to 2.0 s could not be saved.".into()],
        );
        let written: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dir.path().join(super::CAPTURE_NOTES_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(written["schema"], super::CAPTURE_NOTES_SCHEMA);
        assert_eq!(written["notes"].as_array().unwrap().len(), 1);
    }
}
