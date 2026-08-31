//! Private macOS pause-pilot ownership.
//!
//! This is not a second public recorder. It supplies crate-private ownership rules
//! for an exact ScreenCaptureKit display run and independently sealed
//! microphone/Core Audio leaves. It has no server journal/projection caller,
//! public capture admission, verb, UI control, or installed-support claim.

mod audio;
mod close;
mod control;
mod owner;
mod types;
mod validation;

#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod native;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod screen;

pub(crate) use audio::{
    abort_selected, seal_selected_after_screen, start_for_profile, MacosPauseAudioFactory,
    MacosPauseAudioOwner,
};
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
pub(crate) use native::RequiredMacosPauseAudioFactory;
pub(crate) use owner::{MacosPauseRunOwner, MacosPauseScreenOwner};
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
pub(crate) use screen::RequiredMacosPauseScreenOwner;
pub(crate) use types::{
    MacosPauseAcceptedScreen, MacosPauseAudioLayout, MacosPauseCommand, MacosPauseCommandRejection,
    MacosPausePilotEvent, MacosPausePilotProfile, MacosPausePilotRefusal, MacosPausePilotRequest,
    MacosPausePilotStarted, MacosPauseScreenRange, MacosPauseStartError, MacosSealedAudioRun,
    MacosSealedScreenRun,
};

#[cfg(test)]
mod tests;
