//! One doctor cache shared by foreground verbs and background jobs.

use super::fast_scan::{
    apply_deferred_result, should_emit_deferred_update, CancelOnDrop, DeferredWork,
};
use super::DoctorReport;
use crate::events::{Event, EventBus};
use cut_core::{error_codes, CutError};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    scan_seq: Arc<AtomicU64>,
    cancel_on_drop: Arc<CancelOnDrop>,
    background_enrichment: Arc<AtomicBool>,
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
            scan_seq: Arc::new(AtomicU64::new(0)),
            cancel_on_drop: Arc::new(CancelOnDrop::default()),
            background_enrichment: Arc::new(AtomicBool::new(true)),
            runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    /// One-shot standalone CLI invocations exit right after printing JSON with
    /// no WS listener; skip spawning deferred background probes that would be
    /// cancelled immediately on shutdown.
    pub(crate) fn disable_background_enrichment(&self) {
        self.background_enrichment.store(false, Ordering::Release);
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
        self.scan_seq.fetch_add(1, Ordering::AcqRel);
        self.cancel_on_drop.cancel_active();
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
            let snapshot = self.report.read().await.clone();
            if let Some(report) = snapshot {
                let missing = super::chosen_ffmpeg::missing_chosen_path();
                if missing.as_deref() == super::chosen_ffmpeg::report_has_chosen_missing(&report) {
                    return report;
                }
            }
        }
        let _scan = self.scan_gate.lock().await;
        if !self.stale() {
            let snapshot = self.report.read().await.clone();
            if let Some(mut report) = snapshot {
                match (
                    super::chosen_ffmpeg::missing_chosen_path(),
                    super::chosen_ffmpeg::report_has_chosen_missing(&report),
                ) {
                    (None, None) => return report,
                    (Some(ref chosen), Some(reported)) if chosen == reported => return report,
                    (Some(chosen), _) => {
                        self.scan_seq.fetch_add(1, Ordering::AcqRel);
                        self.cancel_on_drop.cancel_active();
                        let previous = report.clone();
                        super::chosen_ffmpeg::apply_missing_chosen_report(&mut report, &chosen);
                        *self.report.write().await = Some(report.clone());
                        if !previous.same_capabilities(&report) {
                            self.events.publish(Event::DoctorUpdated {
                                report: report.clone(),
                            });
                        }
                        return report;
                    }
                    (None, Some(_)) => {}
                }
            }
        }
        self.scan_current(true).await
    }

    pub(crate) async fn rescan(&self) -> DoctorReport {
        self.scan_seq.fetch_add(1, Ordering::AcqRel);
        self.cancel_on_drop.cancel_active();
        let _scan = self.scan_gate.lock().await;
        self.scan_current(false).await
    }

    /// The gate serializes startup, explicit refreshes and missing-tool
    /// recovery. Capture only invalidations known before this scan: a newer
    /// refusal stays dirty for its queued scan/read instead of making this
    /// request repeat probes while other operations keep reporting failures.
    async fn scan_current(&self, allow_deferred: bool) -> DoctorReport {
        let generation = self.invalidated.load(Ordering::Acquire);
        let my_seq = self.scan_seq.fetch_add(1, Ordering::AcqRel) + 1;
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.cancel_on_drop.swap(Some(cancel_flag.clone()));
        let addr = self.addr.read().await.clone();
        let (report, deferred) = tokio::task::spawn_blocking(move || {
            if allow_deferred {
                super::scan_fast(addr)
            } else {
                (super::scan(addr), None)
            }
        })
        .await
        .unwrap_or_else(|_| (super::scan_minimal(), None));
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
        if let Some(deferred) = deferred {
            if !cancel_flag.load(Ordering::Acquire)
                && self.background_enrichment.load(Ordering::Acquire)
            {
                self.spawn_deferred_enrichment(deferred, generation, my_seq, cancel_flag);
            }
        }
        report
    }

    fn spawn_deferred_enrichment(
        &self,
        deferred: DeferredWork,
        generation: u64,
        my_seq: u64,
        cancel_flag: Arc<AtomicBool>,
    ) {
        let Some(runtime) = self
            .runtime
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
        else {
            return;
        };
        let cache = self.clone();
        runtime.spawn(async move {
            let cancel_for_blocking = cancel_flag.clone();
            let enriched =
                tokio::task::spawn_blocking(move || deferred.execute(cancel_for_blocking))
                    .await
                    .ok()
                    .flatten();
            let Some(result) = enriched else {
                return;
            };
            let _scan = cache.scan_gate.lock().await;
            if cancel_flag.load(Ordering::Acquire)
                || cache.scan_seq.load(Ordering::Acquire) != my_seq
                || cache.invalidated.load(Ordering::Acquire) != generation
            {
                return;
            }
            let next = {
                let current = cache.report.read().await;
                let Some(prev) = current.as_ref() else {
                    return;
                };
                let updated = apply_deferred_result(prev.clone(), result);
                if !should_emit_deferred_update(prev, &updated) {
                    return;
                }
                updated
            };
            *cache.report.write().await = Some(next.clone());
            cache.events.publish(Event::DoctorUpdated { report: next });
        });
    }
}
