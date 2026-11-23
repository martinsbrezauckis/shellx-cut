//! Bounded microphone warm-up, isolated from durable microphone capture.

use super::device::{wait_for_thread, MIC_ADMISSION, MIC_CAPTURE_ACTIVE, MIC_WARM_ACTIVE};
use super::device_run::{run_device, DeviceCaptureRequest};
use super::wav_layout::MicrophoneWavLayout;
use record_core::{error_codes, RecordError};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const MIC_JOIN_GRACE: Duration = Duration::from_millis(750);

pub(crate) struct WarmResult {
    pub live: bool,
    pub peak_dbfs: Option<i16>,
    /// Why the microphone could not be opened, when the open itself failed.
    pub problem: Option<crate::MicrophoneProblem>,
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
            problem: None,
        };
    }
    if MIC_CAPTURE_ACTIVE.load(Ordering::Acquire) {
        MIC_WARM_ACTIVE.store(false, Ordering::Release);
        return WarmResult {
            live: false,
            peak_dbfs: None,
            problem: None,
        };
    }
    drop(_admission);

    let stop = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicU32::new(0));
    let mut temp_builder = tempfile::Builder::new();
    temp_builder.prefix("shellx-cut-mic-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp_builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let tmp_dir = match temp_builder.tempdir() {
        Ok(dir) => dir,
        Err(_) => {
            MIC_WARM_ACTIVE.store(false, Ordering::Release);
            return WarmResult {
                live: false,
                peak_dbfs: None,
                problem: Some(crate::MicrophoneProblem::Other),
            };
        }
    };
    let tmp_dir = Arc::new(tmp_dir);
    let tmp = tmp_dir.path().join("warm.wav");
    let worker_dir = tmp_dir.clone();
    let worker_tmp = tmp.clone();
    let worker_stop = stop.clone();
    let worker_ready = ready.clone();
    let worker_peak = peak.clone();
    let handle = thread::spawn(move || {
        // Keep the directory alive even if starting the separate reaper fails.
        let _worker_dir = worker_dir;
        let device = input_device.ok_or_else(|| {
            crate::mic_endpoint::explain_open_error(RecordError::new(
                error_codes::CAPTURE,
                "no input device",
                "no microphone found",
            ))
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
    let mut problem = None;
    stop.store(true, Ordering::Relaxed);
    if wait_for_thread(&handle, MIC_JOIN_GRACE) {
        if let Ok(Err(error)) = handle.join() {
            problem = crate::MicrophoneProblem::from_error(&error);
        }
        drop(tmp_dir);
        MIC_WARM_ACTIVE.store(false, Ordering::Release);
    } else {
        // Joining a cpal worker is itself unbounded on some Windows drivers.
        // Reap and clean it asynchronously if it ever returns, while the API
        // answers on time and the single-flight guard prevents another warm.
        let _ = thread::Builder::new()
            .name("cut-mic-warm-reaper".into())
            .spawn(move || {
                let _ = handle.join();
                drop(tmp_dir);
                MIC_WARM_ACTIVE.store(false, Ordering::Release);
            });
    }
    WarmResult {
        live,
        peak_dbfs: live
            .then(|| peak_dbfs(peak.load(Ordering::Relaxed)))
            .flatten(),
        problem: if live { None } else { problem },
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
