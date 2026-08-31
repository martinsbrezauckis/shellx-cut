//! Compact native-event fact construction for the private pause translator.

use super::pause_worker_protocol::{
    ObservedBoundaryIdentity, PauseWorkerFact, WorkerBoundaryKind, WorkerEpoch,
};
use super::run_seal_coordinator::SealedRunEvidence;
use super::windows_pause_adapter::{WindowsPauseAdapterError, WindowsPauseAdapterEvent};
use record_capture::windows_pause_pilot::WindowsPausePilotOperation;
use record_recovery::RecordingStream;
use std::time::Instant;

pub(super) fn worker_fact(
    kind: WorkerBoundaryKind,
    stream: RecordingStream,
    generation: u64,
    epoch: u64,
    observed_at: Instant,
) -> PauseWorkerFact {
    let observed = observed(epoch, observed_at);
    match kind {
        WorkerBoundaryKind::Pause => PauseWorkerFact::PauseSealed {
            stream,
            generation,
            observed_boundary: observed,
        },
        WorkerBoundaryKind::Resume => PauseWorkerFact::ResumeReady {
            stream,
            generation,
            observed_boundary: observed,
        },
    }
}

pub(super) fn observed(epoch: u64, observed_at: Instant) -> ObservedBoundaryIdentity {
    ObservedBoundaryIdentity::observed(WorkerEpoch::new(epoch), observed_at)
}

pub(super) fn ensure_evidence_generation(
    generation: u64,
    evidence: &SealedRunEvidence,
) -> Result<(), WindowsPauseAdapterError> {
    (evidence.generation() == generation)
        .then_some(())
        .ok_or(WindowsPauseAdapterError::EvidenceRejected)
}

pub(super) fn failed_event(
    operation: WindowsPausePilotOperation,
    generation: Option<u64>,
    epoch: u64,
    observed_at: Instant,
) -> WindowsPauseAdapterEvent {
    let observed = observed(epoch, observed_at);
    match (operation, generation) {
        (WindowsPausePilotOperation::Pause, Some(generation)) => {
            WindowsPauseAdapterEvent::FailedBoundary {
                fact: PauseWorkerFact::Failed {
                    stream: RecordingStream::ScreenVideo,
                    command: WorkerBoundaryKind::Pause,
                    generation,
                    observed_boundary: observed,
                },
            }
        }
        (WindowsPausePilotOperation::Resume, Some(generation)) => {
            WindowsPauseAdapterEvent::FailedBoundary {
                fact: PauseWorkerFact::Failed {
                    stream: RecordingStream::ScreenVideo,
                    command: WorkerBoundaryKind::Resume,
                    generation,
                    observed_boundary: observed,
                },
            }
        }
        _ => WindowsPauseAdapterEvent::StopFailed { epoch, observed_at },
    }
}
