//! cutd mapping for the recorder's project-local capture-root anchor.

use std::path::{Path, PathBuf};

use cut_core::{error_codes, CutError};
use record_recovery::{CaptureRoot, ManifestError};

pub(crate) fn cache_dir(project_dir: &Path) -> Result<PathBuf, CutError> {
    Ok(root(project_dir)?.cache_dir().to_path_buf())
}

pub(crate) fn existing_cache_dir(project_dir: &Path) -> Result<Option<PathBuf>, CutError> {
    CaptureRoot::open_existing(project_dir)
        .map_err(|error| containment_error("inspect the screen-record capture root", error))
        .map(|root| root.map(|root| root.cache_dir().to_path_buf()))
}

pub(crate) fn create_capture_dir(
    project_dir: &Path,
    capture_id: &str,
) -> Result<PathBuf, CutError> {
    root(project_dir)?
        .create_capture_dir(capture_id)
        .map_err(|error| containment_error("create the screen-record capture directory", error))
}

pub(crate) fn existing_capture_dir(
    project_dir: &Path,
    capture_id: &str,
) -> Result<Option<PathBuf>, CutError> {
    root(project_dir)?
        .existing_capture_dir(capture_id)
        .map_err(|error| containment_error("resolve the screen-record capture directory", error))
}

pub(crate) fn capture_file(
    project_dir: &Path,
    capture_id: &str,
    file_name: &str,
) -> Result<PathBuf, CutError> {
    root(project_dir)?
        .capture_file(capture_id, file_name)
        .map_err(|error| containment_error("resolve a screen-record capture file", error))
}

pub(crate) fn publish_marker(
    project_dir: &Path,
    capture_id: &str,
    bytes: &[u8],
) -> Result<PathBuf, CutError> {
    root(project_dir)?
        .publish_new_capture_file(capture_id, ".capture.json", bytes)
        .map_err(|error| containment_error("publish the capture marker", error))
}

fn root(project_dir: &Path) -> Result<CaptureRoot, CutError> {
    CaptureRoot::for_project(project_dir)
        .map_err(|error| containment_error("prepare the screen-record capture root", error))
}

fn containment_error(stage: &str, error: ManifestError) -> CutError {
    // A project folder that refuses new files is a permission problem, not an
    // unsafe link: name the folder and say it cannot be written.
    if let ManifestError::Io { path, source } = &error {
        if source.kind() == std::io::ErrorKind::PermissionDenied {
            let folder = path.parent().unwrap_or(path.as_path());
            return cut_media::folder_access::folder_error("recording folder", folder, source);
        }
    }
    CutError::new(
        error_codes::IO,
        format!("could not {stage}: {error}"),
        "screen-record files must stay in local plain directories under the open project",
    )
    .with_suggested_action("remove the unsafe cache link or reparse point, then retry")
}

#[cfg(all(test, unix))]
mod permission_tests {
    use super::*;

    #[test]
    fn project_that_refuses_new_files_is_named_and_not_called_an_unsafe_link() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("locked.cutproj");
        std::fs::create_dir(&project).unwrap();
        std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = create_capture_dir(&project, "cap_1_2_3");
        std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = result.unwrap_err();
        assert!(
            error.message.contains("cannot be written"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("locked.cutproj"),
            "{}",
            error.message
        );
        assert_ne!(
            error.suggested_action.as_deref(),
            Some("remove the unsafe cache link or reparse point, then retry")
        );
    }
}
