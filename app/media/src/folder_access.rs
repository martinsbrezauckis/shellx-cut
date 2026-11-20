//! Plain-language errors for a destination folder that is missing or cannot be
//! written.
//!
//! A folder that exists but refuses access must never be reported as "not
//! found": the person then looks for a folder that is right there. These
//! helpers name the folder and say which of the two problems it is, with an
//! action that fixes that problem.

use cut_core::{error_codes, CutError};
use std::io::ErrorKind;
use std::path::Path;

/// The folder as the person typed or picked it: without the Windows
/// extended-length prefix that path resolution adds.
fn shown(folder: &Path) -> String {
    let text = folder.display().to_string();
    text.strip_prefix(r"\\?\")
        .map(str::to_string)
        .unwrap_or(text)
}

/// Map an I/O error met while resolving or opening `folder` to an actionable
/// error. `label` is the person-facing name of the folder ("output folder").
pub fn folder_error(label: &str, folder: &Path, error: &std::io::Error) -> CutError {
    match error.kind() {
        ErrorKind::NotFound => CutError::new(
            error_codes::NOT_FOUND,
            format!("{label} not found: {}", shown(folder)),
            error.to_string(),
        )
        .with_suggested_action("create the folder first, or choose an existing folder"),
        ErrorKind::PermissionDenied => cannot_write(label, folder, error),
        _ => CutError::new(
            error_codes::IO,
            format!("{label} cannot be used: {}", shown(folder)),
            error.to_string(),
        )
        .with_suggested_action(
            "check that the folder is on a connected drive you can write to, or choose another folder",
        ),
    }
}

fn cannot_write(label: &str, folder: &Path, error: &std::io::Error) -> CutError {
    CutError::new(
        error_codes::IO,
        format!(
            "{label} cannot be written: {} (access is denied)",
            shown(folder)
        ),
        error.to_string(),
    )
    .with_suggested_action(
        "choose a folder you can write to, or give your user account write access to this folder",
    )
}

/// Prove `folder` accepts a new file by creating and removing one hidden,
/// uniquely named probe file. Nothing else in the folder is touched.
pub fn ensure_writable_folder(label: &str, folder: &Path) -> Result<(), CutError> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let probe = folder.join(format!(
        ".shellx-cut-write-check-{}-{nonce:x}",
        std::process::id()
    ));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            Err(cannot_write(label, folder, &error))
        }
        Err(error) => Err(folder_error(label, folder, &error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denied_access_is_not_reported_as_missing() {
        let folder = Path::new("/projects/locked");
        let denied = std::io::Error::new(ErrorKind::PermissionDenied, "Access is denied.");
        let error = folder_error("output folder", folder, &denied);
        assert_eq!(error.code, error_codes::IO);
        assert!(error.message.contains("cannot be written"));
        assert!(error.message.contains("locked"));
        assert!(!error.message.contains("not found"));
        assert_eq!(
            error.suggested_action.as_deref(),
            Some("choose a folder you can write to, or give your user account write access to this folder")
        );
        let missing = std::io::Error::new(ErrorKind::NotFound, "missing");
        let error = folder_error("output folder", folder, &missing);
        assert_eq!(error.code, error_codes::NOT_FOUND);
        assert!(error.message.starts_with("output folder not found"));
    }

    #[test]
    fn the_folder_is_shown_without_the_extended_length_prefix() {
        assert_eq!(shown(Path::new(r"\\?\C:\CutFix\ro")), r"C:\CutFix\ro");
        assert_eq!(shown(Path::new("/projects/x")), "/projects/x");
    }

    #[test]
    fn writable_folder_passes_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        ensure_writable_folder("output folder", dir.path()).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_folder_is_refused_as_not_writable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = ensure_writable_folder("output folder", &locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Root ignores directory permissions; the refusal is observable only
        // for an ordinary user.
        if unsafe { libc::geteuid() } != 0 {
            let error = result.unwrap_err();
            assert!(
                error.message.contains("cannot be written"),
                "{}",
                error.message
            );
        }
    }
}
