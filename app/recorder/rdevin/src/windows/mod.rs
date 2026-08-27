#![allow(static_mut_refs)]

extern crate winapi;

use thiserror::Error;

mod common;
mod owned;

#[derive(Debug, Error)]
/// Errors that occur while owning Cut's passive Windows low-level hooks.
pub enum ListenError {
    #[error("{0}")]
    Hook(#[from] common::HookError),
    #[error("owned Windows listener message loop failed: {0}")]
    MessageLoop(u32),
    #[error("failed to post owned Windows listener stop message: {0}")]
    StopMessage(u32),
    #[error("owned Windows listener thread ended before startup completed")]
    ListenerThread,
}

pub(crate) use crate::windows::owned::OwnedListener;
