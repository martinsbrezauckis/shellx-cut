use super::audio_level::RollingAudioLevel;
use super::device_run::{run_device, DeviceCaptureRequest};
use super::wav_layout::MicrophoneWavLayout;
use record_core::{error_codes, RecordError, Result};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(super) static MIC_WARM_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(super) static MIC_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(super) static MIC_ADMISSION: Mutex<()> = Mutex::new(());

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

pub(super) struct MicStreamControl<'a> {
    pub(super) stop: &'a Arc<AtomicBool>,
    pub(super) ready: &'a Arc<AtomicBool>,
    pub(super) capture_started: Instant,
    pub(super) meter_peak: Option<Arc<AtomicU32>>,
    pub(super) level: Option<Arc<RollingAudioLevel>>,
    pub(super) recording_gate: Option<Arc<MicRecordingGate>>,
    pub(super) wav_layout: MicrophoneWavLayout,
}

pub(super) fn wait_for_thread<T>(handle: &JoinHandle<T>, max_wait: Duration) -> bool {
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
    spawn_device_mic_with_optional_level(path, device, stop, ready, capture_started, None)
}

/// Start an already-selected microphone stream and feed its existing callback
/// into a caller-owned live meter. This is intentionally an explicit handle:
/// the owner controls its lifecycle instead of consulting process-global audio
/// state, and no additional device or CPAL stream is opened.
pub(crate) fn spawn_device_mic_with_level(
    path: String,
    device: cpal::Device,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
    level: Arc<RollingAudioLevel>,
) -> JoinHandle<Result<CapturedMicrophone>> {
    spawn_device_mic_with_optional_level(path, device, stop, ready, capture_started, Some(level))
}

fn spawn_device_mic_with_optional_level(
    path: String,
    device: cpal::Device,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
    level: Option<Arc<RollingAudioLevel>>,
) -> JoinHandle<Result<CapturedMicrophone>> {
    match reserve_microphone_capture() {
        Ok(reservation) => spawn_device_mic_reserved_with_level(
            path,
            device,
            stop,
            ready,
            capture_started,
            reservation,
            None,
            level,
        ),
        Err(error) => {
            if let Some(level) = level.as_deref() {
                level.mark_stopped();
            }
            thread::spawn(move || Err(error))
        }
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
    spawn_device_mic_reserved_with_level(
        path,
        device,
        stop,
        ready,
        capture_started,
        reservation,
        recording_gate,
        None,
    )
}

/// Reserved microphone variant with a caller-owned live level model. The
/// reservation and the level share the same single CPAL callback.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_device_mic_reserved_with_level(
    path: String,
    device: cpal::Device,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
    reservation: MicrophoneCaptureReservation,
    recording_gate: Option<Arc<MicRecordingGate>>,
    level: Option<Arc<RollingAudioLevel>>,
) -> JoinHandle<Result<CapturedMicrophone>> {
    spawn_device_mic_reserved_with_layout(
        path,
        device,
        stop,
        ready,
        capture_started,
        reservation,
        recording_gate,
        level,
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
        None,
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
    level: Option<Arc<RollingAudioLevel>>,
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
            level,
            recording_gate,
            wav_layout,
        })
    })
}
