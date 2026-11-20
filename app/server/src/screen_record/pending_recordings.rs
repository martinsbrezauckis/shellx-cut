//! Recordings the person has not been told about yet.
//!
//! A recording lives inside its project's cache, so after a crash, a forced
//! quit, or a power cut nothing outside that project knows it was running, and
//! the next start of the app (which opens no project) cannot offer it. This
//! module keeps one small note per recording in the app's own data folder
//! (`<cut home>/recordings/pending/<capture_id>.json`):
//!
//! - written when a recording starts, before the native capture opens;
//! - removed when Stop saves it normally, or when it recorded nothing;
//! - kept, marked `saved_on_close`, when Stop ran because the app was closing,
//!   so the next start can say what happened (and whether the take reached the
//!   timeline);
//! - kept, marked `failed`, when the recording ended with an error after part
//!   of it was saved;
//! - left as `recording` when the process died: no code ran, and the next
//!   start finds a note for a recording that is no longer running.
//!
//! `screen_record.pending` lists the notes with what can be done, and
//! `screen_record.recover` / `screen_record.pending_dismiss` resolve them.

use std::path::{Path, PathBuf};

use cut_core::{error_codes, CutError, VerbResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::recovery::audio_parts;
use super::recovery::validate_capture_id;

const SCHEMA: &str = "shellx-cut/pending-recording/1";
const MAX_NOTES: usize = 256;
/// Below this much PCM (about a quarter second of 48 kHz stereo) a sound file
/// is not worth offering.
const MIN_AUDIO_BYTES: u64 = 48_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Recording,
    SavedOnClose,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Note {
    pub schema: String,
    pub capture_id: String,
    pub project_dir: PathBuf,
    pub started_unix_ms: u64,
    pub outcome: Outcome,
    #[serde(default)]
    pub added_to_timeline: bool,
}

fn notes_dir() -> Option<PathBuf> {
    crate::userdata::shellx_cut_home().map(|home| home.join("recordings").join("pending"))
}

fn note_path(capture_id: &str) -> Option<PathBuf> {
    validate_capture_id(capture_id).ok()?;
    notes_dir().map(|dir| dir.join(format!("{capture_id}.json")))
}

fn read_note(path: &Path) -> Option<Note> {
    let bytes = std::fs::read(path).ok()?;
    let note: Note = serde_json::from_slice(&bytes).ok()?;
    (note.schema == SCHEMA && validate_capture_id(&note.capture_id).is_ok()).then_some(note)
}

fn write_note(note: &Note) {
    let Some(path) = note_path(&note.capture_id) else {
        return;
    };
    let result = path
        .parent()
        .map(std::fs::create_dir_all)
        .unwrap_or(Ok(()))
        .and_then(|()| {
            let bytes = serde_json::to_vec_pretty(note).map_err(std::io::Error::other)?;
            record_recovery::replace_synced(&path, &bytes)
        });
    if let Err(error) = result {
        // The recording itself is unaffected; only the next-start notice is.
        tracing::warn!(capture_id = %note.capture_id, error = %error, "could not write the pending-recording note");
    }
}

pub(crate) fn remove(capture_id: &str) {
    if let Some(path) = note_path(capture_id) {
        let _ = std::fs::remove_file(path);
    }
}

fn update(capture_id: &str, change: impl FnOnce(&mut Note)) {
    let Some(path) = note_path(capture_id) else {
        return;
    };
    if let Some(mut note) = read_note(&path) {
        change(&mut note);
        write_note(&note);
    }
}

/// A recording is about to open its native capture.
pub(crate) fn note_started(capture_id: &str, project_dir: &Path) {
    write_note(&Note {
        schema: SCHEMA.into(),
        capture_id: capture_id.into(),
        project_dir: super::strip_verbatim_prefix(project_dir),
        started_unix_ms: super::active_capture::unix_ms_now(),
        outcome: Outcome::Recording,
        added_to_timeline: false,
    });
}

/// Stop saved the recording. When the app was closing (`app_exit`) the note
/// stays so the next start can tell the person what happened.
pub(crate) fn after_stop_saved(capture_id: &str, app_exit: bool) {
    if app_exit {
        update(capture_id, |note| note.outcome = Outcome::SavedOnClose);
    } else {
        remove(capture_id);
    }
}

/// The capture worker reported a failure. With nothing recorded there is
/// nothing to offer later.
pub(crate) fn after_failure(capture_id: &str, nothing_recorded: bool) {
    if nothing_recorded {
        remove(capture_id);
    } else {
        update(capture_id, |note| note.outcome = Outcome::Failed);
    }
}

/// The take of a recording saved at close reached the timeline.
pub(crate) fn mark_added_to_timeline(capture_id: &str) {
    update(capture_id, |note| note.added_to_timeline = true);
}

/// The capture id that owns `source` when it is a file directly inside a
/// recording folder (`.../cache/screen_record/<capture_id>/<file>`).
pub(crate) fn capture_id_of_source(source: &Path) -> Option<String> {
    let id = source.parent()?.file_name()?.to_str()?;
    validate_capture_id(id).ok().map(|()| id.to_string())
}

fn project_name(dir: &Path) -> String {
    dir.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.trim_end_matches(".cutproj").to_string())
        .unwrap_or_else(|| dir.display().to_string())
}

fn audio_part_bytes(capture_dir: &Path) -> u64 {
    ["mic-wav", "system-wav"]
        .iter()
        .filter_map(|prefix| audio_parts::find_part(capture_dir, prefix))
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .max()
        .unwrap_or(0)
}

/// Whether anything playable can be made from the files in `capture_dir`
/// that has not already been added to the timeline by screen_record.recover.
/// Read-only and process-free: never probes media or changes a file.
/// Written when a Stop came before the first screen frame: whatever the native
/// layer wrote after that request is not a take and is never offered.
pub(crate) const NOTHING_RECORDED_MARKER: &str = "nothing-recorded.json";

pub(crate) fn mark_nothing_recorded(capture_dir: &Path) {
    let _ = record_recovery::replace_synced(
        &capture_dir.join(NOTHING_RECORDED_MARKER),
        br#"{"reason":"stop came before the first screen frame"}"#,
    );
}

pub(crate) fn has_recoverable_media(capture_dir: &Path) -> bool {
    let marked = |name: &str| {
        cut_core::store::plain_file_state(&capture_dir.join(name))
            == cut_core::store::PlainFileState::Plain
    };
    if marked(super::recover_take::ADDED_MARKER) || marked(NOTHING_RECORDED_MARKER) {
        // Already recovered to the timeline, or never a take.
        return false;
    }
    let status = record_recovery::recovery_status(capture_dir);
    let video = status
        .receipt
        .as_ref()
        .is_some_and(|receipt| receipt.source.is_some())
        || (status.receipt.is_none() && status.checkpoints > 0);
    let plain = |name: &str| {
        cut_core::store::plain_file_state(&capture_dir.join(name))
            == cut_core::store::PlainFileState::Plain
    };
    video
        || plain("mic.wav")
        || plain("system.wav")
        || audio_part_bytes(capture_dir) > MIN_AUDIO_BYTES
}

fn capture_dir_for(note: &Note) -> PathBuf {
    note.project_dir
        .join("cache")
        .join("screen_record")
        .join(&note.capture_id)
}

/// What recovery would keep against what was recorded, from the files alone
/// (nothing is probed or changed). The same measure `screen_record.recover`
/// reports afterwards, so the notice and the reply say the same thing.
fn extent_before_recover(folder: &Path) -> Option<super::recovered_extent::Extent> {
    let status = record_recovery::recovery_status(folder);
    // Finished checkpoints are candidates, not measured recovery. Without a
    // receipt, measure() would mistake the absent kept-segment count for zero.
    if status.receipt.is_none() && status.checkpoints > 0 {
        return None;
    }
    let video = status
        .receipt
        .as_ref()
        .is_some_and(|receipt| receipt.source.is_some())
        || (status.receipt.is_none() && status.checkpoints > 0);
    let plain = |name: &str| {
        let path = folder.join(name);
        (cut_core::store::plain_file_state(&path) == cut_core::store::PlainFileState::Plain)
            .then_some(path)
    };
    let mic = plain("mic.wav").or_else(|| audio_parts::find_part(folder, "mic-wav"));
    let system = plain("system.wav").or_else(|| audio_parts::find_part(folder, "system-wav"));
    Some(super::recovered_extent::measure(
        folder,
        video,
        &[mic.as_deref(), system.as_deref()],
    ))
}

/// One note as a person and an agent read it.
fn describe(note: &Note) -> Option<Value> {
    if super::capture_registry::is_capture_active(&note.capture_id) {
        // Still recording in this process: not interrupted.
        return None;
    }
    let project = project_name(&note.project_dir);
    let folder = capture_dir_for(note);
    let files_exist = folder.is_dir();
    let status = files_exist.then(|| record_recovery::recovery_status(&folder));
    let can_recover = files_exist && has_recoverable_media(&folder);
    let finished = status
        .as_ref()
        .and_then(|status| status.receipt.as_ref())
        .is_some_and(|receipt| receipt.state == record_recovery::RecoveryState::Complete);
    let (situation, message, action) = match (&note.outcome, files_exist) {
        (_, false) => (
            "files_missing",
            format!("A recording in “{project}” was interrupted, but its files are no longer there."),
            "dismiss this notice with screen_record.pending_dismiss",
        ),
        (Outcome::SavedOnClose, true) if note.added_to_timeline => (
            "saved_on_close",
            format!("ShellX Cut was closed during a recording in “{project}”. The recording was saved and added to the timeline."),
            "nothing to do; dismiss this notice with screen_record.pending_dismiss",
        ),
        (Outcome::SavedOnClose, true) => (
            "saved_on_close",
            format!("ShellX Cut was closed during a recording in “{project}”. The recording was saved but is not on the timeline yet."),
            "open the project with project.open, then add the take with screen_record.recover",
        ),
        (Outcome::Recording, true) if finished => (
            "saved_not_added",
            format!("A recording in “{project}” finished, but it was not added to the timeline."),
            "open the project with project.open, then add the take with screen_record.recover",
        ),
        (Outcome::Failed, true) if can_recover => (
            "failed",
            format!("A recording in “{project}” stopped with an error. The part recorded before the error was saved."),
            "open the project with project.open, then add what was saved with screen_record.recover",
        ),
        (Outcome::Recording, true) if can_recover => (
            "interrupted",
            format!("ShellX Cut closed while recording in “{project}”. What was saved can be added to the timeline."),
            "open the project with project.open, then add what was saved with screen_record.recover",
        ),
        (_, true) => (
            "nothing_saved",
            format!("A recording in “{project}” was interrupted before any part of it was saved. Nothing can be recovered."),
            "dismiss this notice with screen_record.pending_dismiss; the leftover files stay in files_folder",
        ),
    };
    // How much of the recording can be saved, said before recover is pressed.
    let can_recover_now =
        can_recover && !(note.outcome == Outcome::SavedOnClose && note.added_to_timeline);
    let extent = can_recover_now
        .then(|| extent_before_recover(&folder))
        .flatten();
    let extent_sentence = extent
        .map(|extent| extent.notice_sentence())
        .unwrap_or_else(|| {
            if can_recover_now {
                "How much of it was saved has not been measured yet.".into()
            } else {
                String::new()
            }
        });
    let message = if extent_sentence.is_empty() {
        message
    } else {
        format!("{message} {extent_sentence}")
    };
    let mut row = json!({
        "capture_id": note.capture_id,
        "project_path": note.project_dir.display().to_string(),
        "project_name": project,
        "started_unix_ms": note.started_unix_ms,
        "situation": situation,
        "added_to_timeline": note.added_to_timeline,
        "can_recover": can_recover && !(note.outcome == Outcome::SavedOnClose && note.added_to_timeline),
        "recovery_state": status.as_ref().map(|status| serde_json::to_value(&status.state).unwrap_or(Value::Null)),
        "checkpoints": status.as_ref().map(|status| status.checkpoints),
        "files_folder": folder.display().to_string(),
        "message": message,
        "suggested_action": action,
    });
    if can_recover_now {
        row["recovered_extent_state"] = json!(if extent.is_some() {
            "measured"
        } else {
            "not_measured"
        });
    }
    if let (Some(extent), false) = (extent, extent_sentence.is_empty()) {
        row["recovered_ms"] = json!(extent.recovered_ms);
        row["recorded_ms_at_least"] = json!(extent.recorded_ms_at_least);
        row["recorded_longer_unknown"] = json!(extent.longer_unknown);
    }
    Some(row)
}

fn all_notes() -> Vec<Note> {
    let Some(dir) = notes_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut notes: Vec<Note> = entries
        .flatten()
        .take(MAX_NOTES)
        .filter_map(|entry| read_note(&entry.path()))
        .collect();
    notes.sort_by_key(|note| std::cmp::Reverse(note.started_unix_ms));
    notes
}

/// `screen_record.pending{}`: recordings the person has not been told about.
pub(crate) async fn pending_handler(args: Value) -> Result<VerbResult, CutError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {}
    let _: Args = crate::dispatch::parse_args(args)?;
    let recordings: Vec<Value> = all_notes().iter().filter_map(describe).collect();
    Ok(VerbResult::ok(json!({ "recordings": recordings })))
}

/// `screen_record.pending_dismiss{capture_id}`: forget the notice. The
/// recording's files are not touched.
pub(crate) async fn dismiss_handler(args: Value) -> Result<VerbResult, CutError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        capture_id: String,
    }
    let args: Args = crate::dispatch::parse_args(args)?;
    validate_capture_id(&args.capture_id)?;
    let path = note_path(&args.capture_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "there is no notice for this recording",
            "the app data folder is not available",
        )
    })?;
    let Some(note) = read_note(&path) else {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "there is no notice for this recording",
            format!("no pending note for {}", args.capture_id),
        )
        .with_suggested_action("list current notices with screen_record.pending"));
    };
    if super::capture_registry::is_capture_active(&note.capture_id) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "this recording is still running",
            "a running recording has no notice to dismiss",
        )
        .with_suggested_action("stop the recording with screen_record.stop"));
    }
    remove(&note.capture_id);
    Ok(VerbResult::ok(json!({
        "dismissed": true,
        "capture_id": note.capture_id,
        "files_folder": capture_dir_for(&note).display().to_string(),
    })))
}

#[cfg(test)]
#[path = "pending_recordings_tests.rs"]
mod tests;
