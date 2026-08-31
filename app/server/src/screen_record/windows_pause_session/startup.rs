use super::*;

impl<J, W, L> WindowsPauseSession<J, W, L>
where
    J: RecordingSessionJournalSink,
    W: WindowsPauseEvidenceFactory,
    L: WindowsPauseLifecycle,
{
    /// Consume the first independently observed native Started fact before a
    /// session exists. The production form also carries the owner of required
    /// per-run sidecars; generic ordering tests deliberately use the lean form.
    pub(crate) fn from_started_lifecycle(
        journal: J,
        admission: &WindowsPauseSessionAdmission,
        lifecycle: L,
        evidence_factory: W,
    ) -> Result<Self, WindowsPauseSessionError> {
        Self::from_started_lifecycle_inner(
            journal,
            admission.streams.clone(),
            lifecycle,
            evidence_factory,
            None,
        )
    }

    pub(crate) fn from_started_lifecycle_with_input_sidecars(
        journal: J,
        admission: &WindowsPauseSessionAdmission,
        lifecycle: L,
        evidence_factory: W,
        input_sidecars: super::super::windows_pause_input_sidecar::WindowsPauseInputSidecarOwner,
    ) -> Result<Self, WindowsPauseSessionError> {
        Self::from_started_lifecycle_inner(
            journal,
            admission.streams.clone(),
            lifecycle,
            evidence_factory,
            Some(input_sidecars),
        )
    }

    pub(crate) fn from_started_macos_lifecycle_with_input_sidecars(
        journal: J,
        streams: record_capture::SelectedCaptureStreams,
        lifecycle: L,
        evidence_factory: W,
        input_sidecars: super::super::windows_pause_input_sidecar::WindowsPauseInputSidecarOwner,
    ) -> Result<Self, WindowsPauseSessionError> {
        Self::from_started_lifecycle_inner(
            journal,
            streams,
            lifecycle,
            evidence_factory,
            Some(input_sidecars),
        )
    }

    fn from_started_lifecycle_inner(
        journal: J,
        streams: record_capture::SelectedCaptureStreams,
        mut lifecycle: L,
        evidence_factory: W,
        input_sidecars: Option<
            super::super::windows_pause_input_sidecar::WindowsPauseInputSidecarOwner,
        >,
    ) -> Result<Self, WindowsPauseSessionError> {
        let adapter =
            WindowsPauseDispatchAdapter::with_streams(lifecycle.command_sender(), streams.clone());
        let mut owner = match PauseSessionOwner::new(journal, streams.clone(), adapter) {
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
        let mut translator =
            WindowsPauseEventTranslator::with_streams(events, evidence_factory, streams);
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
            input_sidecars,
        })
    }
}
