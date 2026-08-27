#![allow(static_mut_refs)]

use thiserror::Error;

mod common;
mod keyboard;
mod owned;

#[derive(Debug, Error)]
/// Errors that occur while owning Cut's passive macOS event tap.
pub enum ListenError {
    #[error("Failed to create event tap")]
    EventTapError,
    #[error("Failed to create run loop source")]
    LoopSourceError,
    #[error("Failed to get the listener thread run loop")]
    RunLoopError,
    #[error("Owned macOS listener thread ended before startup completed")]
    ListenerThread,
}

pub(crate) use crate::macos::owned::OwnedListener;
