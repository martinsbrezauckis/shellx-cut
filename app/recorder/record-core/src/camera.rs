//! Camera capture artifact contract.
//!
//! This module intentionally contains no device or platform code.  It records
//! the immutable result of a camera capture once a native backend has delivered
//! it, using the screen recorder's shared `CaptureClock` as its only timebase.
//! A camera file is never identified by an absolute host path: `video` is a
//! contained, project-local relative path and the server validates it before it
//! is exposed or imported.

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::{error_codes, RecordError, Result};

/// Schema tag for a camera capture artifact embedded in a recording project.
pub const CAMERA_ARTIFACT_SCHEMA: &str = "shellx-record/camera-artifact/1";

/// Immutable camera stream facts produced by a completed (or terminal) capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraArtifact {
    /// Versioned schema for this artifact, independent from `RecordingProject`.
    pub schema: String,
    /// Identity of the screen-capture session whose `CaptureClock` owns offsets.
    pub capture_id: String,
    /// Stable identity of this finalized camera artifact within that capture.
    pub artifact_id: String,
    /// Relative path from the capture directory to the camera video.  Absolute,
    /// parent-traversing, and platform-path-like values are invalid.
    pub video: String,
    /// First and last delivered camera-frame times measured on the shared
    /// screen-capture `CaptureClock`.
    pub clock: CameraClockRange,
    /// Measured media facts and a full-content SHA-256 integrity commitment.
    pub media: CameraMediaFacts,
    /// Terminal disposition. A non-complete artifact remains editable only for
    /// the measured interval; consumers must not fabricate missing frames.
    pub terminal_state: CameraTerminalState,
}

/// Camera frame interval measured from the screen capture's `CaptureClock`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraClockRange {
    pub first_frame_offset_ms: u64,
    pub end_frame_offset_ms: u64,
}

/// Facts measured from the finalized camera media, not caller-provided intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraMediaFacts {
    pub width: u32,
    pub height: u32,
    /// Rational frame rate so the artifact never depends on a lossy float.
    pub fps_num: u32,
    pub fps_den: u32,
    pub frame_count: u64,
    /// Duration of the available camera stream. It must equal its measured
    /// `CaptureClock` interval for this first contract revision.
    pub duration_ms: u64,
    /// Lowercase hexadecimal SHA-256 of the complete video file (64 chars).
    pub sha256: String,
}

/// How the camera stream ended. This is intentionally not a scene/event state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraTerminalState {
    Complete,
    Cancelled,
    DeviceLost,
    Recovered,
}

impl CameraArtifact {
    /// Build a validated camera artifact from finalized capture facts.
    pub fn new(
        capture_id: impl Into<String>,
        artifact_id: impl Into<String>,
        video: impl Into<String>,
        clock: CameraClockRange,
        media: CameraMediaFacts,
        terminal_state: CameraTerminalState,
    ) -> Result<Self> {
        let artifact = Self {
            schema: CAMERA_ARTIFACT_SCHEMA.to_string(),
            capture_id: capture_id.into(),
            artifact_id: artifact_id.into(),
            video: video.into(),
            clock,
            media,
            terminal_state,
        };
        artifact.validate()?;
        Ok(artifact)
    }

    /// Validate structural invariants before an artifact is persisted, replayed,
    /// or accepted at the server boundary.
    pub fn validate(&self) -> Result<()> {
        if self.schema != CAMERA_ARTIFACT_SCHEMA {
            return Err(bad("camera artifact schema is unsupported"));
        }
        validate_identity("capture_id", &self.capture_id)?;
        validate_identity("artifact_id", &self.artifact_id)?;
        validate_relative_video(&self.video)?;
        if self.clock.end_frame_offset_ms < self.clock.first_frame_offset_ms {
            return Err(bad(
                "camera artifact end_frame_offset_ms precedes first_frame_offset_ms",
            ));
        }
        if self.media.width == 0 || self.media.height == 0 {
            return Err(bad("camera artifact dimensions must be non-zero"));
        }
        if self.media.fps_num == 0 || self.media.fps_den == 0 {
            return Err(bad(
                "camera artifact FPS numerator and denominator must be non-zero",
            ));
        }
        if self.media.frame_count == 0 || self.media.duration_ms == 0 {
            return Err(bad(
                "camera artifact frame_count and duration_ms must be non-zero",
            ));
        }
        let clock_duration = self
            .clock
            .end_frame_offset_ms
            .saturating_sub(self.clock.first_frame_offset_ms);
        if self.media.duration_ms != clock_duration {
            return Err(bad(
                "camera artifact duration_ms must equal its CaptureClock frame interval",
            ));
        }
        if !is_sha256_hex(&self.media.sha256) {
            return Err(bad(
                "camera artifact sha256 must be 64 lowercase hexadecimal characters",
            ));
        }
        Ok(())
    }
}

fn validate_identity(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(bad(&format!(
            "camera artifact {label} must be 1-128 ASCII letters, digits, '-' or '_'"
        )));
    }
    Ok(())
}

fn validate_relative_video(video: &str) -> Result<()> {
    if video.is_empty() || video.trim() != video || video.contains(['\\', ':']) {
        return Err(bad("camera artifact video must be a clean relative path"));
    }
    let path = Path::new(video);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_)
                    | Component::RootDir
                    | Component::CurDir
                    | Component::ParentDir
            )
        })
        || !path
            .components()
            .any(|component| matches!(component, Component::Normal(_)))
    {
        return Err(bad(
            "camera artifact video must stay contained under its capture directory",
        ));
    }
    Ok(())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn bad(message: &str) -> RecordError {
    RecordError::new(
        error_codes::INVALID_ARGS,
        message,
        "invalid CameraArtifact@1 contract",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_artifact() -> CameraArtifact {
        CameraArtifact::new(
            "cap_01",
            "camera_01",
            "camera/camera.mp4",
            CameraClockRange {
                first_frame_offset_ms: 500,
                end_frame_offset_ms: 1_500,
            },
            CameraMediaFacts {
                width: 1280,
                height: 720,
                fps_num: 30,
                fps_den: 1,
                frame_count: 30,
                duration_ms: 1_000,
                sha256: "a".repeat(64),
            },
            CameraTerminalState::Complete,
        )
        .unwrap()
    }

    #[test]
    fn camera_artifact_roundtrips_and_retains_clock_contract() {
        let artifact = fixture_artifact();
        let json = serde_json::to_string(&artifact).unwrap();
        let back: CameraArtifact = serde_json::from_str(&json).unwrap();
        assert_eq!(back, artifact);
        assert_eq!(back.schema, CAMERA_ARTIFACT_SCHEMA);
        assert_eq!(back.clock.first_frame_offset_ms, 500);
        assert_eq!(back.clock.end_frame_offset_ms, 1_500);
    }

    #[test]
    fn camera_artifact_rejects_escape_hash_and_timing_tampering() {
        let mut artifact = fixture_artifact();
        artifact.video = "../camera.mp4".into();
        assert!(artifact.validate().is_err());

        let mut artifact = fixture_artifact();
        artifact.video = "./camera.mp4".into();
        assert!(artifact.validate().is_err());

        let mut artifact = fixture_artifact();
        artifact.media.sha256 = "UPPERCASE".into();
        assert!(artifact.validate().is_err());

        let mut artifact = fixture_artifact();
        artifact.media.duration_ms = 999;
        assert!(artifact.validate().is_err());
    }
}
