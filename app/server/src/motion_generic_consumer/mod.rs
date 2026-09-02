//! Generic, data-driven ShellX Motion connector consumer.
//!
//! This is deliberately an injected transport boundary.  It does not discover
//! executables, create a coordinator, or turn a caller-supplied id into
//! authority.  A later authenticated two-process binding supplies the fixed
//! Motion transport without adding capability-specific Cut code.

mod adapter;
mod binding;
mod canonical;
mod catalog;
mod contract;
mod delivery;
mod descriptor;
mod events;
mod schema;
mod status;
mod status_fields;
mod syntax;

// Discovery is deliberately reusable without exposing the injected transport or
// a submission lane.  The runtime probe/catalog validators remain the single
// structural authority for every Cut-owned Motion discovery caller.
pub(crate) use contract::ConsumerContract;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MotionFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub remedy: Option<String>,
    pub retry_after_ms: Option<u64>,
    pub suggested_action: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConsumerError {
    Refused(String),
    Lookup(MotionFailure),
    Operation(MotionFailure),
    Transport(String),
}

impl ConsumerError {
    pub(crate) fn refusal(message: impl Into<String>) -> Self {
        Self::Refused(message.into())
    }
}
