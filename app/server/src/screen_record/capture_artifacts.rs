//! Validated capture artifacts consumed by `screen_record.stop`.

use std::path::{Path, PathBuf};

use cut_core::CutError;

use super::{optional_plain_file_in_dir, plain_existing_file_under_dir};

const RETRY_CAPTURE_ACTION: &str =
    "discard the incomplete capture or retry recording before requesting screen_record.stop";

pub(crate) struct StopArtifacts {
    pub(crate) source_video: PathBuf,
    pub(crate) webcam: Option<String>,
    pub(crate) studio_events: Option<String>,
    pub(crate) mic: Option<PathBuf>,
    pub(crate) system: Option<PathBuf>,
    pub(crate) system_timing: Option<String>,
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
    webcam_video_raw: Option<&str>,
    audio_raw: Option<&str>,
) -> Result<StopArtifacts, CutError> {
    let source_video = plain_existing_file_under_dir(
        capture_dir,
        &capture_candidate(capture_dir, source_video_raw),
        "capture source_video",
        RETRY_CAPTURE_ACTION,
    )?;
    let webcam = webcam_video_raw
        .map(|raw| {
            plain_existing_file_under_dir(
                capture_dir,
                &capture_candidate(capture_dir, raw),
                "capture webcam_video",
                "discard the incomplete camera stream or retry recording before requesting screen_record.stop",
            )
            .map(|path| path.display().to_string())
        })
        .transpose()?;
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
        webcam,
        studio_events,
        mic,
        system,
        system_timing: system_timing_path.map(|path| path.display().to_string()),
    })
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
