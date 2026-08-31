//! Durable-recording projection for already sealed private camera evidence.

use record_core::{CameraArtifact, FrameRate, Result};
use record_recovery::{RecordingStream, StreamFragment, StreamFragmentFacts};

use crate::camera_session::invalid;

/// Validated private evidence ready for a future durable recording-session
/// owner. It does not write a journal itself, so only that owner decides which
/// screen run contains the camera prefix and when the append is durable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CameraSessionEvidence {
    pub(super) artifact: CameraArtifact,
    pub(super) bytes: u64,
}

impl CameraSessionEvidence {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "camera evidence remains private until the selected-camera owner appends it to the durable session journal"
        )
    )]
    pub(crate) fn artifact(&self) -> &CameraArtifact {
        &self.artifact
    }

    /// Bind this prefix to one physical screen run measured on the same
    /// `CaptureClock`. This does not compact paused time; a durable owner must
    /// separately project physical runs into its logical session timeline.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "camera evidence remains private until the selected-camera owner appends it to the durable session journal"
        )
    )]
    pub(crate) fn stream_fragment(
        &self,
        run_capture_clock_start_ms: u64,
        run_capture_clock_end_ms: u64,
        stream_sequence: u64,
    ) -> Result<StreamFragment> {
        self.artifact.validate()?;
        if run_capture_clock_end_ms <= run_capture_clock_start_ms {
            return Err(invalid("containing physical screen run is empty"));
        }
        if self.artifact.clock.first_frame_offset_ms < run_capture_clock_start_ms
            || self.artifact.clock.end_frame_offset_ms > run_capture_clock_end_ms
        {
            return Err(invalid(
                "camera prefix is not fully contained by the physical screen run",
            ));
        }
        let start_offset_ms = self
            .artifact
            .clock
            .first_frame_offset_ms
            .checked_sub(run_capture_clock_start_ms)
            .ok_or_else(|| invalid("camera prefix begins before the containing screen run"))?;
        let end_offset_ms = self
            .artifact
            .clock
            .end_frame_offset_ms
            .checked_sub(run_capture_clock_start_ms)
            .ok_or_else(|| invalid("camera prefix ends before the containing screen run"))?;
        if end_offset_ms <= start_offset_ms {
            return Err(invalid(
                "camera prefix is empty within the containing screen run",
            ));
        }
        let frame_rate = FrameRate::new(
            u64::from(self.artifact.media.fps_num),
            u64::from(self.artifact.media.fps_den),
        )
        .map_err(|_| invalid("camera media FPS cannot form journal frame-rate evidence"))?;
        Ok(StreamFragment {
            stream: RecordingStream::CameraVideo,
            checkpoint_sequence: None,
            stream_sequence,
            artifact: self.artifact.video.clone(),
            bytes: self.bytes,
            sha256: self.artifact.media.sha256.clone(),
            facts: StreamFragmentFacts {
                start_offset_ms,
                end_offset_ms,
                media_duration_ms: self.artifact.media.duration_ms,
                decoded_video_frames: Some(self.artifact.media.frame_count),
                avg_frame_rate: Some(frame_rate),
                r_frame_rate: Some(frame_rate),
            },
        })
    }
}
