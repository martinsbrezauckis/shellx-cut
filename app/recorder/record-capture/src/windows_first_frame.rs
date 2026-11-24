//! Coordinator delaying Windows CaptureClock/input/mic startup until the
//! first WGC frame arrives so a take starts at the first captured picture.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use record_core::Result;

use crate::windows_wgc_clock_origin::{FirstFramePlacement, QpcCaptureOrigin};
use crate::CaptureClock;

#[cfg(all(windows, feature = "capture-windows"))]
use crate::{input_listener::InputSeedHandle, mic::MicRecordingGate};

type FirstFrameOrigin = (Instant, QpcCaptureOrigin);
type FrameReadyNotify = Arc<(Mutex<Option<FirstFrameOrigin>>, Condvar)>;

#[derive(Clone, Default)]
pub(crate) struct WindowsFirstFrameAnchor {
    clock: Option<CaptureClock>,
    #[cfg(all(windows, feature = "capture-windows"))]
    mic_gate: Option<Arc<MicRecordingGate>>,
    #[cfg(all(windows, feature = "capture-windows"))]
    input_seed: Arc<Mutex<Option<InputSeedHandle>>>,
    input_origin: Arc<OnceLock<Instant>>,
    qpc_origin: Arc<OnceLock<FirstFrameOrigin>>,
    ready_notify: FrameReadyNotify,
}

impl WindowsFirstFrameAnchor {
    pub(crate) fn new(clock: Option<CaptureClock>) -> Self {
        Self {
            clock,
            #[cfg(all(windows, feature = "capture-windows"))]
            mic_gate: None,
            #[cfg(all(windows, feature = "capture-windows"))]
            input_seed: Arc::new(Mutex::new(None)),
            input_origin: Arc::new(OnceLock::new()),
            qpc_origin: Arc::new(OnceLock::new()),
            ready_notify: Arc::new((Mutex::new(None), Condvar::new())),
        }
    }

    #[cfg(all(windows, feature = "capture-windows"))]
    pub(crate) fn with_mic_gate(mut self, mic_gate: Option<Arc<MicRecordingGate>>) -> Self {
        self.mic_gate = mic_gate;
        self
    }

    #[cfg(all(windows, feature = "capture-windows"))]
    pub(crate) fn set_input_seed(&self, seed: InputSeedHandle) {
        *self
            .input_seed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(seed);
    }

    pub(crate) fn input_origin_cell(&self) -> Arc<OnceLock<Instant>> {
        self.input_origin.clone()
    }

    #[cfg_attr(not(all(windows, feature = "capture-windows")), allow(dead_code))]
    pub(crate) fn qpc_origin(&self) -> Option<FirstFrameOrigin> {
        self.qpc_origin.get().copied()
    }

    /// Anchor `CaptureClock`, input, mic, and WGC origin to the first accepted frame.
    pub(crate) fn admit_first_frame_with_sample(
        &self,
        first_native_hns: i64,
        callback_at: Instant,
        callback_qpc: Option<QpcCaptureOrigin>,
    ) -> std::result::Result<(Instant, QpcCaptureOrigin), &'static str> {
        if let Some(existing) = self.qpc_origin.get().copied() {
            return Ok(existing);
        }
        let origin = QpcCaptureOrigin::from_first_frame(first_native_hns)?;
        let origin_instant = callback_qpc
            .and_then(|cb| match cb.place_first_frame(first_native_hns, 0).ok()? {
                FirstFramePlacement::BeforeStart { lead_hns } => {
                    let age = Duration::from_nanos(lead_hns.min(500_000).saturating_mul(100));
                    callback_at.checked_sub(age)
                }
                FirstFramePlacement::AtOrAfterStart { .. } => Some(callback_at),
            })
            .unwrap_or(callback_at);

        let _ = self.input_origin.set(origin_instant);
        #[cfg(all(windows, feature = "capture-windows"))]
        if let Some(seed) = self
            .input_seed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            seed.seed_at_origin();
        }
        #[cfg(all(windows, feature = "capture-windows"))]
        if let Some(gate) = self.mic_gate.as_ref() {
            gate.arm_at(origin_instant);
        }
        if let Some(clock) = self.clock.as_ref() {
            let _ = clock.start_at(origin_instant);
        }
        let pair = (origin_instant, origin);
        let _ = self.qpc_origin.set(pair);
        let (lock, wake) = &*self.ready_notify;
        *lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pair);
        wake.notify_all();
        Ok(self.qpc_origin.get().copied().unwrap_or(pair))
    }

    #[cfg(all(windows, feature = "capture-windows"))]
    pub(crate) fn admit_first_frame(
        &self,
        first_native_hns: i64,
        callback_at: Instant,
    ) -> std::result::Result<(Instant, QpcCaptureOrigin), &'static str> {
        let callback_qpc = QpcCaptureOrigin::sample(callback_at).ok();
        self.admit_first_frame_with_sample(first_native_hns, callback_at, callback_qpc)
    }

    #[cfg_attr(not(all(windows, feature = "capture-windows")), allow(dead_code))]
    pub(crate) fn wait_started(
        &self,
        stop: &AtomicBool,
        mut check_error: impl FnMut() -> Result<()>,
    ) -> Result<Option<(Instant, QpcCaptureOrigin)>> {
        let (lock, wake) = &*self.ready_notify;
        let mut guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(pair) = *guard {
                return Ok(Some(pair));
            }
            check_error()?;
            if stop.load(Ordering::Acquire) {
                return Ok(None);
            }
            let (next, _) = wake
                .wait_timeout(guard, Duration::from_millis(10))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = next;
        }
    }
}

#[cfg(test)]
#[path = "windows_first_frame_tests.rs"]
mod tests;
