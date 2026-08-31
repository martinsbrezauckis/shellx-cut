//! Private, platform-neutral ownership for one future pause-aware session.
//!
//! This module deliberately owns commands, epochs, durable journal transitions,
//! and final projection admission in one place. Native workers remain behind an
//! injected adapter; no verb, UI, process, or platform capture path calls it.

mod coordinator;
mod input;
mod types;

#[allow(unused_imports)]
pub(crate) use coordinator::PauseSessionOwner;
#[allow(unused_imports)]
pub(crate) use types::{
    PauseSessionFactOutcome, PauseSessionOwnerError, PauseSessionOwnerPhase,
    PauseSessionProjectionExecutor, PauseSessionStopRequest, PauseSessionWorkerAdapter,
    PauseSessionWorkerError,
};
