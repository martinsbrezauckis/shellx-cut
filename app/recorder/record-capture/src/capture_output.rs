//! Conversion from backend capture artifacts to the persisted recording project.
//!
//! The optional cadence adapter deliberately consumes `MediaFacts` already
//! returned by final-source verification. It must not inspect the source path or
//! start another FFprobe process.

use record_core::{CaptureCadence, RecordingProject};

use crate::CaptureOutput;

impl CaptureOutput {
    /// Fold the captured artifacts into a `RecordingProject` (ready for autoedit).
    pub fn into_project(self) -> RecordingProject {
        self.into_project_with_optional_cadence(None)
    }

    /// Carry the exact server request plus facts from the same required final
    /// media verification through to the project. Direct/unverified backends
    /// retain no probed_media rather than causing a second FFprobe process.
    pub fn into_project_with_capture_cadence(
        self,
        capture_cadence: CaptureCadence,
    ) -> RecordingProject {
        self.into_project_with_optional_cadence(Some(capture_cadence))
    }

    fn into_project_with_optional_cadence(
        self,
        capture_cadence: Option<CaptureCadence>,
    ) -> RecordingProject {
        let Self {
            source_video,
            events,
            camera_artifact,
            webcam_video,
            audio,
            microphone_outcome: _,
            settings,
            verified_media,
        } = self;
        let mut project = RecordingProject::new(source_video, settings, events);
        project.webcam_video = webcam_video.or_else(|| {
            camera_artifact
                .as_ref()
                .map(|artifact| artifact.video.clone())
        });
        project.camera_artifact = camera_artifact;
        project.audio = audio;
        project.capture_cadence = capture_cadence.map(|cadence| match verified_media {
            Some(media) => cadence.with_probed_media(media.probed_cadence()),
            None => cadence,
        });
        project
    }
}

#[cfg(test)]
mod tests {
    use record_core::{CaptureCadence, FrameRate, Settings};
    use record_recovery::MediaFacts;

    use super::*;

    fn output(verified_media: Option<MediaFacts>) -> CaptureOutput {
        CaptureOutput {
            source_video: "source.mp4".into(),
            events: record_core::fixtures::generate("click-walkthrough").unwrap(),
            camera_artifact: None,
            webcam_video: None,
            audio: None,
            microphone_outcome: crate::MicrophoneCaptureOutcome::NotRequested,
            settings: Settings::default(),
            verified_media,
        }
    }

    #[test]
    fn project_cadence_uses_existing_verified_media_without_a_reprobe() {
        let project = output(Some(MediaFacts {
            duration_ms: 10_010,
            decoded_video_frames: 300,
            has_audio: false,
            avg_frame_rate: Some(FrameRate::new(30_000, 1_001).unwrap()),
            r_frame_rate: Some(FrameRate::new(30, 1).unwrap()),
        }))
        .into_project_with_capture_cadence(CaptureCadence::from_server_fps(29.97).unwrap());
        let cadence = project.capture_cadence.unwrap();
        let probed = cadence.probed_media.unwrap();
        assert_eq!(cadence.requested, FrameRate::new(2_997, 100).unwrap());
        assert_eq!(probed.avg_frame_rate, FrameRate::from_ffprobe("30000/1001"));
        assert_eq!(probed.r_frame_rate, FrameRate::from_ffprobe("30/1"));
        assert_eq!(probed.decoded_video_frames, Some(300));
        assert_eq!(probed.duration_ms, Some(10_010));
    }

    #[test]
    fn unverified_backend_does_not_fabricate_probed_media() {
        let project = output(None)
            .into_project_with_capture_cadence(CaptureCadence::from_server_fps(29.97).unwrap());
        assert!(project.capture_cadence.unwrap().probed_media.is_none());
    }
}
