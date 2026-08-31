//! Private dispatch half for the Windows pause-ready WGC seam.
//!
//! Event translation is split into its own bounded module so this owner remains
//! only the correlated command boundary; it has no journal or evidence access.

use super::pause_session_owner::{PauseSessionWorkerAdapter, PauseSessionWorkerError};
use super::pause_worker_protocol::PauseWorkerCommand;
pub(crate) use super::windows_pause_evidence_contract::WindowsPauseEvidenceFactory;
use record_capture::windows_pause_pilot::{
    WindowsPausePilotCommand, WindowsPausePilotCommandSender,
};

mod translator;
pub(crate) use translator::{
    WindowsPauseAdapterError, WindowsPauseAdapterEvent, WindowsPauseEventTranslator,
};

/// Owner-held dispatch half. It can forward only a correlated exact selected
/// command; it has no event receiver, evidence, lifecycle, or journal access.
pub(crate) struct WindowsPauseDispatchAdapter {
    commands: WindowsPausePilotCommandSender,
    streams: record_capture::SelectedCaptureStreams,
}

impl WindowsPauseDispatchAdapter {
    pub(crate) fn new(commands: WindowsPausePilotCommandSender) -> Self {
        Self::with_streams(
            commands,
            record_capture::SelectedCaptureStreams::screen_only(),
        )
    }

    pub(crate) fn with_streams(
        commands: WindowsPausePilotCommandSender,
        streams: record_capture::SelectedCaptureStreams,
    ) -> Self {
        Self { commands, streams }
    }
}

impl PauseSessionWorkerAdapter for WindowsPauseDispatchAdapter {
    fn dispatch(&mut self, command: &PauseWorkerCommand) -> Result<(), PauseSessionWorkerError> {
        if command.streams() != self.streams.streams() {
            return Err(PauseSessionWorkerError::new(
                "Windows pause pilot command streams do not match the private native owner",
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

#[cfg(test)]
#[path = "windows_pause_adapter_tests.rs"]
mod tests;
