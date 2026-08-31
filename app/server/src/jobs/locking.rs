//! Poison-aware access to the JobManager's in-memory bookkeeping.

use super::{JobManager, JobManagerInner};
use std::sync::MutexGuard;

impl JobManager {
    /// Acquire the sole JobManager lock path.
    ///
    /// Poisoning means an earlier holder unwound; it does not make the
    /// exclusively owned Rust values invalid. Lock scopes contain only manager
    /// bookkeeping: callers never run under this guard, and persistence errors
    /// are recorded on the affected job instead of panicking. Keeping the
    /// recovered value therefore preserves the last coherent job state, rather
    /// than turning a later status, update, cancellation, or terminal report
    /// into a second panic. Clear the advisory flag while holding the recovered
    /// guard so the incident is logged once and all later paths share the same
    /// recovered state.
    pub(super) fn lock_inner(&self) -> MutexGuard<'_, JobManagerInner> {
        match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => {
                tracing::error!("recovered poisoned JobManager lock after an earlier panic");
                self.inner.clear_poison();
                poisoned.into_inner()
            }
        }
    }

    #[cfg(test)]
    pub(super) fn poison_lock_for_tests(&self) {
        let inner = self.inner.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = inner.lock().expect("test takes the JobManager lock");
            panic!("deliberately poison the JobManager lock");
        });
    }
}
