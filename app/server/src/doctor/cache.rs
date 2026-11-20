//! One doctor cache shared by foreground verbs and background jobs.

use super::DoctorReport;
use crate::events::{Event, EventBus};
use cut_core::{error_codes, CutError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

#[derive(Clone, Debug)]
pub(crate) struct DoctorCache {
    report: Arc<RwLock<Option<DoctorReport>>>,
    addr: Arc<RwLock<Option<String>>>,
    events: EventBus,
    scan_gate: Arc<Mutex<()>>,
    invalidated: Arc<AtomicU64>,
    scanned: Arc<AtomicU64>,
    runtime: Option<tokio::runtime::Handle>,
}

impl DoctorCache {
    pub(crate) fn new(
        report: Arc<RwLock<Option<DoctorReport>>>,
        addr: Arc<RwLock<Option<String>>>,
        events: EventBus,
    ) -> Self {
        Self {
            report,
            addr,
            events,
            scan_gate: Arc::new(Mutex::new(())),
            invalidated: Arc::new(AtomicU64::new(0)),
            scanned: Arc::new(AtomicU64::new(0)),
            runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    fn stale(&self) -> bool {
        self.invalidated.load(Ordering::Acquire) != self.scanned.load(Ordering::Acquire)
    }

    /// Every tool_missing refusal uses this boundary, including registered
    /// tools other than FFmpeg. Invalidate before publishing a job failure so
    /// its next doctor read cannot return the old ready report. The background
    /// scan also reaches an idle UI through the existing doctor_updated event.
    pub(crate) fn invalidate_on_missing(&self, error: &CutError) -> bool {
        if error.code != error_codes::TOOL_MISSING {
            return false;
        }
        self.invalidated.fetch_add(1, Ordering::AcqRel);
        // Completion may be reported by an ordinary worker thread. Keep the
        // server runtime rather than requiring that thread to enter Tokio.
        if let Some(runtime) = self
            .runtime
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
        {
            let cache = self.clone();
            runtime.spawn(async move {
                cache.cached().await;
            });
        }
        true
    }

    pub(crate) async fn cached(&self) -> DoctorReport {
        if !self.stale() {
            if let Some(report) = self.report.read().await.clone() {
                return report;
            }
        }
        let _scan = self.scan_gate.lock().await;
        if !self.stale() {
            if let Some(report) = self.report.read().await.clone() {
                return report;
            }
        }
        self.scan_current().await
    }

    pub(crate) async fn rescan(&self) -> DoctorReport {
        let _scan = self.scan_gate.lock().await;
        self.scan_current().await
    }

    /// The gate serializes startup, explicit refreshes and missing-tool
    /// recovery. Capture only invalidations known before this scan: a newer
    /// refusal stays dirty for its queued scan/read instead of making this
    /// request repeat probes while other operations keep reporting failures.
    async fn scan_current(&self) -> DoctorReport {
        let generation = self.invalidated.load(Ordering::Acquire);
        let addr = self.addr.read().await.clone();
        let report = tokio::task::spawn_blocking(move || super::scan(addr))
            .await
            .unwrap_or_else(|_| super::scan_minimal());
        let changed = self
            .report
            .read()
            .await
            .as_ref()
            .is_none_or(|previous| !previous.same_capabilities(&report));
        *self.report.write().await = Some(report.clone());
        self.scanned.store(generation, Ordering::Release);
        crate::state::align_sidecar_ffmpeg_env();
        if changed {
            self.events.publish(Event::DoctorUpdated {
                report: report.clone(),
            });
        }
        report
    }
}
