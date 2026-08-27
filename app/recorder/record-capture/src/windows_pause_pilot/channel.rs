//! One owned command/event channel for the private pause pilot.

use super::{WindowsPausePilotCommand, WindowsPausePilotEvent};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

/// The owned worker endpoint disappeared. This carries no native detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPausePilotChannelError {
    Disconnected,
    NativeTerminal,
}

#[derive(Clone)]
pub struct WindowsPausePilotCommandSender(Sender<WindowsPausePilotCommand>);
pub struct WindowsPausePilotCommandReceiver(Receiver<WindowsPausePilotCommand>);
pub struct WindowsPausePilotEventSender(Sender<WindowsPausePilotEvent>);
pub struct WindowsPausePilotEventReceiver(Receiver<WindowsPausePilotEvent>);

/// Create the two neutral worker directions. The record-capture side owns the
/// receiver for commands and sender for native events; the server owns only the
/// inverse ends and cannot access WGC state.
pub fn channel() -> (
    WindowsPausePilotCommandSender,
    WindowsPausePilotCommandReceiver,
    WindowsPausePilotEventSender,
    WindowsPausePilotEventReceiver,
) {
    let (commands_sender, commands_receiver) = mpsc::channel();
    let (events_sender, events_receiver) = mpsc::channel();
    (
        WindowsPausePilotCommandSender(commands_sender),
        WindowsPausePilotCommandReceiver(commands_receiver),
        WindowsPausePilotEventSender(events_sender),
        WindowsPausePilotEventReceiver(events_receiver),
    )
}

impl WindowsPausePilotCommandSender {
    pub fn send(
        &self,
        command: WindowsPausePilotCommand,
    ) -> Result<(), WindowsPausePilotChannelError> {
        self.0
            .send(command)
            .map_err(|_| WindowsPausePilotChannelError::Disconnected)
    }
}

impl WindowsPausePilotCommandReceiver {
    pub fn try_recv(
        &self,
    ) -> Result<Option<WindowsPausePilotCommand>, WindowsPausePilotChannelError> {
        match self.0.try_recv() {
            Ok(command) => Ok(Some(command)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(WindowsPausePilotChannelError::Disconnected),
        }
    }

    /// Wait for one command without allowing a vanished peer or an idle server
    /// to strand an owned native WGC control indefinitely. `None` is only the
    /// bounded timeout; a disconnected command peer is terminal.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Option<WindowsPausePilotCommand>, WindowsPausePilotChannelError> {
        match self.0.recv_timeout(timeout) {
            Ok(command) => Ok(Some(command)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(WindowsPausePilotChannelError::Disconnected),
        }
    }
}

impl WindowsPausePilotEventSender {
    pub fn send(&self, event: WindowsPausePilotEvent) -> Result<(), WindowsPausePilotChannelError> {
        self.0
            .send(event)
            .map_err(|_| WindowsPausePilotChannelError::Disconnected)
    }
}

impl WindowsPausePilotEventReceiver {
    pub fn try_recv(
        &self,
    ) -> Result<Option<WindowsPausePilotEvent>, WindowsPausePilotChannelError> {
        match self.0.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(WindowsPausePilotChannelError::Disconnected),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_wait_distinguishes_idle_from_a_lost_command_peer() {
        let (commands, receiver, _events, _event_receiver) = channel();
        assert_eq!(
            receiver.recv_timeout(Duration::ZERO),
            Ok(None),
            "an idle peer is a bounded timeout, not terminal loss"
        );
        drop(commands);
        assert_eq!(
            receiver.recv_timeout(Duration::ZERO),
            Err(WindowsPausePilotChannelError::Disconnected)
        );
    }
}
