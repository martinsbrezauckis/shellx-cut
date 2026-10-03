//! Private Windows-only composition, with injected Linux ordering coverage.

use super::monitor_start_admission::Target;
use super::pause_session_owner::{
    PauseSessionFactOutcome, PauseSessionOwner, PauseSessionOwnerPhase,
};
use super::run_seal_coordinator::{RecordingSessionJournalSink, SessionTimeOrigin};
use super::windows_pause_adapter::{
    WindowsPauseAdapterEvent, WindowsPauseDispatchAdapter, WindowsPauseEventTranslator,
    WindowsPauseEvidenceFactory,
};
pub(crate) use super::windows_pause_session_types::{
    WindowsPauseSessionError, WindowsPauseSessionEvent,
};
use record_capture::windows_pause_pilot::{
    WindowsPausePilotChannelError, WindowsPausePilotCommandSender, WindowsPausePilotEventReceiver,
    WindowsPausePilotProfile, WindowsPausePilotRequest,
};
use record_capture::SelectedCaptureStreams;
use record_recovery::{RecordingSessionJournal, TerminalDisposition};
use std::time::Instant;

mod startup;
mod transition;

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
        let requested = streams.streams();
        let microphone = requested.contains(&record_recovery::RecordingStream::MicrophoneAudio);
        let system_audio = requested.contains(&record_recovery::RecordingStream::SystemAudio);
        if checkpoint_interval_ms == 0
            || streams != SelectedCaptureStreams::new(microphone, system_audio, false, false)
        {
            return Err(WindowsPauseSessionError::Admission);
        }
        let profile = WindowsPausePilotProfile::admit(WindowsPausePilotRequest::screen_with_audio(
            exact_monitor_id,
            fps,
            microphone,
            system_audio,
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

#[cfg(target_os = "macos")]
impl WindowsPauseLifecycle for record_capture::private_macos_pause_owner::MacosPausePilotThread {
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
    input_sidecars: Option<super::windows_pause_input_sidecar::WindowsPauseInputSidecarOwner>,
}

impl<J, W, L> WindowsPauseSession<J, W, L>
where
    J: RecordingSessionJournalSink,
    W: WindowsPauseEvidenceFactory,
    L: WindowsPauseLifecycle,
{
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
        let pending_pause = matches!(
            self.owner.phase(),
            PauseSessionOwnerPhase::PauseAwaitingFacts { .. }
                | PauseSessionOwnerPhase::PauseAwaitingSeal { .. }
        );
        let request = self
            .owner
            .request_stop_at(at)
            .map_err(|error| self.owner_error(error))?;
        if pending_pause {
            self.translator.expect_stop_after_pending_pause(&request);
        } else {
            self.translator.expect_stop(&request);
        }
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
                additional_facts,
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
                    if self.pin_recording_input_sidecar(&evidence).is_err() {
                        return Err(self.fail());
                    }
                    self.retained_stop_evidence = Some(evidence);
                    return Ok(Some(WindowsPauseSessionEvent::StopRunRetained));
                }
                if !self.accept_pause_facts(
                    fact,
                    additional_facts,
                    observed_at,
                    evidence.generation(),
                ) || self.pin_recording_input_sidecar(&evidence).is_err()
                    || self.owner.seal_pause_at(evidence, observed_at).is_err()
                {
                    return Err(self.fail());
                }
                Ok(Some(WindowsPauseSessionEvent::Paused))
            }
            WindowsPauseAdapterEvent::ResumeReady {
                fact,
                additional_facts,
                started,
            } => {
                if self.owner.phase() == PauseSessionOwnerPhase::Stopping {
                    return Ok(Some(WindowsPauseSessionEvent::StaleIgnored));
                }
                if !self.accept_resume_facts(
                    fact,
                    additional_facts,
                    started.monotonic_at,
                    fact_generation(fact),
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
                let (evidence, needs_input_pin) =
                    match (self.retained_stop_evidence.take(), evidence) {
                        (Some(retained), None) => (Some(retained), false),
                        (None, evidence) => (evidence, true),
                        (Some(_), Some(_)) => return Err(self.fail()),
                    };
                if needs_input_pin {
                    if let Some(evidence) = evidence.as_ref() {
                        if let Err(error) = self.pin_recording_input_sidecar(evidence) {
                            let _ = self.fail();
                            return Err(error);
                        }
                    }
                }
                if let Err(error) =
                    self.owner
                        .seal_stop_at(evidence, TerminalDisposition::Completed, observed_at)
                {
                    return Err(self.owner_error(error));
                }
                if let Err(error) = self.lifecycle.join_after_terminal() {
                    let _ = self.fail();
                    return Err(WindowsPauseSessionError::Join(error));
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

    /// Publish a completed projection only after the native Stop fact has been
    /// accepted, journaled, and joined. The caller still owns the ordinary
    /// capture recovery receipt; projection failure never invents completion.
    pub(crate) fn execute_completed_projection<P>(
        &mut self,
        projection: &mut P,
    ) -> Result<(), WindowsPauseSessionError>
    where
        P: super::pause_session_owner::PauseSessionProjectionExecutor,
    {
        self.owner
            .execute_completed_projection(projection)
            .map_err(|error| self.owner_error(error))
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

#[cfg(test)]
#[path = "windows_pause_session_tests.rs"]
mod tests;
