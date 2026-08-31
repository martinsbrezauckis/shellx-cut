use super::device_run::{run_device, DeviceCaptureRequest};
use super::wav_layout::MicrophoneWavLayout;
use record_core::{error_codes, RecordError, Result};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MIC_JOIN_GRACE: Duration = Duration::from_millis(750);
static MIC_WARM_ACTIVE: AtomicBool = AtomicBool::new(false);
static MIC_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
static MIC_WARM_SEQ: AtomicU64 = AtomicU64::new(0);
static MIC_ADMISSION: Mutex<()> = Mutex::new(());

/// One process-local ownership claim for the native microphone stream.
///
/// A voiceover and a screen-record microphone sidecar must never race each
/// other for the same CPAL input. The claim is intentionally held by the
/// worker, rather than its caller, through native finalization.
pub(crate) struct MicrophoneCaptureReservation(());

impl Drop for MicrophoneCaptureReservation {
    fn drop(&mut self) {
        MIC_CAPTURE_ACTIVE.store(false, Ordering::Release);
    }
}

pub(crate) fn reserve_microphone_capture() -> Result<MicrophoneCaptureReservation> {
    let _admission = MIC_ADMISSION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if MIC_WARM_ACTIVE.load(Ordering::Acquire) {
        return Err(RecordError::new(
            error_codes::CAPTURE,
            "microphone warm-up is still stopping",
            "the native microphone driver is reserved by the bounded microphone test",
        )
        .with_action("retry after the microphone test finishes"));
    }
    MIC_CAPTURE_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| {
            RecordError::new(
                error_codes::CAPTURE,
                "microphone is already in use",
                "another Cut capture still owns the native microphone stream",
            )
            .with_action("stop the active recording and wait for it to finish")
        })?;
    Ok(MicrophoneCaptureReservation(()))
}

pub(crate) struct CapturedMicrophone {
    pub path: Option<String>,
    pub microphone_lost: bool,
    /// Native callback readiness, measured from the caller's capture origin.
    #[cfg_attr(
        not(all(windows, feature = "capture-windows")),
        allow(
            dead_code,
            reason = "consumed by the Windows pause-sidecar native path"
        )
    )]
    pub first_packet_offset_ms: Option<u64>,
}

/// A one-way recording-clock arm for a microphone stream that must prove the
/// device is live before it starts retaining samples.  The stream still opens
/// only after an explicit capture intent; this gate merely prevents pre-roll
/// callbacks from becoming part of the durable WAV.
#[derive(Debug, Default)]
pub(crate) struct MicRecordingGate(OnceLock<Instant>);

impl MicRecordingGate {
    pub(crate) fn arm(&self) -> Result<Instant> {
        let origin = Instant::now();
        self.0.set(origin).map_err(|_| {
            RecordError::new(
                error_codes::GUARDRAIL,
                "microphone recording has already started",
                "the recording clock is one-way and cannot be restarted inside one take",
            )
        })?;
        Ok(origin)
    }

    pub(crate) fn origin(&self) -> Option<Instant> {
        self.0.get().copied()
    }
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
    pub(super) recording_gate: Option<Arc<MicRecordingGate>>,
    pub(super) wav_layout: MicrophoneWavLayout,
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
    match reserve_microphone_capture() {
        Ok(reservation) => spawn_device_mic_reserved(
            path,
            device,
            stop,
            ready,
            capture_started,
            reservation,
            None,
        ),
        Err(error) => thread::spawn(move || Err(error)),
    }
}

pub(crate) fn spawn_device_mic_reserved(
    path: String,
    device: cpal::Device,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
    reservation: MicrophoneCaptureReservation,
    recording_gate: Option<Arc<MicRecordingGate>>,
) -> JoinHandle<Result<CapturedMicrophone>> {
    spawn_device_mic_reserved_with_layout(
        path,
        device,
        stop,
        ready,
        capture_started,
        reservation,
        recording_gate,
        MicrophoneWavLayout::PaddedToCaptureClock,
    )
}

#[cfg_attr(
    not(all(target_os = "macos", feature = "capture-macos")),
    allow(dead_code, reason = "private macOS pause-sidecar capture path")
)]
pub(crate) fn spawn_device_mic_reserved_unpadded(
    path: String,
    device: cpal::Device,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
    reservation: MicrophoneCaptureReservation,
    recording_gate: Option<Arc<MicRecordingGate>>,
) -> JoinHandle<Result<CapturedMicrophone>> {
    spawn_device_mic_reserved_with_layout(
        path,
        device,
        stop,
        ready,
        capture_started,
        reservation,
        recording_gate,
        MicrophoneWavLayout::PacketStart,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_device_mic_reserved_with_layout(
    path: String,
    device: cpal::Device,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
    reservation: MicrophoneCaptureReservation,
    recording_gate: Option<Arc<MicRecordingGate>>,
    wav_layout: MicrophoneWavLayout,
) -> JoinHandle<Result<CapturedMicrophone>> {
    thread::spawn(move || {
        let _reservation = reservation;
        run_device(DeviceCaptureRequest {
            path,
            device,
            stop,
            ready,
            capture_started,
            meter_peak: None,
            recording_gate,
            wav_layout,
        })
    })
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
            recording_gate: None,
            wav_layout: MicrophoneWavLayout::PaddedToCaptureClock,
        })
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

/// Convert an actual integer sample peak into a restrained, display-safe dBFS
/// reading. Zero is absence of a measurable signal, not a made-up floor.
pub(super) fn peak_dbfs(peak: u32) -> Option<i16> {
    if peak == 0 {
        return None;
    }
    let ratio = (peak as f32 / i16::MAX as f32).min(1.0);
    Some((20.0 * ratio.log10()).round().clamp(-96.0, 0.0) as i16)
}
