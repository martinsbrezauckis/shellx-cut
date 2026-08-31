//! Shared private session outcome types kept separate from the lifecycle owner.

use super::pause_session_owner::PauseSessionOwnerError;

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
    Owner(#[allow(dead_code)] PauseSessionOwnerError),
    Lifecycle,
    #[cfg(any(windows, target_os = "macos"))]
    Setup,
}
