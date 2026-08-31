//! Durable public Recording Scenes receipt and replay hand-off.
//!
//! The receipt is published before `project.json`: stop therefore never sees a
//! finalized project whose admitted public scene replay has not been sealed.

use std::path::{Path, PathBuf};

use record_capture::RecordingSceneProjection;
use record_core::{RecordError, RecordingProject};
use record_recovery::{is_plain_regular_file, CaptureRoot};

use super::capture_session_control::CaptureSessionControl;
pub(crate) use super::scene_projection_receipt_support::completed_receipt_for_stop;
pub(crate) use super::scene_projection_receipt_support::RECORDING_SCENE_RECEIPT_FILE;
pub(super) use super::scene_projection_receipt_support::{
    apply_to_edit_plan, receipt_for_autoedit,
};
use super::scene_projection_receipt_support::{
    matches_owned_source, sha256_bytes, sha256_file, RecordingSceneReceipt,
    RECORDING_SCENE_RECEIPT_SCHEMA,
};

/// Seal a validated terminal engine projection before the final project becomes
/// visible. A matching pre-existing receipt is a recovery retry; it is never
/// replaced. A project that is already visible without that receipt fails
/// closed, preserving the ordering guarantee for normal capture.
pub(super) fn publish_completed_projection(
    control: &CaptureSessionControl,
    project_dir: &Path,
    capture_id: &str,
    out_dir: &Path,
    project_path: &Path,
    project: &RecordingProject,
    project_bytes: &[u8],
) -> record_core::Result<Option<PathBuf>> {
    let Some(projection) = control
        .completed_recording_scene_projection()
        .map_err(control_error)?
    else {
        return Ok(None);
    };
    publish_projection(
        &projection,
        project_dir,
        capture_id,
        out_dir,
        project_path,
        project,
        project_bytes,
    )
}

/// Publish a projection already returned by the capture owner. Kept separate
/// so receipt durability can be tested without a native capture clock.
pub(super) fn publish_projection(
    projection: &RecordingSceneProjection,
    project_dir: &Path,
    capture_id: &str,
    out_dir: &Path,
    project_path: &Path,
    project: &RecordingProject,
    project_bytes: &[u8],
) -> record_core::Result<Option<PathBuf>> {
    projection.validate().map_err(scene_error)?;

    let root = CaptureRoot::for_project(project_dir).map_err(root_error)?;
    let capture_dir = root
        .existing_capture_dir(capture_id)
        .map_err(root_error)?
        .ok_or_else(|| invalid("public scene receipt has no capture directory"))?;
    if capture_dir != out_dir {
        return Err(invalid(
            "public scene receipt output directory does not match its capture owner",
        ));
    }
    let expected_project = root
        .capture_file(capture_id, "project.json")
        .map_err(root_error)?;
    if expected_project != project_path {
        return Err(invalid(
            "public scene receipt project is not the capture-owned local project.json",
        ));
    }
    let source = root
        .capture_file(capture_id, "source.mp4")
        .map_err(root_error)?;
    if !matches_owned_source(&project.source_video, &source)
        || !is_plain_regular_file(&source).map_err(root_error)?
    {
        return Err(invalid(
            "public scene receipt screen output is not the capture-owned local source.mp4",
        ));
    }

    let receipt = RecordingSceneReceipt {
        schema: RECORDING_SCENE_RECEIPT_SCHEMA.into(),
        capture_id: capture_id.into(),
        screen_output: "source.mp4".into(),
        editable_project: "project.json".into(),
        source_sha256: sha256_file(&source).map_err(io_error)?,
        project_sha256: sha256_bytes(project_bytes),
        projection: projection.clone(),
    };
    let bytes = serde_json::to_vec(&receipt)
        .map_err(|error| invalid(format!("serialize public scene receipt: {error}")))?;
    let receipt_path = root
        .capture_file(capture_id, RECORDING_SCENE_RECEIPT_FILE)
        .map_err(root_error)?;
    match std::fs::symlink_metadata(&receipt_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if project_is_visible(project_path)? {
                return Err(invalid(
                    "refusing to publish public scene receipt after project.json became visible",
                ));
            }
            root.publish_new_capture_file(capture_id, RECORDING_SCENE_RECEIPT_FILE, &bytes)
                .map(Some)
                .map_err(root_error)
        }
        Err(error) => Err(io_error(error)),
        Ok(_) if !is_plain_regular_file(&receipt_path).map_err(root_error)? => Err(invalid(
            "public scene receipt is linked or not a local regular file",
        )),
        Ok(_) => {
            let existing = std::fs::read(&receipt_path).map_err(io_error)?;
            if existing == bytes {
                Ok(Some(receipt_path))
            } else {
                Err(invalid(
                    "existing public scene receipt differs from the capture outputs",
                ))
            }
        }
    }
}

fn project_is_visible(path: &Path) -> record_core::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(error)),
        Ok(_) if !is_plain_regular_file(path).map_err(root_error)? => Err(invalid(
            "capture-owned project.json is linked or not a local regular file",
        )),
        Ok(_) => Ok(true),
    }
}

fn root_error(error: record_recovery::ManifestError) -> RecordError {
    invalid(format!("resolve public scene receipt owner: {error}"))
}

fn control_error(error: record_capture::PrivateSceneCoordinatorError) -> RecordError {
    invalid(format!("seal public Recording Scenes projection: {error}"))
}

fn scene_error(error: record_capture::RecordingSceneEngineError) -> RecordError {
    invalid(format!(
        "validate public Recording Scenes projection: {error}"
    ))
}

fn io_error(error: std::io::Error) -> RecordError {
    invalid(format!(
        "read public Recording Scenes receipt output: {error}"
    ))
}

fn invalid(detail: impl Into<String>) -> RecordError {
    RecordError::new(
        record_core::error_codes::IO,
        "public Recording Scenes receipt could not be published",
        detail.into(),
    )
}
