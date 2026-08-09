//! Managed job runtime lifetime.
//!
//! Cancelling an async wait cannot stop an already-running `spawn_blocking`
//! closure. This module scopes every managed job to a small runtime and counts
//! blocking workers until their closures actually return.

use std::future::Future;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

tokio::task_local! {
    static CURRENT_JOB_RUNTIME: JobRuntime;
}

#[derive(Debug, Clone)]
struct JobRuntime {
    inner: Arc<JobRuntimeInner>,
}

#[derive(Debug)]
struct JobRuntimeInner {
    cancel_reason: AtomicU8,
    blocking_workers: AtomicUsize,
    process_workers: AtomicUsize,
    workers_changed: Notify,
}

impl JobRuntime {
    fn new() -> Self {
        Self {
            inner: Arc::new(JobRuntimeInner {
                cancel_reason: AtomicU8::new(JobCancellationReason::None as u8),
                blocking_workers: AtomicUsize::new(0),
                process_workers: AtomicUsize::new(0),
                workers_changed: Notify::new(),
            }),
        }
    }

    fn begin_blocking_worker(&self) -> JobBlockingWorkerGuard {
        self.inner.blocking_workers.fetch_add(1, Ordering::AcqRel);
        JobBlockingWorkerGuard {
            runtime: self.clone(),
        }
    }

    fn begin_process_worker(&self) -> JobProcessWorkerGuard {
        self.inner.process_workers.fetch_add(1, Ordering::AcqRel);
        JobProcessWorkerGuard {
            runtime: self.clone(),
        }
    }

    fn cancellation(&self) -> JobCancellation {
        JobCancellation {
            inner: Some(self.inner.clone()),
        }
    }

    fn request_cancel(&self, reason: JobCancellationReason) {
        let _ = self.inner.cancel_reason.compare_exchange(
            JobCancellationReason::None as u8,
            reason as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn has_blocking_workers(&self) -> bool {
        self.inner.blocking_workers.load(Ordering::Acquire) != 0
    }

    fn has_process_workers(&self) -> bool {
        self.inner.process_workers.load(Ordering::Acquire) != 0
    }

    fn has_workers(&self) -> bool {
        self.has_blocking_workers() || self.has_process_workers()
    }

    async fn wait_for_blocking_workers(&self) {
        loop {
            let changed = self.inner.workers_changed.notified();
            if !self.has_workers() {
                return;
            }
            changed.await;
        }
    }
}

/// Why a managed child must stop. This mirrors the persisted job outcome rather
/// than flattening every shutdown into an anonymous cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum JobCancellationReason {
    None = 0,
    CancelledByUser = 1,
    ProjectSwitch = 2,
    Restart = 3,
    Superseded = 4,
}

impl JobCancellationReason {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::CancelledByUser => "cancelled_by_user",
            Self::ProjectSwitch => "project_switch",
            Self::Restart => "restart",
            Self::Superseded => "superseded",
        }
    }
}

/// Cooperative cancellation passed into a blocking worker. It is deliberately
/// small and cloneable so a worker can hand it to every child-process wrapper.
/// Outside a managed job it stays inactive, preserving foreground callers.
#[derive(Debug, Clone)]
pub(crate) struct JobCancellation {
    inner: Option<Arc<JobRuntimeInner>>,
}

impl JobCancellation {
    fn inactive() -> Self {
        Self { inner: None }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.reason().is_some()
    }

    pub(crate) fn reason(&self) -> Option<JobCancellationReason> {
        let value = self
            .inner
            .as_ref()
            .map(|inner| inner.cancel_reason.load(Ordering::Acquire))?;
        match value {
            value if value == JobCancellationReason::CancelledByUser as u8 => {
                Some(JobCancellationReason::CancelledByUser)
            }
            value if value == JobCancellationReason::ProjectSwitch as u8 => {
                Some(JobCancellationReason::ProjectSwitch)
            }
            value if value == JobCancellationReason::Restart as u8 => {
                Some(JobCancellationReason::Restart)
            }
            value if value == JobCancellationReason::Superseded as u8 => {
                Some(JobCancellationReason::Superseded)
            }
            _ => None,
        }
    }

    pub(crate) fn request_cancel(&self) {
        self.request_cancel_with(JobCancellationReason::CancelledByUser);
    }

    pub(crate) fn request_cancel_with(&self, reason: JobCancellationReason) {
        if let Some(inner) = &self.inner {
            let _ = inner.cancel_reason.compare_exchange(
                JobCancellationReason::None as u8,
                reason as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn test_active() -> Self {
        JobRuntime::new().cancellation()
    }
}

/// Guard moved into the blocking closure itself, so dropping the awaiting
/// async future does not make the worker disappear from lifecycle accounting.
#[derive(Debug)]
pub(crate) struct JobBlockingWorkerGuard {
    runtime: JobRuntime,
}

impl Drop for JobBlockingWorkerGuard {
    fn drop(&mut self) {
        let prior = self
            .runtime
            .inner
            .blocking_workers
            .fetch_sub(1, Ordering::AcqRel);
        debug_assert!(prior > 0, "job blocking-worker counter underflow");
        self.runtime.inner.workers_changed.notify_waiters();
    }
}

/// Guard for an async process owner. It prevents task-level abort from
/// dropping the owner halfway through tree cleanup and makes job draining wait
/// until the child has actually been reaped.
#[derive(Debug)]
pub(crate) struct JobProcessWorkerGuard {
    runtime: JobRuntime,
}

impl Drop for JobProcessWorkerGuard {
    fn drop(&mut self) {
        let prior = self
            .runtime
            .inner
            .process_workers
            .fetch_sub(1, Ordering::AcqRel);
        debug_assert!(prior > 0, "job process-worker counter underflow");
        self.runtime.inner.workers_changed.notify_waiters();
    }
}

/// Called by the shared dispatch `run_blocking` bridge. Outside a managed job
/// there is no runtime to track, so foreground dispatch remains unchanged.
pub(crate) fn begin_current_blocking_worker() -> Option<JobBlockingWorkerGuard> {
    CURRENT_JOB_RUNTIME
        .try_with(JobRuntime::begin_blocking_worker)
        .ok()
}

/// Track an async external-process owner while it performs cancellation and
/// reaping. Outside a managed job this returns `None` for foreground callers.
pub(crate) fn begin_current_process_worker() -> Option<JobProcessWorkerGuard> {
    CURRENT_JOB_RUNTIME
        .try_with(JobRuntime::begin_process_worker)
        .ok()
}

/// Capture the managed job's cooperative cancellation signal before crossing
/// into `spawn_blocking`. Outside a job this is an inert signal.
pub(crate) fn current_job_cancellation() -> JobCancellation {
    CURRENT_JOB_RUNTIME
        .try_with(JobRuntime::cancellation)
        .unwrap_or_else(|_| JobCancellation::inactive())
}

#[derive(Debug)]
pub(super) struct JobTaskControl {
    handle: Option<tokio::task::JoinHandle<()>>,
    runtime: JobRuntime,
}

impl JobTaskControl {
    pub(super) fn spawn<F>(future: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let runtime = JobRuntime::new();
        let handle = tokio::spawn(CURRENT_JOB_RUNTIME.scope(runtime.clone(), future));
        Self {
            handle: Some(handle),
            runtime,
        }
    }

    pub(super) fn request_cancel(&self, reason: JobCancellationReason) {
        self.runtime.request_cancel(reason);
        // A cooperative process owner must stay alive long enough to kill and
        // reap its tree. Pure async work still receives Tokio abort semantics;
        // blocking work is tracked separately until its closure returns.
        if !self.runtime.has_process_workers() {
            if let Some(handle) = &self.handle {
                handle.abort();
            }
        }
    }

    pub(super) async fn wait_until(&mut self, deadline: tokio::time::Instant) -> bool {
        if let Some(handle) = self.handle.as_mut() {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() || tokio::time::timeout(remaining, handle).await.is_err() {
                return false;
            }
            self.handle = None;
        }
        if !self.runtime.has_workers() {
            return true;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        !remaining.is_zero()
            && tokio::time::timeout(remaining, self.runtime.wait_for_blocking_workers())
                .await
                .is_ok()
    }

    #[cfg(test)]
    pub(super) fn has_blocking_workers(&self) -> bool {
        self.runtime.has_workers()
    }
}
