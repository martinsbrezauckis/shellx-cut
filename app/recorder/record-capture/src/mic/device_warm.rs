//! Bounded microphone warm-up, isolated from durable microphone capture.

use super::device::{wait_for_thread, MIC_ADMISSION, MIC_CAPTURE_ACTIVE, MIC_WARM_ACTIVE};
use super::device_run::{run_device, DeviceCaptureRequest};
use super::wav_layout::MicrophoneWavLayout;
use record_core::{error_codes, RecordError};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const MIC_JOIN_GRACE: Duration = Duration::from_millis(750);
static MIC_WARM_SEQ: AtomicU64 = AtomicU64::new(0);

pub(crate) struct WarmResult {
    pub live: bool,
    pub peak_dbfs: Option<i16>,
}

pub(crate) fn warm_device(input_device: Option<cpal::Device>, max_ms: u64) -> WarmResult {
    let _admission = MIC_ADMISSION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if MIC_WARM_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return WarmResult {
            live: false,
            peak_dbfs: None,
        };
    }
    if MIC_CAPTURE_ACTIVE.load(Ordering::Acquire) {
        MIC_WARM_ACTIVE.store(false, Ordering::Release);
        return WarmResult {
            live: false,
            peak_dbfs: None,
        };
    }
    drop(_admission);

    let stop = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicU32::new(0));
    let seq = MIC_WARM_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!(
        "shellx_mic_warm_{}_{}.wav",
        std::process::id(),
        seq
    ));
    let worker_tmp = tmp.clone();
    let worker_stop = stop.clone();
    let worker_ready = ready.clone();
    let worker_peak = peak.clone();
    let handle = thread::spawn(move || {
        let device = input_device.ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "no input device",
                "no microphone found",
            )
        })?;
        run_device(DeviceCaptureRequest {
            path: worker_tmp.to_string_lossy().into_owned(),
            device,
            stop: worker_stop,
            ready: worker_ready,
            capture_started: Instant::now(),
            meter_peak: Some(worker_peak),
            level: None,
            recording_gate: None,
            wav_layout: MicrophoneWavLayout::PaddedToCaptureClock,
        })
    });
    let started = Instant::now();
    let budget = Duration::from_millis(max_ms.max(200));
    while !handle.is_finished() {
        let Some(remaining) = budget.checked_sub(started.elapsed()) else {
            break;
        };
        thread::sleep(remaining.min(Duration::from_millis(20)));
    }
    let live = ready.load(Ordering::Relaxed);
    stop.store(true, Ordering::Relaxed);
    if wait_for_thread(&handle, MIC_JOIN_GRACE) {
        let _ = handle.join();
        let _ = std::fs::remove_file(&tmp);
        MIC_WARM_ACTIVE.store(false, Ordering::Release);
    } else {
        // Joining a cpal worker is itself unbounded on some Windows drivers.
        // Reap and clean it asynchronously if it ever returns, while the API
        // answers on time and the single-flight guard prevents another warm.
        let _ = thread::Builder::new()
            .name("cut-mic-warm-reaper".into())
            .spawn(move || {
                let _ = handle.join();
                let _ = std::fs::remove_file(&tmp);
                MIC_WARM_ACTIVE.store(false, Ordering::Release);
            });
    }
    WarmResult {
        live,
        peak_dbfs: live
            .then(|| peak_dbfs(peak.load(Ordering::Relaxed)))
            .flatten(),
    }
}

/// Convert an actual integer sample peak into a restrained, display-safe dBFS
/// reading. Zero is absence of a measurable signal, not a made-up floor.
pub(super) fn peak_dbfs(peak: u32) -> Option<i16> {
    if peak == 0 {
        return None;
    }
    let ratio = (peak as f32 / i16::MAX as f32).min(1.0);
    Some((20.0 * ratio.log10()).round().clamp(-96.0, 0.0) as i16)
}
