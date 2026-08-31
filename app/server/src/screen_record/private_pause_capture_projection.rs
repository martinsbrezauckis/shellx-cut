//! Platform-neutral final projection for the private native pause owners.

use super::pause_projection::SealedLegacyProjectionRun;
use super::pause_projection_executor::{
    execute_frame_grid_pause_projection, FfmpegFrameGridPauseProjectionSourceStager,
    FfmpegPauseProjectionArtifactVerifier,
};
use super::pause_session_owner::{PauseSessionProjectionExecutor, PauseSessionWorkerError};
use record_capture::{CaptureOutput, MicrophoneCaptureOutcome, SelectedCaptureStreams};
use record_core::{error_codes, RecordError, RecordingProject};
use record_recovery::{CaptureRoot, RecordingSessionJournal, RecordingStream};

pub(super) struct PrivateFrameGridProjection {
    root: CaptureRoot,
    capture_id: String,
    streams: SelectedCaptureStreams,
    require_completed_audio_receipt: bool,
}

impl PrivateFrameGridProjection {
    pub(super) fn new(
        root: CaptureRoot,
        capture_id: String,
        streams: SelectedCaptureStreams,
    ) -> Self {
        Self {
            root,
            capture_id,
            streams,
            require_completed_audio_receipt: true,
        }
    }

    #[cfg(all(test, windows))]
    pub(super) fn test_without_completed_audio_receipt(
        root: CaptureRoot,
        capture_id: String,
        streams: SelectedCaptureStreams,
    ) -> Self {
        Self {
            root,
            capture_id,
            streams,
            require_completed_audio_receipt: false,
        }
    }

    pub(super) fn capture_output(&self) -> record_core::Result<CaptureOutput> {
        if self.require_completed_audio_receipt {
            super::pause_projection_executor::verify_completed_audio_projection(
                &self.root,
                &self.capture_id,
            )
            .map_err(|_| capture_error("verify private completed audio projection"))?;
        }
        let capture_dir = self
            .root
            .existing_capture_dir(&self.capture_id)
            .map_err(|_| capture_error("resolve private projected capture directory"))?
            .ok_or_else(|| capture_error("resolve private projected capture directory"))?;
        let project_path = self
            .root
            .capture_file(&self.capture_id, "project.json")
            .map_err(|_| capture_error("resolve private projected project"))?;
        if !record_recovery::is_plain_regular_file(&project_path).unwrap_or(false) {
            return Err(capture_error("verify private projected project"));
        }
        let bytes = std::fs::read(&project_path)
            .map_err(|_| capture_error("read private projected project"))?;
        let project: RecordingProject = serde_json::from_slice(&bytes)
            .map_err(|_| capture_error("decode private projected project"))?;
        if project.source_video != "source.mp4"
            || project.webcam_video.is_some()
            || project.camera_artifact.is_some()
            || project.plan.is_some()
        {
            return Err(capture_error("verify private projected project"));
        }
        let microphone_selected = self
            .streams
            .streams()
            .contains(&RecordingStream::MicrophoneAudio);
        let system_audio_selected = self
            .streams
            .streams()
            .contains(&RecordingStream::SystemAudio);
        let audio = match (microphone_selected, project.audio.as_deref()) {
            (false, None) => None,
            (true, Some("mic.wav")) => {
                let path = self
                    .root
                    .capture_file(&self.capture_id, "mic.wav")
                    .map_err(|_| capture_error("resolve private projected microphone audio"))?;
                if !record_recovery::is_plain_regular_file(&path).unwrap_or(false) {
                    return Err(capture_error("verify private projected microphone audio"));
                }
                Some("mic.wav".to_string())
            }
            _ => return Err(capture_error("verify private projected project audio")),
        };
        if system_audio_selected {
            let path = capture_dir.join("system.wav");
            let timing = super::system_audio::read_timing(&capture_dir)
                .map_err(|_| capture_error("verify private projected system-audio timing"))?;
            if !record_recovery::is_plain_regular_file(&path).unwrap_or(false) || timing.is_none() {
                return Err(capture_error("verify private projected system audio"));
            }
        };
        Ok(CaptureOutput {
            source_video: project.source_video,
            events: project.events,
            camera_artifact: None,
            webcam_video: None,
            audio: audio.clone(),
            microphone_outcome: if audio.is_some() {
                MicrophoneCaptureOutcome::Saved
            } else {
                MicrophoneCaptureOutcome::NotRequested
            },
            settings: project.settings,
            capture_quality: None,
            verified_media: None,
        })
    }
}

impl PauseSessionProjectionExecutor for PrivateFrameGridProjection {
    fn execute(
        &mut self,
        journal: &RecordingSessionJournal,
        event_runs: &[SealedLegacyProjectionRun],
    ) -> Result<(), PauseSessionWorkerError> {
        let ffmpeg = cut_media::toolpath::ffmpeg();
        let ffprobe = cut_media::toolpath::ffprobe();
        let verifier = FfmpegPauseProjectionArtifactVerifier::new(
            ffmpeg.to_string_lossy(),
            ffprobe.to_string_lossy(),
        );
        let mut stager = FfmpegFrameGridPauseProjectionSourceStager::new(ffmpeg.to_string_lossy());
        execute_frame_grid_pause_projection(
            &self.root,
            &self.capture_id,
            journal,
            event_runs,
            &verifier,
            &mut stager,
        )
        .map(|_| ())
        .map_err(|_| PauseSessionWorkerError::new("private native pause projection failed"))
    }
}

fn capture_error(message: &str) -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        message,
        "the private pause owner retained its incomplete capture evidence",
    )
}
