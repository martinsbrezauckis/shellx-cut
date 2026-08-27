//! Private Windows-only composition, with injected Linux ordering coverage.

use super::monitor_start_admission::Target;
use super::pause_session_owner::{
    PauseSessionFactOutcome, PauseSessionOwner, PauseSessionOwnerError, PauseSessionOwnerPhase,
};
use super::run_seal_coordinator::{RecordingSessionJournalSink, SessionTimeOrigin};
use super::windows_pause_adapter::{
    WindowsPauseAdapterError, WindowsPauseAdapterEvent, WindowsPauseDispatchAdapter,
    WindowsPauseEventTranslator, WindowsPauseEvidenceFactory,
};
use record_capture::windows_pause_pilot::{
    WindowsPausePilotChannelError, WindowsPausePilotCommandSender, WindowsPausePilotEventReceiver,
    WindowsPausePilotProfile, WindowsPausePilotRequest,
};
use record_capture::SelectedCaptureStreams;
use record_recovery::{RecordingSessionJournal, TerminalDisposition};
use std::time::Instant;

/// Purely admitted private input. It cannot carry a legacy index or an
/// optional stream: all filesystem and native work happens after this gate.
#[derive(Clone)]
pub(crate) struct WindowsPauseSessionAdmission {
    pub(super) profile: WindowsPausePilotProfile,
    pub(super) streams: SelectedCaptureStreams,
    pub(super) checkpoint_interval_ms: u64,
}

impl WindowsPauseSessionAdmission {
    pub(crate) fn admit(
        target: Target,
        streams: SelectedCaptureStreams,
        fps: f64,
        checkpoint_interval_ms: u64,
    ) -> Result<Self, WindowsPauseSessionError> {
        let exact_monitor_id = target.exact_id.ok_or(WindowsPauseSessionError::Admission)?;
        if streams != SelectedCaptureStreams::screen_only() || checkpoint_interval_ms == 0 {
            return Err(WindowsPauseSessionError::Admission);
        }
        let profile = WindowsPausePilotProfile::admit(WindowsPausePilotRequest::screen_video_only(
            exact_monitor_id,
            fps,
        ))
        .map_err(|_| WindowsPauseSessionError::Admission)?;
        Ok(Self {
            profile,
            streams,
            checkpoint_interval_ms,
        })
    }

    pub(super) fn profile(&self) -> &WindowsPausePilotProfile {
        &self.profile
    }
}

/// Owned lifecycle edge used by the server composition. A real WGC thread and
/// a deterministic Linux test double each expose the same ownership rules.
pub(crate) trait WindowsPauseLifecycle {
    fn command_sender(&self) -> WindowsPausePilotCommandSender;
    fn take_event_receiver(&mut self) -> Option<WindowsPausePilotEventReceiver>;
    fn shutdown_and_join(&mut self) -> Result<(), WindowsPausePilotChannelError>;
    fn join_after_terminal(&mut self) -> Result<(), WindowsPausePilotChannelError>;
}

#[cfg(windows)]
impl WindowsPauseLifecycle for record_capture::windows_pause_pilot::WindowsPausePilotThread {
    fn command_sender(&self) -> WindowsPausePilotCommandSender {
        self.command_sender()
    }

    fn take_event_receiver(&mut self) -> Option<WindowsPausePilotEventReceiver> {
        self.take_event_receiver()
    }

    fn shutdown_and_join(&mut self) -> Result<(), WindowsPausePilotChannelError> {
        self.shutdown_and_join()
    }

    fn join_after_terminal(&mut self) -> Result<(), WindowsPausePilotChannelError> {
        self.join_after_terminal()
    }
}

pub(crate) struct WindowsPauseSession<J, W, L>
where
    J: RecordingSessionJournalSink,
    W: WindowsPauseEvidenceFactory,
    L: WindowsPauseLifecycle,
{
    owner: PauseSessionOwner<J, WindowsPauseDispatchAdapter>,
    translator: WindowsPauseEventTranslator<W>,
    lifecycle: L,
    lifecycle_closed: bool,
    retained_stop_evidence: Option<super::run_seal_coordinator::SealedRunEvidence>,
}

impl<J, W, L> WindowsPauseSession<J, W, L>
where
    J: RecordingSessionJournalSink,
    W: WindowsPauseEvidenceFactory,
    L: WindowsPauseLifecycle,
{
    /// Consume the already-queued initial Started event before returning a
    /// session. The caller must have created/synced Intent before it starts the
    /// lifecycle supplied here.
    pub(crate) fn from_started_lifecycle(
        journal: J,
        admission: &WindowsPauseSessionAdmission,
        mut lifecycle: L,
        evidence_factory: W,
    ) -> Result<Self, WindowsPauseSessionError> {
        let adapter = WindowsPauseDispatchAdapter::new(lifecycle.command_sender());
        let mut owner = match PauseSessionOwner::new(journal, admission.streams.clone(), adapter) {
            Ok(owner) => owner,
            Err(error) => {
                let _ = lifecycle.shutdown_and_join();
                return Err(WindowsPauseSessionError::Owner(error));
            }
        };
        let Some(events) = lifecycle.take_event_receiver() else {
            owner.block();
            let _ = lifecycle.shutdown_and_join();
            return Err(WindowsPauseSessionError::InitialStarted);
        };
        let mut translator = WindowsPauseEventTranslator::new(events, evidence_factory);
        let started = match translator.try_next() {
            Ok(Some(WindowsPauseAdapterEvent::Started { started })) => started,
            Ok(_) | Err(_) => {
                translator.discard_staged();
                owner.block();
                let _ = lifecycle.shutdown_and_join();
                return Err(WindowsPauseSessionError::InitialStarted);
            }
        };
        let origin = SessionTimeOrigin::observed(started.monotonic_at, started.unix_ms);
        if translator.set_session_origin(origin).is_err()
            || owner.start_after_backend_origin(origin).is_err()
        {
            translator.discard_staged();
            owner.block();
            let _ = lifecycle.shutdown_and_join();
            return Err(WindowsPauseSessionError::InitialStarted);
        }
        Ok(Self {
            owner,
            translator,
            lifecycle,
            lifecycle_closed: false,
            retained_stop_evidence: None,
        })
    }

    pub(crate) fn request_pause_at(&mut self, at: Instant) -> Result<(), WindowsPauseSessionError> {
        self.owner
            .request_pause_at(at)
            .map(|_| ())
            .map_err(|error| self.owner_error(error))
    }

    pub(crate) fn request_resume_at(
        &mut self,
        at: Instant,
    ) -> Result<(), WindowsPauseSessionError> {
        self.owner
            .request_resume_at(at)
            .map_err(|error| self.owner_error(error))
    }

    /// Owner transition and native Stop dispatch happen first; only then is the
    /// translator allowed to consume its one expected terminal event.
    pub(crate) fn request_stop_at(&mut self, at: Instant) -> Result<(), WindowsPauseSessionError> {
        let request = self
            .owner
            .request_stop_at(at)
            .map_err(|error| self.owner_error(error))?;
        self.translator.expect_stop(&request);
        Ok(())
    }

    /// Translate at most one exact native event and make its matching durable
    /// transition. Every durable timestamp is supplied by the native event.
    pub(crate) fn pump_once(
        &mut self,
    ) -> Result<Option<WindowsPauseSessionEvent>, WindowsPauseSessionError> {
        let event = match self.translator.try_next() {
            Ok(event) => event,
            Err(error) => return Err(self.fail_adapter(error)),
        };
        let Some(event) = event else { return Ok(None) };
        match event {
            WindowsPauseAdapterEvent::PauseSealed {
                fact,
                evidence,
                observed_at,
            } => {
                if self.owner.phase() == PauseSessionOwnerPhase::Stopping {
                    if self.retained_stop_evidence.is_some()
                        || self.translator.stop_generation() != Some(evidence.generation())
                        || self.translator.retain_stop_run().is_err()
                    {
                        return Err(self.fail());
                    }
                    self.retained_stop_evidence = Some(evidence);
                    return Ok(Some(WindowsPauseSessionEvent::StopRunRetained));
                }
                if !matches!(
                    self.owner.accept_worker_fact(fact, observed_at),
                    Ok(PauseSessionFactOutcome::PauseFactsComplete { generation }) if generation == evidence.generation()
                ) || self.owner.seal_pause_at(evidence, observed_at).is_err()
                {
                    return Err(self.fail());
                }
                Ok(Some(WindowsPauseSessionEvent::Paused))
            }
            WindowsPauseAdapterEvent::ResumeReady { fact, started } => {
                if self.owner.phase() == PauseSessionOwnerPhase::Stopping {
                    return Ok(Some(WindowsPauseSessionEvent::StaleIgnored));
                }
                if !matches!(
                    self.owner.accept_worker_fact(fact, started.monotonic_at),
                    Ok(PauseSessionFactOutcome::ResumeFactsComplete { generation }) if generation == fact_generation(fact)
                ) || self.owner.seal_resume_at(started.monotonic_at).is_err()
                {
                    return Err(self.fail());
                }
                Ok(Some(WindowsPauseSessionEvent::Resumed))
            }
            WindowsPauseAdapterEvent::ResumeRefused { fact, observed_at } => {
                if self.owner.phase() == PauseSessionOwnerPhase::Stopping {
                    self.translator.discard_staged();
                    return Ok(Some(WindowsPauseSessionEvent::StaleIgnored));
                }
                if !matches!(
                    self.owner.accept_worker_fact(fact, observed_at),
                    Ok(PauseSessionFactOutcome::ResumeRefused { .. })
                ) {
                    return Err(self.fail());
                }
                self.translator.discard_staged();
                Ok(Some(WindowsPauseSessionEvent::ResumeRefused))
            }
            WindowsPauseAdapterEvent::StopSealed {
                evidence,
                observed_at,
                ..
            } => {
                let evidence = match (self.retained_stop_evidence.take(), evidence) {
                    (Some(retained), None) => Some(retained),
                    (None, evidence) => evidence,
                    (Some(_), Some(_)) => return Err(self.fail()),
                };
                if self
                    .owner
                    .seal_stop_at(evidence, TerminalDisposition::Completed, observed_at)
                    .is_err()
                {
                    return Err(self.fail());
                }
                if self.lifecycle.join_after_terminal().is_err() {
                    return Err(self.fail());
                }
                self.lifecycle_closed = true;
                Ok(Some(WindowsPauseSessionEvent::Stopped))
            }
            WindowsPauseAdapterEvent::FailedBoundary { .. }
            | WindowsPauseAdapterEvent::StopFailed { .. }
            | WindowsPauseAdapterEvent::Started { .. } => Err(self.fail()),
        }
    }

    pub(crate) fn phase(&self) -> PauseSessionOwnerPhase {
        self.owner.phase()
    }

    pub(crate) fn journal(&self) -> &RecordingSessionJournal {
        self.owner.journal()
    }

    fn fail_adapter(&mut self, _error: WindowsPauseAdapterError) -> WindowsPauseSessionError {
        self.fail()
    }

    fn owner_error(&mut self, error: PauseSessionOwnerError) -> WindowsPauseSessionError {
        if matches!(
            self.owner.phase(),
            PauseSessionOwnerPhase::Blocked | PauseSessionOwnerPhase::Stopping
        ) {
            self.fail()
        } else {
            WindowsPauseSessionError::Owner(error)
        }
    }

    fn fail(&mut self) -> WindowsPauseSessionError {
        self.translator.discard_staged();
        self.owner.block();
        let _ = self.lifecycle.shutdown_and_join();
        self.lifecycle_closed = true;
        WindowsPauseSessionError::Lifecycle
    }
}

impl<J, W, L> Drop for WindowsPauseSession<J, W, L>
where
    J: RecordingSessionJournalSink,
    W: WindowsPauseEvidenceFactory,
    L: WindowsPauseLifecycle,
{
    fn drop(&mut self) {
        self.translator.discard_staged();
        self.owner.block();
        if !self.lifecycle_closed {
            let _ = self.lifecycle.shutdown_and_join();
        }
    }
}

fn fact_generation(fact: super::pause_worker_protocol::PauseWorkerFact) -> u64 {
    match fact {
        super::pause_worker_protocol::PauseWorkerFact::PauseSealed { generation, .. }
        | super::pause_worker_protocol::PauseWorkerFact::ResumeReady { generation, .. }
        | super::pause_worker_protocol::PauseWorkerFact::Refused { generation, .. }
        | super::pause_worker_protocol::PauseWorkerFact::Failed { generation, .. } => generation,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowsPauseSessionEvent {
    Paused,
    Resumed,
    ResumeRefused,
    StopRunRetained,
    StaleIgnored,
    Stopped,
}

#[derive(Debug)]
pub(crate) enum WindowsPauseSessionError {
    Admission,
    InitialStarted,
    Owner(PauseSessionOwnerError),
    Lifecycle,
    #[cfg(windows)]
    Setup,
}

#[cfg(test)]
#[path = "windows_pause_session_tests.rs"]
mod tests;
