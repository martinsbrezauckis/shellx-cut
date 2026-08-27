use super::types::{
    PauseWorkerFact, PauseWorkerFactRejection, PauseWorkerProtocolPhase, WorkerBoundaryKind,
    WorkerEpoch,
};
use record_capture::SelectedCaptureStreams;
use record_recovery::RecordingStream;
use std::collections::BTreeSet;
use std::time::Instant;

#[derive(Debug)]
pub(super) struct PendingWorkerBoundary {
    pub(super) kind: WorkerBoundaryKind,
    pub(super) generation: u64,
    pub(super) epoch: WorkerEpoch,
    pub(super) issued_at: Instant,
    pub(super) acknowledged: BTreeSet<RecordingStream>,
}

impl PendingWorkerBoundary {
    pub(super) fn complete(&self, streams: &SelectedCaptureStreams) -> bool {
        self.acknowledged.len() == streams.streams().len()
    }
}

pub(super) fn validate_fact(
    fact: PauseWorkerFact,
    streams: &SelectedCaptureStreams,
    phase: PauseWorkerProtocolPhase,
    pending: Option<&PendingWorkerBoundary>,
    latest_generation: u64,
) -> Result<(), PauseWorkerFactRejection> {
    if phase == PauseWorkerProtocolPhase::Stopped {
        return Err(PauseWorkerFactRejection::PostStop);
    }
    let stream = fact.stream();
    if !streams.streams().contains(&stream) {
        return Err(PauseWorkerFactRejection::WrongStream { stream });
    }
    let Some(pending) = pending else {
        return Err(closed_fact_rejection(
            fact.generation(),
            fact.observed_boundary().epoch(),
            latest_generation,
        ));
    };
    compare_generation(fact.generation(), pending.generation)?;
    compare_epoch(fact.observed_boundary().epoch(), pending.epoch)?;
    if fact.boundary_kind() != pending.kind {
        return Err(PauseWorkerFactRejection::WrongBoundary {
            expected: pending.kind,
            received: fact.boundary_kind(),
        });
    }
    if fact.observed_boundary().observed_at() < pending.issued_at {
        return Err(PauseWorkerFactRejection::PredatesCommand {
            epoch: pending.epoch,
        });
    }
    if pending.acknowledged.contains(&stream) {
        return Err(PauseWorkerFactRejection::DuplicateStream {
            stream,
            generation: pending.generation,
            epoch: pending.epoch,
        });
    }
    Ok(())
}

fn closed_fact_rejection(
    received_generation: u64,
    epoch: WorkerEpoch,
    latest_generation: u64,
) -> PauseWorkerFactRejection {
    if received_generation < latest_generation {
        PauseWorkerFactRejection::StaleGeneration {
            expected: latest_generation,
            received: received_generation,
        }
    } else if received_generation > latest_generation {
        PauseWorkerFactRejection::FutureGeneration {
            expected: latest_generation,
            received: received_generation,
        }
    } else {
        PauseWorkerFactRejection::LateEpoch {
            generation: received_generation,
            epoch,
        }
    }
}

fn compare_generation(received: u64, expected: u64) -> Result<(), PauseWorkerFactRejection> {
    if received < expected {
        Err(PauseWorkerFactRejection::StaleGeneration { expected, received })
    } else if received > expected {
        Err(PauseWorkerFactRejection::FutureGeneration { expected, received })
    } else {
        Ok(())
    }
}

fn compare_epoch(
    received: WorkerEpoch,
    expected: WorkerEpoch,
) -> Result<(), PauseWorkerFactRejection> {
    if received < expected {
        Err(PauseWorkerFactRejection::StaleEpoch { expected, received })
    } else if received > expected {
        Err(PauseWorkerFactRejection::FutureEpoch { expected, received })
    } else {
        Ok(())
    }
}
