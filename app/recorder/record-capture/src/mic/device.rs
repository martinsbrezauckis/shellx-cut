use super::stream::run_stream;
use cpal::traits::DeviceTrait;
use record_core::{error_codes, RecordError, Result};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MIC_JOIN_GRACE: Duration = Duration::from_millis(750);
static MIC_WARM_ACTIVE: AtomicBool = AtomicBool::new(false);
static MIC_WARM_SEQ: AtomicU64 = AtomicU64::new(0);

pub(crate) struct CapturedMicrophone {
    pub path: Option<String>,
    pub microphone_lost: bool,
}

pub(crate) struct WarmResult {
    pub live: bool,
    pub peak_dbfs: Option<i16>,
}

pub(super) struct MicStreamControl<'a> {
    pub(super) stop: &'a Arc<AtomicBool>,
    pub(super) ready: &'a Arc<AtomicBool>,
    pub(super) capture_started: Instant,
    pub(super) meter_peak: Option<Arc<AtomicU32>>,
}

fn wait_for_thread<T>(handle: &JoinHandle<T>, max_wait: Duration) -> bool {
    let started = std::time::Instant::now();
    while !handle.is_finished() && started.elapsed() < max_wait {
        thread::sleep(Duration::from_millis(20));
    }
    handle.is_finished()
}

pub(crate) fn join_bounded<T>(
    handle: JoinHandle<Result<T>>,
    max_wait: Duration,
) -> Option<Result<T>> {
    if wait_for_thread(&handle, max_wait) {
        handle.join().ok()
    } else {
        None
    }
}

pub(crate) fn spawn_device_mic(
    path: String,
    device: cpal::Device,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
) -> JoinHandle<Result<CapturedMicrophone>> {
    if MIC_WARM_ACTIVE.load(Ordering::Acquire) {
        return thread::spawn(|| {
            Err(RecordError::new(
                error_codes::CAPTURE,
                "microphone warm-up is still stopping",
                "the native microphone driver did not finish its bounded warm-up",
            )
            .with_action("retry after the microphone test finishes"))
        });
    }
    thread::spawn(move || run_device(path, device, stop, ready, capture_started, None))
}

pub(crate) fn warm_device(input_device: Option<cpal::Device>, max_ms: u64) -> WarmResult {
    if MIC_WARM_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return WarmResult {
            live: false,
            peak_dbfs: None,
        };
    }

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
        run_device(
            worker_tmp.to_string_lossy().into_owned(),
            device,
            worker_stop,
            worker_ready,
            Instant::now(),
            Some(worker_peak),
        )
    });
    let started = std::time::Instant::now();
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

fn run_device(
    path: String,
    device: cpal::Device,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
    meter_peak: Option<Arc<AtomicU32>>,
) -> Result<CapturedMicrophone> {
    let supported = device
        .default_input_config()
        .map_err(|e| RecordError::new(error_codes::CAPTURE, "mic config", e.to_string()))?;
    let end = run_stream(
        &device,
        supported,
        &path,
        None,
        MicStreamControl {
            stop: &stop,
            ready: &ready,
            capture_started,
            meter_peak,
        },
    )?;
    Ok(CapturedMicrophone {
        path: end.samples_written.then_some(path),
        microphone_lost: end.microphone_lost,
    })
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
