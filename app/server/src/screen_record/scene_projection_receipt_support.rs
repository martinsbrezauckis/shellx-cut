//! Small receipt wire and byte-integrity helpers shared by scene projection.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use cut_core::{error_codes, CutError};
use record_capture::RecordingSceneProjection;
use record_core::scene_projection::EditableSceneTimeline;
use record_core::EditPlan;
use record_recovery::CaptureRoot;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::capture_artifacts::ValidatedCameraArtifact;

pub(crate) const RECORDING_SCENE_RECEIPT_FILE: &str = "recording-scene.receipt.json";
pub(super) const RECORDING_SCENE_RECEIPT_SCHEMA: &str = "shellx-cut/recording-scene-receipt@1";
const MAX_RECEIPT_BYTES: u64 = 512 * 1024;
pub(super) const RETRY_CAPTURE_ACTION: &str =
    "discard the modified capture or retry it before auto-editing Recording Scenes";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecordingSceneReceipt {
    pub(super) schema: String,
    pub(super) capture_id: String,
    pub(super) screen_output: String,
    pub(super) editable_project: String,
    pub(super) source_sha256: String,
    pub(super) project_sha256: String,
    pub(super) projection: RecordingSceneProjection,
}

pub(crate) struct VerifiedSceneReceipt {
    pub(crate) path: PathBuf,
    pub(crate) capture_dir: PathBuf,
    pub(crate) timeline: EditableSceneTimeline,
}

/// Locate and fully validate the optional receipt surfaced by `screen_record.stop`.
pub(crate) fn completed_receipt_for_stop(
    project_dir: &Path,
    capture_id: &str,
    out_dir: &Path,
) -> Result<Option<VerifiedSceneReceipt>, CutError> {
    let root = CaptureRoot::open_existing(project_dir)
        .map_err(cut_root_error)?
        .ok_or_else(|| receipt_error("screen-record capture root is unavailable"))?;
    let capture_dir = root
        .existing_capture_dir(capture_id)
        .map_err(cut_root_error)?
        .ok_or_else(|| receipt_error("capture directory is unavailable"))?;
    let capture_dir = capture_dir.canonicalize().map_err(cut_io_error)?;
    let out_dir = out_dir.canonicalize().map_err(cut_io_error)?;
    if capture_dir != out_dir {
        return Err(receipt_error(
            "public scene receipt capture directory does not match screen_record.stop",
        ));
    }
    let Some(path) = super::optional_plain_file_in_dir(
        &capture_dir,
        RECORDING_SCENE_RECEIPT_FILE,
        "Recording Scenes receipt",
        RETRY_CAPTURE_ACTION,
    )?
    else {
        return Ok(None);
    };
    verify_receipt(&capture_dir, &path, capture_id).map(Some)
}

/// Resolve the explicit `scene_receipt` autoedit argument. It must be the
/// fixed capture leaf beside the EventTrack, never an arbitrary receipt path.
pub(super) fn receipt_for_autoedit(
    project_dir: &Path,
    track_path: &Path,
    requested: &str,
) -> Result<VerifiedSceneReceipt, CutError> {
    let path = super::plain_existing_file_under_project(
        project_dir,
        requested,
        "Recording Scenes receipt",
        "pass the scene_receipt path returned by screen_record.stop",
    )?;
    if path.file_name().and_then(|name| name.to_str()) != Some(RECORDING_SCENE_RECEIPT_FILE) {
        return Err(receipt_error(
            "scene_receipt must name recording-scene.receipt.json",
        ));
    }
    let track_parent = track_path
        .parent()
        .ok_or_else(|| receipt_error("EventTrack has no capture directory"))?
        .canonicalize()
        .map_err(cut_io_error)?;
    let capture_dir = path
        .parent()
        .filter(|parent| *parent == track_parent.as_path())
        .ok_or_else(|| {
            receipt_error("scene_receipt and EventTrack must belong to the same capture")
        })?;
    let capture_id = capture_id_from_dir(capture_dir)?;
    let root = CaptureRoot::open_existing(project_dir)
        .map_err(cut_root_error)?
        .ok_or_else(|| receipt_error("screen-record capture root is unavailable"))?;
    let expected_capture_dir = root
        .existing_capture_dir(&capture_id)
        .map_err(cut_root_error)?
        .ok_or_else(|| receipt_error("capture directory is unavailable"))?
        .canonicalize()
        .map_err(cut_io_error)?;
    let expected_receipt = super::plain_existing_file_under_dir(
        &expected_capture_dir,
        &expected_capture_dir.join(RECORDING_SCENE_RECEIPT_FILE),
        "Recording Scenes receipt",
        RETRY_CAPTURE_ACTION,
    )?;
    if expected_receipt != path {
        return Err(receipt_error(
            "scene_receipt is not under the capture directory named by its receipt",
        ));
    }
    verify_receipt(&expected_capture_dir, &path, &capture_id)
}

/// Lower a verified replay into the ordinary editable plan primitive. Presenter
/// spans accept only the finalized `CameraArtifact`, not a caller-supplied path.
pub(super) fn apply_to_edit_plan(
    plan: &mut EditPlan,
    receipt: &VerifiedSceneReceipt,
    camera: Option<&ValidatedCameraArtifact>,
) -> Result<(), CutError> {
    let has_presenter = receipt
        .timeline
        .camera
        .iter()
        .any(|segment| segment.presenter.is_some());
    if has_presenter && camera.is_none() {
        return Err(receipt_error(
            "Recording Scenes presenter spans require a verified finalized camera artifact",
        ));
    }
    let source = camera.map(|camera| camera.video_path.display().to_string());
    let clock = camera.map(|camera| camera.artifact.clock.clipped_to_plan(plan.duration_ms));
    receipt
        .timeline
        .apply_to_edit_plan(plan, source, clock)
        .map_err(|error| receipt_error(format!("apply Recording Scenes timeline: {error}")))?;
    plan.validate()
        .map_err(|error| receipt_error(format!("validate Recording Scenes EditPlan: {error}")))
}

fn verify_receipt(
    capture_dir: &Path,
    path: &Path,
    expected_capture_id: &str,
) -> Result<VerifiedSceneReceipt, CutError> {
    // Canonicalizing a directory and canonicalizing a child can spell the same
    // Windows path differently (notably a short-name temp root). Resolve the
    // fixed leaf itself before comparing identities; this still fails closed
    // for a missing, linked, or substituted receipt.
    let fixed_path = super::plain_existing_file_under_dir(
        capture_dir,
        &capture_dir.join(RECORDING_SCENE_RECEIPT_FILE),
        "Recording Scenes receipt",
        RETRY_CAPTURE_ACTION,
    )?;
    if path != fixed_path {
        return Err(receipt_error(
            "Recording Scenes receipt is not the fixed capture-owned leaf",
        ));
    }
    let bytes = read_bounded(path)?;
    let receipt: RecordingSceneReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| receipt_error(format!("parse Recording Scenes receipt: {error}")))?;
    if receipt.schema != RECORDING_SCENE_RECEIPT_SCHEMA
        || receipt.capture_id != expected_capture_id
        || receipt.screen_output != "source.mp4"
        || receipt.editable_project != "project.json"
        || !is_sha256(&receipt.source_sha256)
        || !is_sha256(&receipt.project_sha256)
    {
        return Err(receipt_error(
            "Recording Scenes receipt schema, capture identity, paths, or SHA-256 commitments are invalid",
        ));
    }
    receipt
        .projection
        .validate()
        .map_err(|error| receipt_error(format!("validate Recording Scenes projection: {error}")))?;
    let source = super::plain_existing_file_under_dir(
        capture_dir,
        &capture_dir.join("source.mp4"),
        "Recording Scenes screen output",
        RETRY_CAPTURE_ACTION,
    )?;
    let project = super::plain_existing_file_under_dir(
        capture_dir,
        &capture_dir.join("project.json"),
        "Recording Scenes project metadata",
        RETRY_CAPTURE_ACTION,
    )?;
    if sha256_file(&source).map_err(cut_io_error)? != receipt.source_sha256
        || sha256_file(&project).map_err(cut_io_error)? != receipt.project_sha256
    {
        return Err(receipt_error(
            "Recording Scenes receipt does not match the finalized source or project bytes",
        ));
    }
    let timeline = EditableSceneTimeline::from_replay(
        receipt.projection.snapshot(),
        receipt.projection.events(),
        receipt.projection.logical_media_time_ms(),
        receipt.projection.journal_sha256(),
    )
    .map_err(|error| receipt_error(format!("replay Recording Scenes receipt: {error}")))?;
    if timeline.terminal != *receipt.projection.scene() {
        return Err(receipt_error(
            "Recording Scenes receipt terminal projection differs from its replay timeline",
        ));
    }
    Ok(VerifiedSceneReceipt {
        path: path.to_path_buf(),
        capture_dir: capture_dir.to_path_buf(),
        timeline,
    })
}

pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>, CutError> {
    let file = File::open(path).map_err(cut_io_error)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(cut_io_error)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(receipt_error(
            "Recording Scenes receipt exceeds its 512 KiB limit",
        ));
    }
    Ok(bytes)
}

pub(super) fn matches_owned_source(source_video: &str, expected_source: &Path) -> bool {
    Path::new(source_video) == expected_source || Path::new(source_video) == Path::new("source.mp4")
}

pub(super) fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(format!("{:x}", digest.finalize()));
        }
        digest.update(&buffer[..read]);
    }
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn receipt_error(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "Recording Scenes receipt is invalid",
        detail.into(),
    )
    .with_suggested_action(RETRY_CAPTURE_ACTION)
}

pub(super) fn cut_io_error(error: std::io::Error) -> CutError {
    receipt_error(format!("read Recording Scenes receipt output: {error}"))
}

fn cut_root_error(error: record_recovery::ManifestError) -> CutError {
    receipt_error(format!("resolve Recording Scenes receipt owner: {error}"))
}

fn capture_id_from_dir(capture_dir: &Path) -> Result<String, CutError> {
    capture_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| receipt_error("scene_receipt capture directory has no valid identity"))
}
