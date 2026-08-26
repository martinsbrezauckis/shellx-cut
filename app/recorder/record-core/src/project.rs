//! project.rs — `RecordingProject`: the on-disk record that ties everything.
//!
//! A recording = output settings + the raw captured video + the input EventTrack
//! (+ optional webcam/audio) + an optional EditPlan. Non-destructive: the plan is
//! regenerated and the output re-rendered from this state, so re-editing never
//! touches the source. Mirrors the `.cutproj/project.json` idea from ShellX Cut.

use serde::{Deserialize, Serialize};

use crate::camera::CameraArtifact;
use crate::event::EventTrack;
use crate::plan::EditPlan;

/// Output geometry / encoding settings (mirrors Cut's `Settings`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub audio_rate: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_rate: 48000,
        }
    }
}

/// The full recording project (serialized as `project.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingProject {
    /// Schema tag, e.g. "shellx-record/1".
    pub schema: String,
    pub settings: Settings,
    /// Path to the raw captured (or synthetic) screen video.
    pub source_video: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webcam_video: Option<String>,
    /// CameraArtifact@1 is the authoritative synchronized camera stream. The
    /// older `webcam_video` path remains serialized as a compatibility adapter
    /// for old projects and existing webcam-overlay plan consumers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_artifact: Option<CameraArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    pub events: EventTrack,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<EditPlan>,
}

impl RecordingProject {
    /// Construct a project from a captured source + event track (no plan yet).
    pub fn new(source_video: impl Into<String>, settings: Settings, events: EventTrack) -> Self {
        Self {
            schema: crate::SCHEMA.to_string(),
            settings,
            source_video: source_video.into(),
            webcam_video: None,
            camera_artifact: None,
            audio: None,
            events,
            plan: None,
        }
    }

    /// Canonical camera video for presentation consumers. New projects prefer
    /// the validated CameraArtifact@1; legacy projects keep their historical
    /// `webcam_video` behavior without acquiring invented timing or hash facts.
    pub fn camera_video_for_presentation(&self) -> Option<&str> {
        self.camera_artifact
            .as_ref()
            .map(|artifact| artifact.video.as_str())
            .or(self.webcam_video.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    #[test]
    fn project_roundtrips_json() {
        let events = fixtures::generate("click-walkthrough").unwrap();
        let proj = RecordingProject::new("cap.mp4", Settings::default(), events);
        let json = serde_json::to_string_pretty(&proj).unwrap();
        let back: RecordingProject = serde_json::from_str(&json).unwrap();
        assert_eq!(proj, back);
        assert_eq!(back.schema, crate::SCHEMA);
    }

    #[test]
    fn camera_artifact_is_preferred_without_breaking_legacy_webcam_projects() {
        let events = fixtures::generate("click-walkthrough").unwrap();
        let mut project = RecordingProject::new("cap.mp4", Settings::default(), events);
        project.webcam_video = Some("legacy-webcam.mp4".into());
        assert_eq!(
            project.camera_video_for_presentation(),
            Some("legacy-webcam.mp4")
        );
        project.camera_artifact = Some(
            CameraArtifact::new(
                "cap_01",
                "camera_01",
                "camera/camera.mp4",
                crate::CameraClockRange {
                    first_frame_offset_ms: 500,
                    end_frame_offset_ms: 1_500,
                },
                crate::CameraMediaFacts {
                    width: 1280,
                    height: 720,
                    fps_num: 30,
                    fps_den: 1,
                    frame_count: 30,
                    duration_ms: 1_000,
                    sha256: "b".repeat(64),
                },
                crate::CameraTerminalState::Complete,
            )
            .unwrap(),
        );
        assert_eq!(
            project.camera_video_for_presentation(),
            Some("camera/camera.mp4")
        );
    }
}
