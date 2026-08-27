//! Serializable v1 recording-session journal entries and immutable capture facts.

use serde::{Deserialize, Serialize};

pub const RECORDING_SESSION_JOURNAL_SCHEMA: &str = "shellx-cut/recording-session-journal/1";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SessionJournalError {
    #[error("recording session journal is invalid: {0}")]
    Invalid(String),
}

/// Individual capture sources are intentionally explicit: downstream recovery
/// must never conflate microphone, system, camera, screen, or input evidence.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RecordingStream {
    ScreenVideo,
    MicrophoneAudio,
    SystemAudio,
    CameraVideo,
    InputEvents,
}

impl RecordingStream {
    pub(crate) fn carries_video_frames(self) -> bool {
        matches!(self, Self::ScreenVideo | Self::CameraVideo)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordingSessionIntent {
    pub schema: String,
    pub session_id: String,
    pub created_unix_ms: u64,
    pub checkpoint_interval_ms: u64,
    /// Immutable requested capture cadence, not probed media cadence truth.
    pub fps: f64,
    /// Optional CaptureCadence@1 evidence. Older session journals carried only
    /// the legacy numeric request, so absence means unknown rather than a
    /// reconstructed rational or measured-media claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_cadence: Option<record_core::CaptureCadence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_duration_limit_ms: Option<u64>,
    pub record_keys: bool,
    /// Bounded, path-free identity for the selected target/source. It is an
    /// opaque descriptor, never a native window title or filesystem path.
    pub target_descriptor: String,
    /// Sorted, duplicate-free stream declaration. Sealed fragments may only name
    /// a stream that was part of this immutable request.
    pub requested_streams: Vec<RecordingStream>,
}

impl RecordingSessionIntent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: impl Into<String>,
        created_unix_ms: u64,
        checkpoint_interval_ms: u64,
        fps: f64,
        active_duration_limit_ms: Option<u64>,
        record_keys: bool,
        target_descriptor: impl Into<String>,
        requested_streams: Vec<RecordingStream>,
    ) -> Self {
        Self {
            schema: RECORDING_SESSION_JOURNAL_SCHEMA.into(),
            session_id: session_id.into(),
            created_unix_ms,
            checkpoint_interval_ms,
            fps,
            capture_cadence: None,
            active_duration_limit_ms,
            record_keys,
            target_descriptor: target_descriptor.into(),
            requested_streams,
        }
    }

    pub fn with_capture_cadence(mut self, capture_cadence: record_core::CaptureCadence) -> Self {
        self.capture_cadence = Some(capture_cadence);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_intent_json_defaults_capture_cadence_to_absent() {
        let intent: RecordingSessionIntent = serde_json::from_value(serde_json::json!({
            "schema": RECORDING_SESSION_JOURNAL_SCHEMA,
            "session_id": "legacy-session",
            "created_unix_ms": 1,
            "checkpoint_interval_ms": 100,
            "fps": 29.97,
            "active_duration_limit_ms": null,
            "record_keys": false,
            "target_descriptor": "opaque-target",
            "requested_streams": ["screen_video"],
        }))
        .unwrap();
        assert!(intent.capture_cadence.is_none());
    }

    #[test]
    fn legacy_fragment_json_defaults_probe_rates_to_absent() {
        let facts: StreamFragmentFacts = serde_json::from_value(serde_json::json!({
            "start_offset_ms": 0,
            "end_offset_ms": 100,
            "media_duration_ms": 100,
            "decoded_video_frames": 3,
        }))
        .unwrap();
        assert!(facts.avg_frame_rate.is_none());
        assert!(facts.r_frame_rate.is_none());
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordingSessionState {
    Started,
    Paused,
    Resumed,
    Stopping,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DurableStateTransition {
    pub sequence: u64,
    pub state: RecordingSessionState,
    /// Compact session time. A resumed run starts at this same offset rather than
    /// retaining elapsed wall-clock pause time.
    pub logical_offset_ms: u64,
    pub observed_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointSequenceRange {
    pub first: u64,
    pub last: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamFragmentFacts {
    /// Offsets are relative to the sealed run's compact logical interval.
    pub start_offset_ms: u64,
    pub end_offset_ms: u64,
    /// Duration independently measured from the sealed stream artifact. Video
    /// shorter than its observed span is padded by the stitch model.
    pub media_duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded_video_frames: Option<u64>,
    /// Exact average rate from the same completed-fragment verification pass as
    /// the decoded frame count. Absence remains compatible with v1 recovery,
    /// but a future frame-grid pause projection must refuse it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_frame_rate: Option<record_core::FrameRate>,
    /// Exact nominal rate from that same verification pass. Strict CFR pause
    /// projection requires it to agree with `avg_frame_rate`; it never selects
    /// one ambiguous probe field as the source of truth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r_frame_rate: Option<record_core::FrameRate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamFragment {
    pub stream: RecordingStream,
    /// `screen_video` alone references the current v1 checkpoint sequence. Other
    /// streams are independently finalized and use `stream_sequence` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_sequence: Option<u64>,
    /// Contiguous sequence scoped to one stream within one sealed run.
    pub stream_sequence: u64,
    pub artifact: String,
    pub bytes: u64,
    pub sha256: String,
    pub facts: StreamFragmentFacts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SealedRun {
    pub sequence: u64,
    /// The source clock is retained for audit only. Gaps between successive runs
    /// are intentional pauses and never become source-timeline padding.
    pub observed_start_ms: u64,
    pub observed_end_ms: u64,
    pub logical_start_ms: u64,
    pub logical_end_ms: u64,
    pub checkpoints: CheckpointSequenceRange,
    /// Canonical order is stream, then stream-local sequence.
    pub fragments: Vec<StreamFragment>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalDisposition {
    Completed,
    Interrupted,
    Discarded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTerminal {
    pub disposition: TerminalDisposition,
    pub logical_end_ms: u64,
    pub observed_unix_ms: u64,
}

/// JSONL-compatible entries. The intent entry must be first and occurs exactly
/// once; every later entry is accepted only through the state machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordingSessionJournalEntry {
    Intent(RecordingSessionIntent),
    Transition(DurableStateTransition),
    Run(SealedRun),
    Terminal(SessionTerminal),
}
