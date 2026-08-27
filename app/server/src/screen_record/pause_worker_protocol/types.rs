use record_capture::SelectedCaptureStreams;
use record_recovery::RecordingStream;
use std::time::Instant;

/// Direction of one physical worker boundary command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerBoundaryKind {
    Pause,
    Resume,
}

/// Unique lifetime of one issued worker command. It is deliberately distinct
/// from the logical boundary generation, because `Stop` invalidates epochs
/// without creating a new logical generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct WorkerEpoch(u64);

impl WorkerEpoch {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

/// Identity observed by a physical worker at its actual boundary.
///
/// The monotonic instant is intentionally not a logical timestamp. A future
/// native owner must construct this only when the worker has observed its own
/// seal/readiness/refusal/failure boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ObservedBoundaryIdentity {
    epoch: WorkerEpoch,
    observed_at: Instant,
}

impl ObservedBoundaryIdentity {
    pub(crate) fn observed(epoch: WorkerEpoch, observed_at: Instant) -> Self {
        Self { epoch, observed_at }
    }

    pub(crate) fn epoch(self) -> WorkerEpoch {
        self.epoch
    }

    pub(crate) fn observed_at(self) -> Instant {
        self.observed_at
    }
}

/// Exact worker commands a future registry may fan out to every selected stream.
/// The command itself does not claim delivery or physical completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PauseWorkerCommand {
    Pause {
        generation: u64,
        epoch: WorkerEpoch,
        streams: SelectedCaptureStreams,
    },
    Resume {
        generation: u64,
        epoch: WorkerEpoch,
        streams: SelectedCaptureStreams,
    },
    Stop {
        epoch: WorkerEpoch,
        streams: SelectedCaptureStreams,
    },
}

impl PauseWorkerCommand {
    pub(super) fn boundary(
        kind: WorkerBoundaryKind,
        generation: u64,
        epoch: WorkerEpoch,
        streams: SelectedCaptureStreams,
    ) -> Self {
        match kind {
            WorkerBoundaryKind::Pause => Self::Pause {
                generation,
                epoch,
                streams,
            },
            WorkerBoundaryKind::Resume => Self::Resume {
                generation,
                epoch,
                streams,
            },
        }
    }

    pub(super) fn stop(epoch: WorkerEpoch, streams: SelectedCaptureStreams) -> Self {
        Self::Stop { epoch, streams }
    }

    pub(crate) fn boundary_kind(&self) -> Option<WorkerBoundaryKind> {
        match self {
            Self::Pause { .. } => Some(WorkerBoundaryKind::Pause),
            Self::Resume { .. } => Some(WorkerBoundaryKind::Resume),
            Self::Stop { .. } => None,
        }
    }

    pub(crate) fn generation(&self) -> Option<u64> {
        match self {
            Self::Pause { generation, .. } | Self::Resume { generation, .. } => Some(*generation),
            Self::Stop { .. } => None,
        }
    }

    pub(crate) fn epoch(&self) -> WorkerEpoch {
        match self {
            Self::Pause { epoch, .. } | Self::Resume { epoch, .. } | Self::Stop { epoch, .. } => {
                *epoch
            }
        }
    }

    pub(crate) fn streams(&self) -> &[RecordingStream] {
        match self {
            Self::Pause { streams, .. }
            | Self::Resume { streams, .. }
            | Self::Stop { streams, .. } => streams.streams(),
        }
    }
}

/// A physical, stream-owned result for a previously issued worker command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseWorkerFact {
    PauseSealed {
        stream: RecordingStream,
        generation: u64,
        observed_boundary: ObservedBoundaryIdentity,
    },
    ResumeReady {
        stream: RecordingStream,
        generation: u64,
        observed_boundary: ObservedBoundaryIdentity,
    },
    Refused {
        stream: RecordingStream,
        command: WorkerBoundaryKind,
        generation: u64,
        observed_boundary: ObservedBoundaryIdentity,
    },
    Failed {
        stream: RecordingStream,
        command: WorkerBoundaryKind,
        generation: u64,
        observed_boundary: ObservedBoundaryIdentity,
    },
}

impl PauseWorkerFact {
    pub(super) fn stream(self) -> RecordingStream {
        match self {
            Self::PauseSealed { stream, .. }
            | Self::ResumeReady { stream, .. }
            | Self::Refused { stream, .. }
            | Self::Failed { stream, .. } => stream,
        }
    }

    pub(super) fn generation(self) -> u64 {
        match self {
            Self::PauseSealed { generation, .. }
            | Self::ResumeReady { generation, .. }
            | Self::Refused { generation, .. }
            | Self::Failed { generation, .. } => generation,
        }
    }

    pub(super) fn boundary_kind(self) -> WorkerBoundaryKind {
        match self {
            Self::PauseSealed { .. } => WorkerBoundaryKind::Pause,
            Self::ResumeReady { .. } => WorkerBoundaryKind::Resume,
            Self::Refused { command, .. } | Self::Failed { command, .. } => command,
        }
    }

    pub(super) fn observed_boundary(self) -> ObservedBoundaryIdentity {
        match self {
            Self::PauseSealed {
                observed_boundary, ..
            }
            | Self::ResumeReady {
                observed_boundary, ..
            }
            | Self::Refused {
                observed_boundary, ..
            }
            | Self::Failed {
                observed_boundary, ..
            } => observed_boundary,
        }
    }

    pub(super) fn is_refused(self) -> bool {
        matches!(self, Self::Refused { .. })
    }

    pub(super) fn is_failed(self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Pure state of the command/fact correlation protocol, not a native status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseWorkerProtocolPhase {
    Recording,
    PausePending,
    Paused,
    ResumePending,
    Blocked,
    Stopped,
}

/// Deterministic reason a physical fact cannot affect the active boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseWorkerFactRejection {
    PostStop,
    WrongStream {
        stream: RecordingStream,
    },
    StaleGeneration {
        expected: u64,
        received: u64,
    },
    FutureGeneration {
        expected: u64,
        received: u64,
    },
    StaleEpoch {
        expected: WorkerEpoch,
        received: WorkerEpoch,
    },
    FutureEpoch {
        expected: WorkerEpoch,
        received: WorkerEpoch,
    },
    LateEpoch {
        generation: u64,
        epoch: WorkerEpoch,
    },
    WrongBoundary {
        expected: WorkerBoundaryKind,
        received: WorkerBoundaryKind,
    },
    DuplicateStream {
        stream: RecordingStream,
        generation: u64,
        epoch: WorkerEpoch,
    },
    PredatesCommand {
        epoch: WorkerEpoch,
    },
}

/// Result of considering one physical fact. A future native layer still owns
/// durable evidence, retry, and user-visible failure mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseWorkerFactResult {
    Accepted {
        phase: PauseWorkerProtocolPhase,
        completed_epoch: bool,
    },
    Refused {
        stream: RecordingStream,
        command: WorkerBoundaryKind,
        generation: u64,
        epoch: WorkerEpoch,
    },
    Failed {
        stream: RecordingStream,
        command: WorkerBoundaryKind,
        generation: u64,
        epoch: WorkerEpoch,
    },
    Rejected(PauseWorkerFactRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseWorkerCommandRejection {
    Stopped,
    Blocked,
    BoundaryPending,
    WrongPhase {
        expected: PauseWorkerProtocolPhase,
        actual: PauseWorkerProtocolPhase,
    },
    GenerationExhausted,
    EpochExhausted,
}
