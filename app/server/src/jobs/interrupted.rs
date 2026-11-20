//! Work someone asked for that a quit without asking, a crash, a kill or a
//! power cut cut off.
//!
//! Job records live in the project (`<project>/jobs/`), and a record left
//! running is turned into "interrupted by restart" only when that project is
//! opened again. The app starts with no project open, so nobody was told that
//! an export was lost. While a job someone asked for (see `consent`) runs, a
//! small note sits in the app's own data folder
//! (`<cut home>/jobs/running/<pid>-<project>-<job>.json`); the job's normal
//! end removes it. A note whose engine process is gone names an interrupted
//! job: `jobs.interrupted` lists it with a sentence for the person, and
//! `jobs.interrupted_dismiss` forgets it.
//!
//! An export cut off this way leaves its own staging files beside the output
//! (`.cut-ffmpeg-*.tmp.*`, `.<name>.shellx-cut-reservation.lock`,
//! `.<name>.shellx-cut-reserving`). When an interrupted export is listed, the
//! engine removes those files from that project's `exports` folder: only names
//! it writes, only ordinary files, never while any export in that folder still
//! holds its lock.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SCHEMA: &str = "shellx-cut/running-job-note/1";
const MAX_NOTES: usize = 256;
const MAX_NOTE_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Note {
    pub schema: String,
    pub project_dir: PathBuf,
    pub job_id: String,
    pub kind: String,
    pub started_unix_ms: u64,
    pub owner_pid: u32,
    pub owner_identity: String,
    /// Leftover staging and lock files already removed for this job.
    #[serde(default)]
    pub leftover_files_removed: usize,
    /// The engine run's mark its helpers carry (`engine_lifeline`), so the
    /// next start can stop a helper that outlived it before removing files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_run: Option<String>,
    /// The folders the job could write its export into: the project's
    /// `exports` and the app-wide export folder when one is chosen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub export_dirs: Vec<PathBuf>,
    /// Helpers of the dead run that were still running and were stopped.
    #[serde(default)]
    pub helpers_stopped: usize,
}

/// This engine process: id and start-time identity, as the recording journal
/// records its owner.
fn this_run() -> &'static (u32, String) {
    static RUN: OnceLock<(u32, String)> = OnceLock::new();
    RUN.get_or_init(|| {
        let start = record_recovery::CaptureStart::new("running-jobs", 0);
        (start.owner_pid, start.owner_identity)
    })
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|value| u64::try_from(value.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn notes_dir() -> Option<PathBuf> {
    crate::userdata::shellx_cut_home().map(|home| home.join("jobs").join("running"))
}

fn project_tag(project_dir: &Path) -> String {
    let digest = Sha256::digest(project_dir.to_string_lossy().as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn safe_job_id(job_id: &str) -> bool {
    !job_id.is_empty()
        && job_id.len() <= 64
        && job_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn note_path(project_dir: &Path, job_id: &str) -> Option<PathBuf> {
    if !safe_job_id(job_id) {
        return None;
    }
    let (pid, _) = this_run();
    notes_dir().map(|dir| dir.join(format!("{pid}-{}-{job_id}.json", project_tag(project_dir))))
}

/// The key a person or agent uses to dismiss one note: its file stem.
fn key_of(path: &Path) -> Option<String> {
    path.file_stem()?.to_str().map(str::to_string)
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A job someone asked for was created for the project in `project_dir`.
pub(crate) fn note_started(project_dir: &Path, job_id: &str, kind: &str) {
    if !super::consent::asks_before_cancel(kind) {
        return;
    }
    let Some(path) = note_path(project_dir, job_id) else {
        return;
    };
    let (pid, identity) = this_run();
    let note = Note {
        schema: SCHEMA.into(),
        project_dir: project_dir.to_path_buf(),
        job_id: job_id.into(),
        kind: kind.into(),
        started_unix_ms: unix_ms_now(),
        owner_pid: *pid,
        owner_identity: identity.clone(),
        leftover_files_removed: 0,
        engine_run: crate::engine_lifeline::this_run_mark(),
        export_dirs: export_folders(project_dir),
        helpers_stopped: 0,
    };
    let result = path
        .parent()
        .map(std::fs::create_dir_all)
        .unwrap_or(Ok(()))
        .and_then(|()| {
            let bytes = serde_json::to_vec(&note).map_err(std::io::Error::other)?;
            record_recovery::replace_synced(&path, &bytes)
        });
    if let Err(error) = result {
        // The job itself runs on; only a later "interrupted" notice is lost.
        tracing::warn!(job_id, error = %error, "could not write the running-job note");
    }
}

/// The job ended (done, failed, cancelled, replaced): nothing to tell later.
pub(crate) fn note_ended(project_dir: &Path, job_id: &str) {
    if let Some(path) = note_path(project_dir, job_id) {
        let _ = std::fs::remove_file(path);
    }
}

fn read_note(path: &Path) -> Option<Note> {
    let mut file = cut_core::store::open_plain_file(path).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_NOTE_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    let note: Note = serde_json::from_slice(&bytes).ok()?;
    (note.schema == SCHEMA && safe_job_id(&note.job_id)).then_some(note)
}

/// Whether the engine that wrote `note` is gone. Notes of this process are
/// running work; another live engine's notes are its running work too.
fn owner_gone(note: &Note) -> bool {
    let (pid, identity) = this_run();
    if note.owner_pid == *pid && &note.owner_identity == identity {
        return false;
    }
    let start = record_recovery::CaptureStart {
        schema: String::new(),
        capture_id: String::new(),
        owner_pid: note.owner_pid,
        owner_identity: note.owner_identity.clone(),
        owner_nonce: String::new(),
        started_unix_ms: note.started_unix_ms,
        checkpoint_interval_ms: 0,
        output_cadence: None,
    };
    record_recovery::owner_state(&start) == record_recovery::OwnerState::Dead
}

/// Notes of jobs whose engine is gone, oldest first, with their keys.
pub(crate) fn interrupted() -> Vec<(String, Note)> {
    let Some(dir) = notes_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, Note)> = entries
        .flatten()
        .take(MAX_NOTES)
        .filter_map(|entry| {
            let path = entry.path();
            let key = key_of(&path)?;
            let note = read_note(&path)?;
            (valid_key(&key) && owner_gone(&note)).then_some((key, note))
        })
        .collect();
    found.sort_by_key(|(_, note)| note.started_unix_ms);
    found
}

/// Forget one interrupted-job note. Returns false when there is none.
pub(crate) fn dismiss(key: &str) -> bool {
    if !valid_key(key) {
        return false;
    }
    let Some(dir) = notes_dir() else {
        return false;
    };
    let path = dir.join(format!("{key}.json"));
    match read_note(&path) {
        Some(note) if owner_gone(&note) => std::fs::remove_file(path).is_ok(),
        _ => false,
    }
}

/// `.cut-ffmpeg-<random>.tmp.<ext>` (ffmpeg staging) and
/// `.<stem>.<pid>.<n>.tmp[.<ext>]` (other atomic writes), the two staging
/// names the engine gives a file it has not published yet.
fn is_staging_temp(name: &str) -> bool {
    if name.starts_with(".cut-ffmpeg-") && name.contains(".tmp.") {
        return true;
    }
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let parts: Vec<&str> = rest.split('.').collect();
    let Some(tmp_at) = parts.iter().rposition(|part| *part == "tmp") else {
        return false;
    };
    let ext_ok = match parts.len() - tmp_at {
        1 => true,
        2 => {
            let ext = parts[tmp_at + 1];
            !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }
        _ => false,
    };
    let digits = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
    ext_ok && tmp_at >= 3 && digits(parts[tmp_at - 1]) && digits(parts[tmp_at - 2])
}

fn is_reservation_file(name: &str) -> bool {
    name.starts_with('.')
        && (name.ends_with(".shellx-cut-reservation.lock")
            || name.ends_with(".shellx-cut-reserving"))
}

fn plain(path: &Path) -> bool {
    cut_core::store::plain_file_state(path) == cut_core::store::PlainFileState::Plain
}

/// True when nobody holds the advisory lock of this reservation file: its
/// export is no longer running (the OS releases the lock when a process dies).
fn lock_is_free(path: &Path) -> bool {
    use fs2::FileExt;
    let Ok(file) = cut_core::store::open_plain_file(path) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = fs2::FileExt::unlock(&file);
            true
        }
        Err(_) => false,
    }
}

/// Where a job of this project can write an export: the project's `exports`
/// and the app-wide export folder when one is chosen.
fn export_folders(project_dir: &Path) -> Vec<PathBuf> {
    let mut folders = vec![project_dir.join("exports")];
    if let Some(chosen) = crate::output_paths::app_export_folder() {
        if !folders.contains(&chosen) {
            folders.push(chosen);
        }
    }
    folders
}

/// A `.<stem>.<pid>.<n>.tmp` name written by this very process: its own work.
fn written_by_this_process(name: &str) -> bool {
    let own = format!(".{}.", std::process::id());
    name.contains(&own) && is_staging_temp(name) && !name.starts_with(".cut-ffmpeg-")
}

/// Remove the staging files an interrupted export left in `folder`, by the
/// names the engine writes. Nothing is removed while any reservation in the
/// folder is still locked (an export is running there), nothing this process
/// wrote, and only files last changed before `before`. Returns the removed
/// names.
pub(crate) fn sweep_export_leftovers(folder: &Path, before: SystemTime) -> Vec<String> {
    if !record_recovery::is_plain_dir(folder).unwrap_or(false) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let candidates: Vec<(String, PathBuf)> = entries
        .flatten()
        .take(4_096)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            ((is_staging_temp(&name) || is_reservation_file(&name))
                && !written_by_this_process(&name))
            .then(|| (name, entry.path()))
        })
        .filter(|(_, path)| plain(path))
        .collect();
    let reservations_free = candidates
        .iter()
        .filter(|(name, _)| name.ends_with(".shellx-cut-reservation.lock"))
        .all(|(_, path)| lock_is_free(path));
    if !reservations_free {
        return Vec::new();
    }
    let mut removed = Vec::new();
    for (name, path) in candidates {
        let old_enough = std::fs::symlink_metadata(&path)
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| modified < before);
        if old_enough && std::fs::remove_file(&path).is_ok() {
            removed.push(name);
        }
    }
    removed
}

/// Interrupted jobs, after stopping any helper of the dead run that is still
/// running (an export's ffmpeg it started) and then removing that export's
/// unfinished files from each folder it could write to. The engine runs this
/// once at start, before any work of its own; counts are kept in the note.
pub(crate) fn interrupted_after_cleanup() -> Vec<(String, Note)> {
    let listed = cleanup(true);
    // Then whatever else a dead run left running (work no one is told about).
    let others = crate::engine_lifeline::stop_dead_runs();
    if others > 0 {
        tracing::info!(
            stopped = others,
            "stopped helpers a killed engine left running"
        );
    }
    listed
}

/// `jobs.interrupted`: tidies up again only when this engine runs no work
/// (a running export of this run holds files in the same folders).
pub(crate) fn interrupted_for_listing(may_tidy: bool) -> Vec<(String, Note)> {
    cleanup(may_tidy)
}

fn cleanup(tidy: bool) -> Vec<(String, Note)> {
    let Some(dir) = notes_dir() else {
        return Vec::new();
    };
    interrupted()
        .into_iter()
        .map(|(key, mut note)| {
            if !tidy {
                return (key, note);
            }
            // A writer that is still alive would refill what is removed, and
            // the notice would say "removed" untruly: stop it first.
            let stopped = note
                .engine_run
                .as_deref()
                .map(crate::engine_lifeline::stop_marked)
                .unwrap_or(0);
            let before = SystemTime::now();
            let folders = if note.export_dirs.is_empty() {
                vec![note.project_dir.join("exports")]
            } else {
                note.export_dirs.clone()
            };
            let removed: Vec<String> = folders
                .iter()
                .flat_map(|folder| sweep_export_leftovers(folder, before))
                .collect();
            if stopped > 0 || !removed.is_empty() {
                tracing::info!(job_id = %note.job_id, stopped, removed = ?removed, "tidied up after an interrupted export");
                note.helpers_stopped += stopped;
                note.leftover_files_removed += removed.len();
                if let Ok(bytes) = serde_json::to_vec(&note) {
                    let _ = record_recovery::replace_synced(&dir.join(format!("{key}.json")), &bytes);
                }
            }
            (key, note)
        })
        .collect()
}

#[cfg(test)]
#[path = "interrupted_tests.rs"]
mod tests;
