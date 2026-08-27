//! Private Windows pause-pilot worker protocol.
//!
//! This is deliberately a native seam, not a public capture path. It is only
//! capable of one exact monitor, screen video, and integer FPS. No record server
//! type enters this module and it owns neither a journal nor lifecycle truth.

mod channel;
#[cfg(any(test, all(windows, feature = "capture-windows")))]
pub(crate) mod lifecycle;
#[cfg(any(test, all(windows, feature = "capture-windows")))]
mod logical_run;
#[cfg(all(windows, feature = "capture-windows"))]
mod native;
mod types;
#[cfg(any(test, all(windows, feature = "capture-windows")))]
#[allow(dead_code)] // The live lifecycle hook is deliberately not wired yet.
mod worker;
#[cfg(any(test, all(windows, feature = "capture-windows")))]
mod worker_commands;
#[cfg(any(test, all(windows, feature = "capture-windows")))]
mod worker_facts;

pub use channel::{
    channel, WindowsPausePilotChannelError, WindowsPausePilotCommandReceiver,
    WindowsPausePilotCommandSender, WindowsPausePilotEventReceiver, WindowsPausePilotEventSender,
};
#[cfg(all(windows, feature = "capture-windows"))]
#[doc(hidden)]
pub use lifecycle::WindowsPausePilotThread;

/// Windows-only construction for a future server-private pause-session owner.
/// It deliberately has no current caller in `screen_record.start`: no request
/// can select this path until a later slice constructs the matching durable
/// session, evidence factory, and terminal owner around the returned handle.
#[cfg(all(windows, feature = "capture-windows"))]
#[doc(hidden)]
pub fn start_private(
    profile: WindowsPausePilotProfile,
    checkpoint: crate::CheckpointConfig,
) -> Result<WindowsPausePilotThread, WindowsPausePilotStartError> {
    native::start_private(profile, checkpoint)
}
pub use types::{
    WindowsPausePilotAcceptedCapture, WindowsPausePilotCaptureRange,
    WindowsPausePilotCheckpointRange, WindowsPausePilotCommand, WindowsPausePilotEvent,
    WindowsPausePilotOperation, WindowsPausePilotProfile, WindowsPausePilotRefusal,
    WindowsPausePilotRequest, WindowsPausePilotStartError, WindowsPausePilotStarted,
    WindowsSealedScreenRun, WindowsSealedWgcCheckpoint,
};
