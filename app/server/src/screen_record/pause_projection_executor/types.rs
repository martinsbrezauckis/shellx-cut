use std::path::{Path, PathBuf};

use cut_core::CutError;
use record_core::FrameRate;
use record_recovery::{RunAwareStitchPlan, RunAwareStitchSpan};

/// Re-probed facts for an immutable sealed screen artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProjectionMediaFacts {
    pub duration_ms: u64,
    pub decoded_video_frames: u64,
    pub has_audio: bool,
    pub avg_frame_rate: Option<FrameRate>,
    pub r_frame_rate: Option<FrameRate>,
}

/// The exact path and journal facts a private source stager may consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedPauseProjectionSource {
    pub(super) run_sequence: u64,
    pub(super) checkpoint_sequence: u64,
    pub(super) path: PathBuf,
    pub(super) bytes: u64,
    pub(super) sha256: String,
    pub(super) duration_ms: u64,
    pub(super) decoded_video_frames: u64,
    pub(super) avg_frame_rate: Option<FrameRate>,
    pub(super) r_frame_rate: Option<FrameRate>,
}

impl VerifiedPauseProjectionSource {
    pub(crate) fn run_sequence(&self) -> u64 {
        self.run_sequence
    }

    pub(crate) fn checkpoint_sequence(&self) -> u64 {
        self.checkpoint_sequence
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(crate) fn decoded_video_frames(&self) -> u64 {
        self.decoded_video_frames
    }

    pub(crate) fn avg_frame_rate(&self) -> Option<FrameRate> {
        self.avg_frame_rate
    }

    pub(crate) fn r_frame_rate(&self) -> Option<FrameRate> {
        self.r_frame_rate
    }
}

/// Production verifies artifacts with an owned media verifier. Tests can use a
/// deterministic verifier without requiring a desktop encoder or a native worker.
pub(crate) trait PauseProjectionArtifactVerifier {
    fn verify(&self, path: &Path) -> Result<ProjectionMediaFacts, CutError>;
}

/// The production verifier used by a future private executor owner. It wraps
/// record-recovery's bounded probe-and-decode path without exposing tool paths
/// through any public recording command.
pub(crate) struct FfmpegPauseProjectionArtifactVerifier {
    ffmpeg: String,
    ffprobe: String,
}

impl FfmpegPauseProjectionArtifactVerifier {
    pub(crate) fn new(ffmpeg: impl Into<String>, ffprobe: impl Into<String>) -> Self {
        Self {
            ffmpeg: ffmpeg.into(),
            ffprobe: ffprobe.into(),
        }
    }
}

impl PauseProjectionArtifactVerifier for FfmpegPauseProjectionArtifactVerifier {
    fn verify(&self, path: &Path) -> Result<ProjectionMediaFacts, CutError> {
        let facts =
            record_recovery::verify_media(&self.ffmpeg, &self.ffprobe, path).map_err(|error| {
                CutError::new(
                    cut_core::error_codes::INVALID_ARGS,
                    "cannot execute pause-session legacy projection",
                    format!("verify sealed screen artifact: {error}"),
                )
            })?;
        Ok(ProjectionMediaFacts {
            duration_ms: facts.duration_ms,
            decoded_video_frames: facts.decoded_video_frames,
            has_audio: facts.has_audio,
            avg_frame_rate: facts.avg_frame_rate,
            r_frame_rate: facts.r_frame_rate,
        })
    }
}

/// Source assembly is deliberately private and injected: no current server path
/// may synthesize a pause-aware source until a native owner supplies this seam.
pub(crate) trait PauseProjectionSourceStager {
    fn stage(
        &mut self,
        sources: &[VerifiedPauseProjectionSource],
        plan: &RunAwareStitchPlan,
        output: &Path,
    ) -> Result<StagedPauseProjectionSource, CutError>;
}

/// A stager can report only the compact duration it actually assembled; the
/// executor independently requires it to match the sealed run-aware plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StagedPauseProjectionSource {
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseProjectionExecution {
    Published,
    AlreadyComplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PauseProjectionResult {
    pub execution: PauseProjectionExecution,
    pub source_duration_ms: u64,
}

pub(super) fn source_spans(
    plan: &RunAwareStitchPlan,
) -> impl Iterator<Item = (u64, u64, &str, &str, u64)> {
    plan.spans.iter().filter_map(|span| match span {
        RunAwareStitchSpan::Source {
            run_sequence,
            checkpoint_sequence,
            artifact,
            sha256,
            source_duration_ms,
            ..
        } => Some((
            *run_sequence,
            *checkpoint_sequence,
            artifact.as_str(),
            sha256.as_str(),
            *source_duration_ms,
        )),
        RunAwareStitchSpan::EncoderGapPadding { .. } => None,
    })
}
