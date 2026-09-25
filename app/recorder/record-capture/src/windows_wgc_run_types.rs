//! Private WGC run facts shared by the normal checkpoint owner and pause pilot.

#![cfg_attr(not(all(windows, feature = "capture-windows")), allow(dead_code))]

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use record_core::{error_codes, RecordError, Result, Settings};
use record_recovery::Checkpoint;

/// The exact desktop rectangle accepted for a monitor WGC capture. Window WGC
/// runs intentionally have no stable range because their geometry can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WgcCaptureRange {
    pub(crate) origin_x: i32,
    pub(crate) origin_y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl WgcCaptureRange {
    pub(crate) fn new(origin_x: i32, origin_y: i32, width: u32, height: u32) -> Result<Self> {
        (width > 0 && height > 0)
            .then_some(Self {
                origin_x,
                origin_y,
                width,
                height,
            })
            .ok_or_else(|| invalid("WGC capture range dimensions must be non-zero"))
    }
}

/// Encoder settings and, for monitor capture, its accepted native rectangle.
/// These facts are constructed beside the successful WGC start, not from a
/// server request or later projection setting.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WgcAcceptedCapture {
    pub(crate) settings: Settings,
    pub(crate) range: Option<WgcCaptureRange>,
}

impl WgcAcceptedCapture {
    pub(crate) fn new(settings: Settings, range: Option<WgcCaptureRange>) -> Result<Self> {
        if settings.width == 0 || settings.height == 0 {
            return Err(invalid("accepted WGC settings dimensions must be non-zero"));
        }
        if !settings.fps.is_finite() || !(1.0..=240.0).contains(&settings.fps) {
            return Err(invalid(
                "accepted WGC settings FPS is outside the native range",
            ));
        }
        if let Some(range) = range {
            if range.width != settings.width || range.height != settings.height {
                return Err(invalid(
                    "accepted WGC settings do not match the native monitor range",
                ));
            }
        }
        Ok(Self { settings, range })
    }
}

/// One post-open native observation. `start_ms` is measured on the recording
/// clock; the paired monotonic and Unix values establish the durable origin
/// without deriving one clock from the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WgcStartObservation {
    pub(crate) start_ms: u64,
    pub(crate) monotonic_at: Instant,
    pub(crate) unix_ms: u64,
}

impl WgcStartObservation {
    #[cfg_attr(not(all(windows, feature = "capture-windows")), allow(dead_code))]
    pub(crate) fn observed_now(start_ms: u64) -> Result<Self> {
        let monotonic_at = Instant::now();
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("system clock predates the Unix epoch"))?
            .as_millis();
        let unix_ms = u64::try_from(unix_ms)
            .map_err(|_| invalid("Unix timestamp does not fit in milliseconds"))?;
        Ok(Self {
            start_ms,
            monotonic_at,
            unix_ms,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(start_ms: u64, monotonic_at: Instant, unix_ms: u64) -> Self {
        Self {
            start_ms,
            monotonic_at,
            unix_ms,
        }
    }
}

/// A successfully started native WGC control and the exact capture facts it
/// accepted. The control cannot be retained without those facts.
pub(crate) struct WgcStartedControl<C> {
    pub(crate) control: C,
    pub(crate) accepted: WgcAcceptedCapture,
}

impl<C> WgcStartedControl<C> {
    pub(crate) fn new(control: C, accepted: WgcAcceptedCapture) -> Self {
        Self { control, accepted }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScreenRunIdentity {
    pub(crate) physical_generation: u64,
    /// Checkpoint media boundary. Ordinary WGC may encode during native start,
    /// before the distinct post-open `started` observation is sampled.
    pub(crate) start_ms: u64,
    pub(crate) started: WgcStartObservation,
    pub(crate) accepted: WgcAcceptedCapture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreenRunBoundary {
    pub(crate) physical_generation: u64,
    pub(crate) start_ms: u64,
    pub(crate) end_ms: u64,
    pub(crate) event_offset_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SealedScreenRun {
    pub(crate) boundary: ScreenRunBoundary,
    pub(crate) accepted: WgcAcceptedCapture,
    pub(crate) checkpoint: Checkpoint,
}

fn invalid(detail: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, "invalid WGC run fact", detail)
        .with_action("stop the recording and start a fresh capture")
}
