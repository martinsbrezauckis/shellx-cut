//! Controller-held native event translation for the private Windows owner.

use super::WindowsPauseEvidenceFactory;
use crate::screen_record::pause_session_owner::PauseSessionStopRequest;
use crate::screen_record::pause_worker_protocol::{PauseWorkerFact, WorkerBoundaryKind};
use crate::screen_record::run_seal_coordinator::{SealedRunEvidence, SessionTimeOrigin};
use crate::screen_record::windows_pause_adapter_events::{
    ensure_evidence_generation, failed_event, observed, worker_fact,
};
use record_capture::windows_pause_pilot::{
    WindowsPausePilotEvent, WindowsPausePilotEventReceiver, WindowsPausePilotOperation,
    WindowsPausePilotStarted, WindowsSealedScreenRun,
};
use record_recovery::RecordingStream;

/// Controller-held event half. It binds a returned `StopSealRequest` before it
/// drains a queued terminal event, so the owner dispatch and terminal evidence
/// remain composable without making the owner a polling loop.
pub(crate) struct WindowsPauseEventTranslator<F> {
    events: WindowsPausePilotEventReceiver,
    evidence_factory: F,
    stop_expectation: Option<(Option<u64>, u64)>,
    streams: record_capture::SelectedCaptureStreams,
}

impl<F: WindowsPauseEvidenceFactory> WindowsPauseEventTranslator<F> {
    pub(crate) fn new(events: WindowsPausePilotEventReceiver, evidence_factory: F) -> Self {
        Self::with_streams(
            events,
            evidence_factory,
            record_capture::SelectedCaptureStreams::screen_only(),
        )
    }

    pub(crate) fn with_streams(
        events: WindowsPausePilotEventReceiver,
        evidence_factory: F,
        streams: record_capture::SelectedCaptureStreams,
    ) -> Self {
        Self {
            events,
            evidence_factory,
            stop_expectation: None,
            streams,
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
                input,
                audio,
                observed_at,
            } => {
                if input.is_some() || !self.matches_selected_audio(&audio) {
                    return Err(WindowsPauseAdapterError::EvidenceRejected);
                }
                self.sealed_pause(generation, epoch, run, audio, observed_at)
            }
            WindowsPausePilotEvent::ResumeReady {
                generation,
                epoch,
                started,
            } => {
                self.evidence_factory
                    .stage_started(Some(generation), &started)?;
                Ok(WindowsPauseAdapterEvent::ResumeReady {
                    fact: worker_fact(
                        WorkerBoundaryKind::Resume,
                        RecordingStream::ScreenVideo,
                        generation,
                        epoch,
                        started.monotonic_at,
                    ),
                    additional_facts: self.additional_facts(
                        WorkerBoundaryKind::Resume,
                        generation,
                        epoch,
                        started.monotonic_at,
                    ),
                    started,
                })
            }
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
                input,
                audio,
                observed_at,
            } => {
                if input.is_some() || !self.matches_selected_audio(&audio) {
                    return Err(WindowsPauseAdapterError::EvidenceRejected);
                }
                self.sealed_stop(epoch, run, audio, observed_at)
            }
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
        audio: Vec<record_capture::windows_pause_pilot::WindowsSealedAudioRun>,
        observed_at: std::time::Instant,
    ) -> Result<WindowsPauseAdapterEvent, WindowsPauseAdapterError> {
        let evidence = self.evidence_factory.verify_and_build_with_audio(
            generation,
            &run,
            &audio,
            observed_at,
        )?;
        ensure_evidence_generation(generation, &evidence)?;
        Ok(WindowsPauseAdapterEvent::PauseSealed {
            fact: worker_fact(
                WorkerBoundaryKind::Pause,
                RecordingStream::ScreenVideo,
                generation,
                epoch,
                observed_at,
            ),
            additional_facts: self.additional_facts(
                WorkerBoundaryKind::Pause,
                generation,
                epoch,
                observed_at,
            ),
            evidence,
            observed_at,
        })
    }

    fn sealed_stop(
        &mut self,
        epoch: u64,
        run: Option<WindowsSealedScreenRun>,
        audio: Vec<record_capture::windows_pause_pilot::WindowsSealedAudioRun>,
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
            (None, None) if audio.is_empty() => Ok(WindowsPauseAdapterEvent::StopSealed {
                epoch,
                evidence: None,
                observed_at,
            }),
            (Some(generation), Some(run)) => {
                let evidence = self.evidence_factory.verify_and_build_with_audio(
                    generation,
                    &run,
                    &audio,
                    observed_at,
                )?;
                ensure_evidence_generation(generation, &evidence)?;
                Ok(WindowsPauseAdapterEvent::StopSealed {
                    epoch,
                    evidence: Some(evidence),
                    observed_at,
                })
            }
            (None, Some(run)) if audio.is_empty() => {
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

    fn additional_facts(
        &self,
        kind: WorkerBoundaryKind,
        generation: u64,
        epoch: u64,
        observed_at: std::time::Instant,
    ) -> Vec<PauseWorkerFact> {
        self.streams
            .streams()
            .iter()
            .copied()
            .filter(|stream| *stream != RecordingStream::ScreenVideo)
            .map(|stream| worker_fact(kind, stream, generation, epoch, observed_at))
            .collect()
    }

    /// The worker event must carry one and only one owner result for every
    /// admitted audio stream. Facts are generated from the same private
    /// selection only after this check, so a missing, stale, duplicate, or
    /// unselected WAV can never complete a durable transition.
    fn matches_selected_audio(
        &self,
        audio: &[record_capture::windows_pause_pilot::WindowsSealedAudioRun],
    ) -> bool {
        let mut actual = audio.iter().map(|item| item.stream).collect::<Vec<_>>();
        actual.sort_unstable();
        let expected = self
            .streams
            .streams()
            .iter()
            .copied()
            .filter(|stream| *stream != RecordingStream::ScreenVideo)
            .collect::<Vec<_>>();
        actual == expected
    }
}

/// Server-private outcome after translating one observed native event.
pub(crate) enum WindowsPauseAdapterEvent {
    Started {
        started: WindowsPausePilotStarted,
    },
    PauseSealed {
        fact: PauseWorkerFact,
        additional_facts: Vec<PauseWorkerFact>,
        evidence: SealedRunEvidence,
        observed_at: std::time::Instant,
    },
    ResumeReady {
        fact: PauseWorkerFact,
        additional_facts: Vec<PauseWorkerFact>,
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
