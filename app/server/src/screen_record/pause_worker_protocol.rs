//! Private command/fact correlation for a future pause-aware worker registry.
//!
//! This is deliberately not a native pause implementation. It only gives a
//! later owner typed, generation-and-epoch-correlated commands plus a pure way
//! to reject physical facts that cannot belong to the currently pending edge.

mod coordinator;
mod types;
mod validation;

#[allow(unused_imports)]
pub(crate) use coordinator::PauseWorkerCoordinator;
#[allow(unused_imports)]
pub(crate) use types::{
    ObservedBoundaryIdentity, PauseWorkerCommand, PauseWorkerCommandRejection, PauseWorkerFact,
    PauseWorkerFactRejection, PauseWorkerFactResult, PauseWorkerProtocolPhase, WorkerBoundaryKind,
    WorkerEpoch,
};
