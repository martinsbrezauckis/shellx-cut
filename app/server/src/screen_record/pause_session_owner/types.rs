use super::super::pause_projection::SealedLegacyProjectionRun;
use super::super::pause_worker_protocol::WorkerEpoch;
use super::super::pause_worker_protocol::{
    PauseWorkerCommand, PauseWorkerCommandRejection, PauseWorkerFactRejection,
};
use super::super::run_seal_coordinator::{RunSealCoordinatorError, StopSealRequest};
use record_recovery::RecordingSessionJournal;

/// The only lifecycle phases a future native owner may report internally.
///
/// `PauseAwaitingSeal` and `ResumeAwaitingDurability` are intentionally
/// distinct from `Paused` and `Recording`: receiving worker facts alone never
/// claims that a journal transition was made durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseSessionOwnerPhase {
    Preparing,
    Recording,
    PauseAwaitingFacts { generation: u64 },
    PauseAwaitingSeal { generation: u64 },
    Paused,
    ResumeAwaitingFacts { generation: u64 },
    ResumeAwaitingDurability { generation: u64 },
    Blocked,
    Stopping,
    Stopped,
}

/// Observable result of considering one independently observed worker fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseSessionFactOutcome {
    AwaitingWorkerFacts,
    PauseFactsComplete { generation: u64 },
    ResumeFactsComplete { generation: u64 },
    ResumeRefused { generation: u64 },
    Blocked,
}

/// Bind one durable Stop expectation to the exact native worker epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PauseSessionStopRequest {
    seal: StopSealRequest,
    epoch: WorkerEpoch,
}

impl PauseSessionStopRequest {
    pub(super) fn new(seal: StopSealRequest, epoch: WorkerEpoch) -> Self {
        Self { seal, epoch }
    }

    pub(crate) fn seal(&self) -> &StopSealRequest {
        &self.seal
    }
    pub(crate) fn epoch(&self) -> WorkerEpoch {
        self.epoch
    }
    pub(crate) fn requires_sealed_run(&self) -> bool {
        self.seal.requires_sealed_run()
    }
}

/// The opaque dispatch failure from a platform worker adapter.
///
/// Command adapters supply no target title, credential, or provider detail.
/// Completed projection adapters retain a bounded media-verification cause
/// so terminal capture errors identify the failed admission or assembly step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PauseSessionWorkerError(String);

impl PauseSessionWorkerError {
    pub(crate) fn new(detail: impl Into<String>) -> Self {
        Self(detail.into())
    }

    pub(crate) fn detail(&self) -> &str {
        &self.0
    }
}

/// Platform-native command delivery boundary.
///
/// The adapter receives only generation-and-epoch-correlated commands. It must
/// return separately observed worker facts and exact sealed run evidence to the
/// owner; a Windows adapter derives that evidence only after its private
/// `WgcRunOwner` has closed and published the exact run. Dispatch is never
/// treated as physical completion.
pub(crate) trait PauseSessionWorkerAdapter {
    fn dispatch(&mut self, command: &PauseWorkerCommand) -> Result<(), PauseSessionWorkerError>;
}

/// Adapter over the private completed-session projection executor.
///
/// A production implementation must call `execute_pause_projection` only with
/// its validated `CaptureRoot`, media verifier, and stager. This generic seam
/// keeps the session owner platform-neutral and makes the ordering testable.
pub(crate) trait PauseSessionProjectionExecutor {
    fn execute(
        &mut self,
        journal: &RecordingSessionJournal,
        event_runs: &[SealedLegacyProjectionRun],
    ) -> Result<(), PauseSessionWorkerError>;
}

/// Fail-closed private owner error. It is not a public verb error mapping.
#[derive(Debug)]
pub(crate) enum PauseSessionOwnerError {
    RunSeal(RunSealCoordinatorError),
    WorkerCommand(PauseWorkerCommandRejection),
    WorkerFact(PauseWorkerFactRejection),
    WorkerDispatch(PauseSessionWorkerError),
    Projection(PauseSessionWorkerError),
    WrongPhase {
        action: &'static str,
        actual: PauseSessionOwnerPhase,
    },
    MismatchedGeneration {
        expected: u64,
        received: u64,
    },
    ProjectionUnavailable,
}

impl From<RunSealCoordinatorError> for PauseSessionOwnerError {
    fn from(error: RunSealCoordinatorError) -> Self {
        Self::RunSeal(error)
    }
}
