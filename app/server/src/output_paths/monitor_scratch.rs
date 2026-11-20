//! The app's playback audio files (item 98): the Preview's monitor mix, the
//! Mixer's per-track meter stems and the track Listen button.
//!
//! These were ordinary `export.audio` exports, so every edit added another
//! `audio_a1t-N.wav` or `_monitor_b.mp3` to the project's `exports/` folder,
//! where the person keeps their deliveries. With `scratch` set, export.audio
//! writes into the ENGINE's private working folder (cut_media::private_work),
//! subfolder `monitor/`, under a fixed name per slot; each new mix replaces the
//! last one by rename from a fresh private name.
//!
//! Nothing is created, written or deleted inside the project: a project folder
//! may come from someone else, and a planted `.cache` link or junction would
//! otherwise send a create or a recursive delete to a folder outside it.
//!
//! The files are removed when the project closes, when another project starts
//! using scratch audio in this engine, and with the whole private folder when
//! the engine shuts down (a crashed engine's folder is swept later by
//! private_work). The `monitor/` folder is a read root of `/api/export-file`.

use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Subfolder of the engine's private folder.
const MONITOR: &str = "monitor";

/// The project whose playback audio the monitor folder holds now.
static CURRENT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Admit `slot` (1 to 32 of a-z, 0-9, _) and return the scratch file path
/// `<private>/monitor/<slot>.<ext>` for `project_dir`'s playback audio.
pub(crate) fn scratch_output(
    project_dir: &Path,
    slot: &str,
    ext: &str,
) -> Result<PathBuf, CutError> {
    if slot.is_empty()
        || slot.len() > 32
        || !slot
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("scratch slot '{slot}' is not a plain name"),
            "scratch is 1 to 32 characters of a-z, 0-9 and _",
        )
        .with_suggested_action("pass a short name such as \"monitor_a\" or \"stem_a1t\""));
    }
    let mut current = CURRENT.lock().unwrap_or_else(|p| p.into_inner());
    if current.as_deref() != Some(project_dir) {
        // Another project's playback audio from this engine goes first.
        remove_monitor();
        *current = Some(project_dir.to_path_buf());
    }
    let dir = cut_media::private_work::session_subdir(MONITOR)?;
    Ok(dir.join(format!("{slot}.{ext}")))
}

/// The monitor folder when it exists (a read root of `/api/export-file`).
pub(crate) fn monitor_dir() -> Option<PathBuf> {
    let dir = cut_media::private_work::session_dir().ok()?.join(MONITOR);
    dir.is_dir().then_some(dir)
}

/// A fresh private file beside `final_path` (same extension, so ffmpeg picks
/// the format) that the caller renders into and then renames onto
/// `final_path`; it is deleted if the render fails.
pub(crate) fn private_temp(final_path: &Path) -> Result<tempfile::TempPath, CutError> {
    let dir = final_path.parent().unwrap_or_else(|| Path::new("."));
    let ext = final_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    Ok(tempfile::Builder::new()
        .prefix(".cut-scratch-")
        .suffix(&ext)
        .tempfile_in(dir)?
        .into_temp_path())
}

/// Remove the playback audio (project close).
pub(crate) fn clear_all() {
    let mut current = CURRENT.lock().unwrap_or_else(|p| p.into_inner());
    remove_monitor();
    *current = None;
}

/// Remove the whole private folder (engine shutdown).
pub(crate) fn shutdown() {
    clear_all();
    cut_media::private_work::remove_session_dir();
}

fn remove_monitor() {
    if let Some(dir) = monitor_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scratch registry and the private folder are process-wide; these
    /// tests take turns.
    static TURN: Mutex<()> = Mutex::new(());

    fn sentinel_outside(root: &Path) -> PathBuf {
        let outside = root.join("outside");
        std::fs::create_dir_all(outside.join("monitor")).unwrap();
        std::fs::write(outside.join("monitor").join("sentinel.txt"), b"keep").unwrap();
        outside
    }

    fn use_and_clear(project: &Path) {
        let path = scratch_output(project, "monitor_a", "mp3").unwrap();
        std::fs::write(&path, b"mix").unwrap();
        assert!(
            !path.starts_with(project),
            "scratch audio is never in the project"
        );
        clear_all();
        assert!(!path.exists());
    }

    #[test]
    fn scratch_is_private_and_cleared_per_project() {
        let _turn = TURN.lock().unwrap_or_else(|p| p.into_inner());
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a.cutproj");
        let b = root.path().join("b.cutproj");
        let pa = scratch_output(&a, "monitor_a", "mp3").unwrap();
        std::fs::write(&pa, b"a").unwrap();
        let pb = scratch_output(&b, "stem_a1t", "wav").unwrap();
        assert!(!pa.exists(), "another project's playback audio goes first");
        std::fs::write(&pb, b"b").unwrap();
        clear_all();
        assert!(!pb.exists());
        assert_eq!(
            scratch_output(&a, "../x", "mp3").unwrap_err().code,
            error_codes::INVALID_ARGS
        );
        clear_all();
    }

    /// Security review: a received project whose `.cache` is a link to an
    /// outside folder with a `monitor` subfolder. The old code deleted
    /// `<project>/.cache/monitor` and so the outside folder.
    #[cfg(unix)]
    #[test]
    fn linked_cache_folder_is_never_touched() {
        let _turn = TURN.lock().unwrap_or_else(|p| p.into_inner());
        let root = tempfile::tempdir().unwrap();
        let outside = sentinel_outside(root.path());
        let p = root.path().join("p.cutproj");
        std::fs::create_dir_all(&p).unwrap();
        std::os::unix::fs::symlink(&outside, p.join(".cache")).unwrap();
        use_and_clear(&p);
        assert!(outside.join("monitor").join("sentinel.txt").exists());
    }

    /// The same with `.cache/monitor` itself the link.
    #[cfg(unix)]
    #[test]
    fn linked_monitor_folder_is_never_touched() {
        let _turn = TURN.lock().unwrap_or_else(|p| p.into_inner());
        let root = tempfile::tempdir().unwrap();
        let outside = sentinel_outside(root.path());
        let p = root.path().join("p.cutproj");
        std::fs::create_dir_all(p.join(".cache")).unwrap();
        std::os::unix::fs::symlink(outside.join("monitor"), p.join(".cache").join("monitor"))
            .unwrap();
        use_and_clear(&p);
        assert!(outside.join("monitor").join("sentinel.txt").exists());
    }

    /// A file in the scratch folder hard-linked to a file outside it (made on
    /// the same volume, beside the monitor folder): clearing removes the name
    /// only; the other file keeps its content.
    #[test]
    fn hard_linked_scratch_file_only_loses_its_name() {
        let _turn = TURN.lock().unwrap_or_else(|p| p.into_inner());
        let root = tempfile::tempdir().unwrap();
        let p = root.path().join("p.cutproj");
        let path = scratch_output(&p, "stem_a1t", "wav").unwrap();
        let target = path
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join("outside-sentinel.txt");
        std::fs::write(&target, b"keep").unwrap();
        std::fs::hard_link(&target, &path).unwrap();
        clear_all();
        assert!(!path.exists());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        std::fs::remove_file(&target).unwrap();
    }

    /// Windows: a junction (no privilege needed) at the project's `.cache`.
    #[cfg(windows)]
    #[test]
    #[ignore = "creates a directory junction with cmd mklink /J on an NTFS volume"]
    fn windows_junction_cache_folder_is_never_touched() {
        let _turn = TURN.lock().unwrap_or_else(|p| p.into_inner());
        let root = tempfile::tempdir().unwrap();
        let outside = sentinel_outside(root.path());
        let p = root.path().join("p.cutproj");
        std::fs::create_dir_all(&p).unwrap();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(p.join(".cache"))
            .arg(&outside)
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed: {status}");
        use_and_clear(&p);
        assert!(outside.join("monitor").join("sentinel.txt").exists());
    }
}
