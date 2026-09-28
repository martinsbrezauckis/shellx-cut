//! Classification helpers that keep v1 checkpoint recovery out of pause-owned captures.

use std::io::Read;
use std::path::Path;

use record_recovery::{is_plain_regular_file, CaptureRoot, RECORDING_SESSION_JOURNAL_FILE};

pub(super) const MAX_OWNERSHIP_JOURNAL_BYTES: u64 = 8 * 1024 * 1024;
pub(super) const MAX_COMPLETION_PROJECT_BYTES: u64 = 4 * 1024 * 1024;

pub(super) enum PauseSessionOwnership {
    Absent,
    Present,
}

/// The v1 scanner does not parse, repair, or consume a pause-session journal.
/// Its mere local presence reserves the capture for the pause-aware owner.
pub(super) fn pause_session_ownership(
    root: &Path,
) -> Result<PauseSessionOwnership, std::io::Error> {
    let path = root.join(RECORDING_SESSION_JOURNAL_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(PauseSessionOwnership::Absent)
        }
        Err(error) => Err(error),
        Ok(_) if is_plain_regular_file(&path).map_err(std::io::Error::other)? => {
            let capture_id = root
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| std::io::Error::other("pause-session capture id is unavailable"))?;
            if requires_input_sidecars(&path)? {
                let project_dir = root
                    .parent()
                    .and_then(|path| path.parent())
                    .and_then(|path| path.parent())
                    .ok_or_else(|| {
                        std::io::Error::other("pause-session project root is unavailable")
                    })?;
                let capture_root = CaptureRoot::open_existing(project_dir)
                    .map_err(std::io::Error::other)?
                    .ok_or_else(|| {
                        std::io::Error::other("pause-session capture root is unavailable")
                    })?;
                super::super::windows_pause_input_sidecar::verify_recovery_capture(
                    &capture_root,
                    capture_id,
                )
                .map_err(std::io::Error::other)?;
            }
            Ok(PauseSessionOwnership::Present)
        }
        Ok(_) => Err(std::io::Error::other(
            "pause-session journal is linked or not a local regular file",
        )),
    }
}

fn requires_input_sidecars(path: &Path) -> Result<bool, std::io::Error> {
    let mut options = std::fs::OpenOptions::new();
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
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other(
            "pause-session journal is not regular",
        ));
    }
    if file.metadata()?.len() > MAX_OWNERSHIP_JOURNAL_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "pause-session journal exceeds its size limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_OWNERSHIP_JOURNAL_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_OWNERSHIP_JOURNAL_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "pause-session journal exceeds its size limit",
        ));
    }
    Ok(bytes
        .windows(b"\"input_sidecars_required\":true".len())
        .any(|slice| slice == b"\"input_sidecars_required\":true"))
}

/// A normal v1 completion publishes `project.json` before its receipt. This
/// recognizes only the fixed local source leaf; metadata never redirects it.
pub(super) fn has_sealed_normal_project(root: &Path) -> bool {
    let project = root.join("project.json");
    let source = root.join("source.mp4");
    if !is_plain_regular_file(&project).unwrap_or(false)
        || !is_plain_regular_file(&source).unwrap_or(false)
    {
        return false;
    }
    let Ok(file) = std::fs::File::open(project) else {
        return false;
    };
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.len() <= MAX_COMPLETION_PROJECT_BYTES)
    {
        return false;
    }
    let mut bytes = Vec::new();
    if file
        .take(MAX_COMPLETION_PROJECT_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_COMPLETION_PROJECT_BYTES
    {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    value
        .get("source_video")
        .and_then(serde_json::Value::as_str)
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        == Some("source.mp4")
}
