//! Validated capture artifacts consumed by `screen_record.stop`.

use std::io::Read;
use std::path::{Path, PathBuf};

use cut_core::CutError;
use record_core::CameraArtifact;
use sha2::{Digest, Sha256};

use super::{optional_plain_file_in_dir, plain_existing_file_under_dir};

const RETRY_CAPTURE_ACTION: &str =
    "discard the incomplete capture or retry recording before requesting screen_record.stop";
const MAX_CAPTURE_PROJECT_JSON_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) struct StopArtifacts {
    pub(crate) source_video: PathBuf,
    /// New authoritative camera contract, retained with its relative artifact
    /// leaf while `video_path` is the already-contained resolved file.
    pub(crate) camera: Option<ValidatedCameraArtifact>,
    /// Legacy compatibility path consumed by existing webcam-overlay autoedit.
    pub(crate) webcam: Option<String>,
    pub(crate) studio_events: Option<String>,
    pub(crate) mic: Option<PathBuf>,
    pub(crate) system: Option<PathBuf>,
    pub(crate) system_timing: Option<String>,
}

pub(crate) struct ValidatedCameraArtifact {
    pub(crate) artifact: CameraArtifact,
    pub(crate) video_path: PathBuf,
}

pub(crate) struct RawMuxInputs {
    pub(crate) source_video: PathBuf,
    pub(crate) mic: Option<PathBuf>,
    pub(crate) system: Option<PathBuf>,
    pub(crate) system_offset_ms: Option<u64>,
}

/// Resolve every optional capture leaf before it is surfaced or passed to ffmpeg.
/// Missing optional leaves stay absent; a present link, reparse point, or other
/// non-regular leaf fails closed.
pub(crate) fn resolve_stop_artifacts(
    capture_dir: &Path,
    source_video_raw: &str,
    camera_artifact: Option<CameraArtifact>,
    webcam_video_raw: Option<&str>,
    audio_raw: Option<&str>,
) -> Result<StopArtifacts, CutError> {
    let source_video = plain_existing_file_under_dir(
        capture_dir,
        &capture_candidate(capture_dir, source_video_raw),
        "capture source_video",
        RETRY_CAPTURE_ACTION,
    )?;
    let camera = camera_artifact
        .map(|artifact| resolve_camera_artifact(capture_dir, artifact))
        .transpose()?;
    let webcam = if let Some(camera) = camera.as_ref() {
        Some(camera.video_path.display().to_string())
    } else {
        webcam_video_raw
            .map(|raw| {
                plain_existing_file_under_dir(
                    capture_dir,
                    &capture_candidate(capture_dir, raw),
                    "capture webcam_video",
                    "discard the incomplete camera stream or retry recording before requesting screen_record.stop",
                )
                .map(|path| path.display().to_string())
            })
            .transpose()?
    };
    let studio_events_path = optional_plain_file_in_dir(
        capture_dir,
        crate::screen_record_studio::STUDIO_EVENTS_FILENAME,
        "Studio event metadata",
        RETRY_CAPTURE_ACTION,
    )?;
    let studio_events = studio_events_path
        .as_deref()
        .map(|path| {
            crate::screen_record_studio::read_studio_events(path)?;
            Ok::<_, CutError>(path.display().to_string())
        })
        .transpose()?;
    // A declared audio path is authoritative. Do not silently fall back to a
    // sibling leaf when it points outside the capture or is unsafe.
    let mic = match audio_raw {
        Some(raw) => Some(plain_existing_file_under_dir(
            capture_dir,
            &capture_candidate(capture_dir, raw),
            "capture audio",
            RETRY_CAPTURE_ACTION,
        )?),
        None => optional_plain_file_in_dir(
            capture_dir,
            "mic.wav",
            "capture microphone audio",
            RETRY_CAPTURE_ACTION,
        )?,
    };
    let system = optional_plain_file_in_dir(
        capture_dir,
        "system.wav",
        "capture system audio",
        RETRY_CAPTURE_ACTION,
    )?;
    let system_timing_path = optional_plain_file_in_dir(
        capture_dir,
        crate::screen_record::system_audio::SYSTEM_AUDIO_TIMING_FILE,
        "capture system-audio timing",
        RETRY_CAPTURE_ACTION,
    )?;

    Ok(StopArtifacts {
        source_video,
        camera,
        webcam,
        studio_events,
        mic,
        system,
        system_timing: system_timing_path.map(|path| path.display().to_string()),
    })
}

/// Validate the serialized CameraArtifact@1 and its immutable file commitment.
/// The core contract rejects traversal structurally; this server boundary then
/// proves that the referenced object is a contained plain file with the exact
/// finalized SHA-256 declared by the capture worker.
pub(crate) fn resolve_camera_artifact(
    capture_dir: &Path,
    artifact: CameraArtifact,
) -> Result<ValidatedCameraArtifact, CutError> {
    artifact.validate().map_err(|error| {
        CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            "capture camera_artifact is invalid",
            error.to_string(),
        )
        .with_suggested_action(
            "discard the incomplete camera artifact or retry recording before requesting screen_record.stop",
        )
    })?;
    let expected_capture_id = capture_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            CutError::new(
                cut_core::error_codes::INVALID_ARGS,
                "capture directory has no valid capture identity",
                "camera artifact validation requires a named capture directory",
            )
        })?;
    if artifact.capture_id != expected_capture_id {
        return Err(CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            "capture camera artifact identity does not match its capture directory",
            format!(
                "camera artifact capture_id '{}' does not match capture '{}'",
                artifact.capture_id, expected_capture_id
            ),
        )
        .with_suggested_action(
            "discard the mismatched camera artifact or retry recording before requesting screen_record.stop",
        ));
    }
    let video_path = plain_existing_file_under_dir(
        capture_dir,
        &capture_dir.join(&artifact.video),
        "capture camera artifact",
        "discard the incomplete camera artifact or retry recording before requesting screen_record.stop",
    )?;
    let actual = sha256_file(&video_path).map_err(|error| {
        CutError::new(
            cut_core::error_codes::IO,
            format!("could not hash capture camera artifact at {}", video_path.display()),
            error.to_string(),
        )
        .with_suggested_action(
            "discard the incomplete camera artifact or retry recording before requesting screen_record.stop",
        )
    })?;
    if actual != artifact.media.sha256 {
        return Err(CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            "capture camera artifact integrity check failed",
            format!(
                "camera artifact SHA-256 differs from finalized metadata for {}",
                artifact.video
            ),
        )
        .with_suggested_action(
            "discard the modified camera artifact or retry recording before requesting screen_record.stop",
        ));
    }
    Ok(ValidatedCameraArtifact {
        artifact,
        video_path,
    })
}

/// Read and validate the camera artifact stored beside a finalized screen source.
/// `screen_record.polish` uses this to import a separate editable camera clip
/// without adding a new public UI or live-capture argument surface.
pub(crate) fn camera_artifact_for_capture(
    capture_dir: &Path,
) -> Result<Option<ValidatedCameraArtifact>, CutError> {
    let Some(project_path) = optional_plain_file_in_dir(
        capture_dir,
        "project.json",
        "capture project metadata",
        RETRY_CAPTURE_ACTION,
    )?
    else {
        return Ok(None);
    };
    let file = std::fs::File::open(&project_path).map_err(|error| {
        CutError::new(
            cut_core::error_codes::IO,
            format!(
                "could not read capture project metadata at {}",
                project_path.display()
            ),
            error.to_string(),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_CAPTURE_PROJECT_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CutError::new(
                cut_core::error_codes::IO,
                format!(
                    "could not read capture project metadata at {}",
                    project_path.display()
                ),
                error.to_string(),
            )
        })?;
    if bytes.len() as u64 > MAX_CAPTURE_PROJECT_JSON_BYTES {
        return Err(CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            "capture project metadata is too large",
            "camera artifact metadata must fit within the bounded RecordingProject contract",
        ));
    }
    let project: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            "capture project metadata is not valid JSON",
            error.to_string(),
        )
    })?;
    let Some(raw) = project.get("camera_artifact").cloned() else {
        return Ok(None);
    };
    let artifact: CameraArtifact = serde_json::from_value(raw).map_err(|error| {
        CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            "capture project metadata camera_artifact is invalid",
            error.to_string(),
        )
    })?;
    resolve_camera_artifact(capture_dir, artifact).map(Some)
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

impl StopArtifacts {
    /// Reuse the already-validated local inputs for raw muxing.
    pub(crate) fn raw_mux_inputs(&self, capture_dir: &Path) -> Result<RawMuxInputs, CutError> {
        let timing = self
            .system
            .as_ref()
            .map(|_| crate::screen_record::system_audio::read_timing(capture_dir))
            .transpose()?
            .flatten();
        let mut system = self.system.clone();
        if timing
            .as_ref()
            .is_some_and(|timing| timing.first_packet_offset_ms.is_none())
        {
            system = None;
        }
        Ok(RawMuxInputs {
            source_video: self.source_video.clone(),
            mic: self.mic.clone(),
            system,
            system_offset_ms: timing.and_then(|timing| timing.first_packet_offset_ms),
        })
    }
}

fn capture_candidate(capture_dir: &Path, raw: &str) -> PathBuf {
    let raw = Path::new(raw);
    if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        capture_dir.join(raw)
    }
}
