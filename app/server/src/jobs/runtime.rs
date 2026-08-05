//! Managed job runtime lifetime.
//!
//! Tokio can abort an async task, but an already-running `spawn_blocking`
//! closure continues. This module scopes every managed job to a small runtime
//! and counts blocking workers until their closures actually return.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    blocking_workers: AtomicUsize,
    workers_changed: Notify,
}

impl JobRuntime {
    fn new() -> Self {
        Self {
            inner: Arc::new(JobRuntimeInner {
                blocking_workers: AtomicUsize::new(0),
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

    fn has_blocking_workers(&self) -> bool {
        self.inner.blocking_workers.load(Ordering::Acquire) != 0
    }

    async fn wait_for_blocking_workers(&self) {
        loop {
            let changed = self.inner.workers_changed.notified();
            if !self.has_blocking_workers() {
                return;
            }
            changed.await;
        }
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

/// Called by the shared dispatch `run_blocking` bridge. Outside a managed job
/// there is no runtime to track, so foreground dispatch remains unchanged.
pub(crate) fn begin_current_blocking_worker() -> Option<JobBlockingWorkerGuard> {
    CURRENT_JOB_RUNTIME
        .try_with(JobRuntime::begin_blocking_worker)
        .ok()
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

    pub(super) fn request_cancel(&self) {
        if let Some(handle) = &self.handle {
            handle.abort();
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
        if !self.runtime.has_blocking_workers() {
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
        self.runtime.has_blocking_workers()
    }
}
