#![allow(static_mut_refs)]

// TODO: How TF does any of this work?

extern crate libc;
extern crate x11;

use thiserror::Error;

mod common;
mod keyboard;
mod owned;

#[derive(Debug, Error)]
/// Errors that occur while owning Cut's passive X11 RECORD listener.
pub enum ListenError {
    #[error("No displays")]
    NoDisplays,
    #[error("Failed to enable X11 recording context")]
    EnableRecordContext,
    #[error("Failed to create X11 recording context")]
    CreateRecordContext,
    #[error("Failed to allocate X11 recording range")]
    AllocateRecordRange,
    #[error("Failed to initialize X11 extension")]
    InitExtension,
    #[error("Failed to disable X11 recording context")]
    DisableRecordContext,
    #[error("Owned X11 listener thread ended before startup completed")]
    ListenerThread,
}

pub(crate) use crate::linux::owned::OwnedListener;
