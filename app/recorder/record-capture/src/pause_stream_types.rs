//! Public command, acknowledgement, and status types for pause coordination.

use crate::{SelectedCaptureStreams, SessionPhase, SessionTransitionIgnored};
use record_recovery::RecordingStream;
use serde::Serialize;

/// The operation requested from every selected stream at one generation boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamBoundaryKind {
    Pause,
    Resume,
}

/// A single, generation-tagged command that a future server registry can fan out.
///
/// The coordinator does not send this command or claim that any stream has acted
/// on it. The registry must return explicit [`StreamAcknowledgement`] values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamBoundary {
    pub kind: StreamBoundaryKind,
    pub generation: u64,
    streams: SelectedCaptureStreams,
}

impl StreamBoundary {
    pub(crate) fn new(
        kind: StreamBoundaryKind,
        generation: u64,
        streams: SelectedCaptureStreams,
    ) -> Self {
        Self {
            kind,
            generation,
            streams,
        }
    }

    /// The exact immutable stream set that must acknowledge this boundary.
    pub fn streams(&self) -> &[RecordingStream] {
        self.streams.streams()
    }
}

/// An acknowledgement from one stream for a previously issued boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamAcknowledgement {
    /// The stream sealed its current output and will emit no later timestamp for
    /// this generation.
    PauseSealed {
        stream: RecordingStream,
        generation: u64,
    },
    /// The stream is ready to resume from the frozen logical boundary.
    ResumeReady {
        stream: RecordingStream,
        generation: u64,
    },
    /// The stream declined a resume request. This is not a completion: it moves
    /// the logical session back to safely paused without re-enabling timestamps.
    ResumeRefused {
        stream: RecordingStream,
        generation: u64,
    },
}

impl StreamAcknowledgement {
    pub(crate) fn stream(self) -> RecordingStream {
        match self {
            Self::PauseSealed { stream, .. }
            | Self::ResumeReady { stream, .. }
            | Self::ResumeRefused { stream, .. } => stream,
        }
    }

    pub(crate) fn generation(self) -> u64 {
        match self {
            Self::PauseSealed { generation, .. }
            | Self::ResumeReady { generation, .. }
            | Self::ResumeRefused { generation, .. } => generation,
        }
    }

    pub(crate) fn kind(self) -> StreamBoundaryKind {
        match self {
            Self::PauseSealed { .. } => StreamBoundaryKind::Pause,
            Self::ResumeReady { .. } | Self::ResumeRefused { .. } => StreamBoundaryKind::Resume,
        }
    }

    pub(crate) fn is_refusal(self) -> bool {
        matches!(self, Self::ResumeRefused { .. })
    }
}

/// A resume refusal retained as a bounded, registry-safe status fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StreamRefusal {
    pub stream: RecordingStream,
    pub generation: u64,
}

/// Why a boundary request was not issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundaryRequestResult {
    /// One command must be delivered to every stream in its immutable selection.
    Issued(StreamBoundary),
    /// The requested edge is not valid from the current logical phase.
    Ignored {
        phase: SessionPhase,
        reason: SessionTransitionIgnored,
    },
    /// The monotonically increasing generation domain has been exhausted.
    GenerationExhausted { phase: SessionPhase },
}

/// A deterministic rejection for an acknowledgement that cannot advance state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcknowledgementRejection {
    /// `Stop` is terminal and wins over every pending or late acknowledgement.
    Stopped,
    /// The reporter was not registered in the immutable stream selection.
    UnknownStream { stream: RecordingStream },
    /// No boundary has ever been issued, so an acknowledgement has no owner.
    NoBoundary { received: u64 },
    /// The acknowledgement belongs to a boundary that was superseded or completed.
    StaleGeneration { expected: u64, received: u64 },
    /// The acknowledgement arrived before its generation was issued.
    FutureGeneration { expected: u64, received: u64 },
    /// The acknowledgement has the wrong direction for the current boundary.
    WrongBoundary {
        expected: StreamBoundaryKind,
        received: StreamBoundaryKind,
    },
    /// That stream already acknowledged the active generation.
    DuplicateStream {
        stream: RecordingStream,
        generation: u64,
    },
}

/// The outcome of one [`StreamAcknowledgement`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcknowledgementResult {
    /// The acknowledgement was accepted. `completed_boundary` is true only when
    /// every selected stream has acknowledged the command.
    Accepted {
        phase: SessionPhase,
        completed_boundary: bool,
    },
    /// A selected stream declined resume, so the coordinator safely returned to
    /// `Paused` with the logical timestamp boundary still frozen.
    ResumeRefused(StreamRefusal),
    /// The acknowledgement was ignored without changing the logical clock.
    Rejected(AcknowledgementRejection),
}

/// Status facts for a future server-side worker registry.
///
/// This snapshot is observation-only: it neither sends a boundary nor assumes a
/// worker completed one. Millisecond values use saturating conversion so status
/// remains representable even for an exceptionally long-running process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PauseStreamCoordinatorStatus {
    pub phase: SessionPhase,
    pub selected_streams: SelectedCaptureStreams,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_boundary: Option<PendingStreamBoundary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_generation: Option<u64>,
    pub timestamp_issuance_enabled: bool,
    pub logical_elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_active_duration_ms: Option<u64>,
    pub active_duration_exhausted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_resume_refusal: Option<StreamRefusal>,
}

/// The active boundary and the exact streams still awaited for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingStreamBoundary {
    pub boundary: StreamBoundary,
    pub awaiting_streams: Vec<RecordingStream>,
}
