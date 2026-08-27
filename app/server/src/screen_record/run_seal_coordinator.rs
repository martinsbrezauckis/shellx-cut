//! Private durable ordering for a future pause-aware screen-record session.
//!
//! This component owns no worker registry, native capture call, legacy-root
//! writer, or public command. It turns already-observed backend facts into the
//! only ordering the v1 session journal may accept: intent first, then started,
//! then sealed runs and their logical boundaries. A future native layer must
//! supply the sealed evidence; it must not treat a logical acknowledgement as
//! evidence that an artifact exists.

mod boundaries;
mod coordinator;
mod transitions;
mod types;
mod validation;

// This is a server-private integration seam for the later native coordinator.
// It is exported only to sibling server modules; no verb or live registry calls
// it in this slice.
#[allow(unused_imports)]
pub(crate) use coordinator::RunSealCoordinator;
#[allow(unused_imports)]
pub(crate) use types::{
    PauseSealRequest, RecordingSessionJournalSink, ResumeReadinessEvidence,
    RunSealCoordinatorError, SealedRunEvidence, SessionTimeOrigin, StopSealRequest,
};
