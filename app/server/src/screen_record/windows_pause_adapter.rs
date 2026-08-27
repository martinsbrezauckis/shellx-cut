//! Private server translation for the Windows pause-ready WGC seam.
//!
//! It has no verb, UI, capture-start wiring, journal ownership, or recovery
//! reattachment. The injected factory is the sole source of a real validated
//! screen fragment, immutable artifact identity, settings, and event artifact.

use super::pause_session_owner::{
    PauseSessionStopRequest, PauseSessionWorkerAdapter, PauseSessionWorkerError,
};
use super::pause_worker_protocol::{PauseWorkerCommand, PauseWorkerFact, WorkerBoundaryKind};
use super::run_seal_coordinator::{SealedRunEvidence, SessionTimeOrigin};
use super::windows_pause_adapter_events::{
    ensure_evidence_generation, failed_event, observed, worker_fact,
};
pub(crate) use super::windows_pause_evidence_contract::WindowsPauseEvidenceFactory;
use record_capture::windows_pause_pilot::{
    WindowsPausePilotCommand, WindowsPausePilotCommandSender, WindowsPausePilotEvent,
    WindowsPausePilotEventReceiver, WindowsPausePilotOperation, WindowsPausePilotStarted,
    WindowsSealedScreenRun,
};
use record_recovery::RecordingStream;

/// Owner-held dispatch half. It can forward only a correlated screen-only
/// command; it has no event receiver, evidence, lifecycle, or journal access.
pub(crate) struct WindowsPauseDispatchAdapter {
    commands: WindowsPausePilotCommandSender,
}

impl WindowsPauseDispatchAdapter {
    pub(crate) fn new(commands: WindowsPausePilotCommandSender) -> Self {
        Self { commands }
    }
}

impl PauseSessionWorkerAdapter for WindowsPauseDispatchAdapter {
    fn dispatch(&mut self, command: &PauseWorkerCommand) -> Result<(), PauseSessionWorkerError> {
        if command.streams() != [RecordingStream::ScreenVideo] {
            return Err(PauseSessionWorkerError::new(
                "Windows pause pilot accepts screen video only",
            ));
        }
        let native = match command {
            PauseWorkerCommand::Pause {
                generation, epoch, ..
            } => WindowsPausePilotCommand::Pause {
                generation: *generation,
                epoch: epoch.value(),
            },
            PauseWorkerCommand::Resume {
                generation, epoch, ..
            } => WindowsPausePilotCommand::Resume {
                generation: *generation,
                epoch: epoch.value(),
            },
            PauseWorkerCommand::Stop { epoch, .. } => WindowsPausePilotCommand::Stop {
                epoch: epoch.value(),
            },
        };
        self.commands
            .send(native)
            .map_err(|_| PauseSessionWorkerError::new("Windows pause pilot worker is unavailable"))
    }
}

/// Controller-held event half. It binds a returned `StopSealRequest` before it
/// drains a queued terminal event, so the owner dispatch and terminal evidence
/// remain composable without making the owner a polling loop.
pub(crate) struct WindowsPauseEventTranslator<F> {
    events: WindowsPausePilotEventReceiver,
    evidence_factory: F,
    stop_expectation: Option<(Option<u64>, u64)>,
}

impl<F: WindowsPauseEvidenceFactory> WindowsPauseEventTranslator<F> {
    pub(crate) fn new(events: WindowsPausePilotEventReceiver, evidence_factory: F) -> Self {
        Self {
            events,
            evidence_factory,
            stop_expectation: None,
        }
    }

    /// One-shot bind after `PauseSessionOwner::request_stop_at` returns.
    pub(crate) fn expect_stop(&mut self, request: &PauseSessionStopRequest) {
        self.stop_expectation = Some((request.seal().generation(), request.epoch().value()));
    }

    pub(crate) fn set_session_origin(
        &mut self,
        origin: SessionTimeOrigin,
    ) -> Result<(), WindowsPauseAdapterError> {
        self.evidence_factory.set_session_origin(origin)
    }

    pub(crate) fn discard_staged(&mut self) {
        self.evidence_factory.discard_staged();
    }

    pub(crate) fn stop_generation(&self) -> Option<u64> {
        self.stop_expectation.and_then(|(generation, _)| generation)
    }

    pub(crate) fn retain_stop_run(&mut self) -> Result<(), WindowsPauseAdapterError> {
        let Some(expectation) = self.stop_expectation.as_mut() else {
            return Err(WindowsPauseAdapterError::MissingStopExpectation);
        };
        if expectation.0.is_none() {
            return Err(WindowsPauseAdapterError::UnexpectedStopRun);
        }
        expectation.0 = None;
        Ok(())
    }

    pub(crate) fn try_next(
        &mut self,
    ) -> Result<Option<WindowsPauseAdapterEvent>, WindowsPauseAdapterError> {
        let Some(event) = self
            .events
            .try_recv()
            .map_err(|_| WindowsPauseAdapterError::WorkerChannelClosed)?
        else {
            return Ok(None);
        };
        self.translate(event).map(Some)
    }

    fn translate(
        &mut self,
        event: WindowsPausePilotEvent,
    ) -> Result<WindowsPauseAdapterEvent, WindowsPauseAdapterError> {
        match event {
            WindowsPausePilotEvent::Started { started } => self
                .evidence_factory
                .stage_started(None, &started)
                .map(|()| WindowsPauseAdapterEvent::Started { started }),
            WindowsPausePilotEvent::PauseSealed {
                generation,
                epoch,
                run,
                observed_at,
            } => self.sealed_pause(generation, epoch, run, observed_at),
            WindowsPausePilotEvent::ResumeReady {
                generation,
                epoch,
                started,
            } => self
                .evidence_factory
                .stage_started(Some(generation), &started)
                .map(|()| WindowsPauseAdapterEvent::ResumeReady {
                    fact: worker_fact(
                        WorkerBoundaryKind::Resume,
                        generation,
                        epoch,
                        started.monotonic_at,
                    ),
                    started,
                }),
            WindowsPausePilotEvent::ResumeRefused {
                generation,
                epoch,
                observed_at,
                ..
            } => Ok(WindowsPauseAdapterEvent::ResumeRefused {
                fact: PauseWorkerFact::Refused {
                    stream: RecordingStream::ScreenVideo,
                    command: WorkerBoundaryKind::Resume,
                    generation,
                    observed_boundary: observed(epoch, observed_at),
                },
                observed_at,
            }),
            WindowsPausePilotEvent::StopSealed {
                epoch,
                run,
                observed_at,
            } => self.sealed_stop(epoch, run, observed_at),
            WindowsPausePilotEvent::Failed {
                operation,
                generation,
                epoch,
                observed_at,
            } => self.failed(operation, generation, epoch, observed_at),
        }
    }

    fn sealed_pause(
        &mut self,
        generation: u64,
        epoch: u64,
        run: WindowsSealedScreenRun,
        observed_at: std::time::Instant,
    ) -> Result<WindowsPauseAdapterEvent, WindowsPauseAdapterError> {
        let evidence = self
            .evidence_factory
            .verify_and_build(generation, &run, observed_at)?;
        ensure_evidence_generation(generation, &evidence)?;
        Ok(WindowsPauseAdapterEvent::PauseSealed {
            fact: worker_fact(WorkerBoundaryKind::Pause, generation, epoch, observed_at),
            evidence,
            observed_at,
        })
    }

    fn sealed_stop(
        &mut self,
        epoch: u64,
        run: Option<WindowsSealedScreenRun>,
        observed_at: std::time::Instant,
    ) -> Result<WindowsPauseAdapterEvent, WindowsPauseAdapterError> {
        let (expected, expected_epoch) = self
            .stop_expectation
            .take()
            .ok_or(WindowsPauseAdapterError::MissingStopExpectation)?;
        if epoch != expected_epoch {
            return Err(WindowsPauseAdapterError::UnexpectedStopEpoch);
        }
        match (expected, run) {
            (None, None) => Ok(WindowsPauseAdapterEvent::StopSealed {
                epoch,
                evidence: None,
                observed_at,
            }),
            (Some(generation), Some(run)) => {
                let evidence =
                    self.evidence_factory
                        .verify_and_build(generation, &run, observed_at)?;
                ensure_evidence_generation(generation, &evidence)?;
                Ok(WindowsPauseAdapterEvent::StopSealed {
                    epoch,
                    evidence: Some(evidence),
                    observed_at,
                })
            }
            (None, Some(run)) => {
                self.evidence_factory
                    .verify_discarded_stop(&run, observed_at)?;
                Ok(WindowsPauseAdapterEvent::StopSealed {
                    epoch,
                    evidence: None,
                    observed_at,
                })
            }
            _ => Err(WindowsPauseAdapterError::UnexpectedStopRun),
        }
    }

    fn failed(
        &self,
        operation: WindowsPausePilotOperation,
        generation: Option<u64>,
        epoch: u64,
        observed_at: std::time::Instant,
    ) -> Result<WindowsPauseAdapterEvent, WindowsPauseAdapterError> {
        if operation == WindowsPausePilotOperation::Stop
            && self
                .stop_expectation
                .is_some_and(|(_, expected_epoch)| epoch != expected_epoch)
        {
            return Err(WindowsPauseAdapterError::UnexpectedStopEpoch);
        }
        Ok(failed_event(operation, generation, epoch, observed_at))
    }
}

/// Server-private outcome after translating one observed native event.
pub(crate) enum WindowsPauseAdapterEvent {
    Started {
        started: WindowsPausePilotStarted,
    },
    PauseSealed {
        fact: PauseWorkerFact,
        evidence: SealedRunEvidence,
        observed_at: std::time::Instant,
    },
    ResumeReady {
        fact: PauseWorkerFact,
        started: WindowsPausePilotStarted,
    },
    ResumeRefused {
        fact: PauseWorkerFact,
        observed_at: std::time::Instant,
    },
    StopSealed {
        epoch: u64,
        evidence: Option<SealedRunEvidence>,
        observed_at: std::time::Instant,
    },
    FailedBoundary {
        fact: PauseWorkerFact,
    },
    StopFailed {
        epoch: u64,
        observed_at: std::time::Instant,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowsPauseAdapterError {
    WorkerChannelClosed,
    MissingStopExpectation,
    UnexpectedStopEpoch,
    UnexpectedStopRun,
    EvidenceRejected,
    CalibrationRejected,
}

#[cfg(test)]
#[path = "windows_pause_adapter_tests.rs"]
mod tests;
