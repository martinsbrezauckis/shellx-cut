//! Platform system-audio capture routed through the shared recorder clock.

use crate::dispatch::parse_args;
#[cfg(all(not(windows), not(target_os = "linux")))]
use cut_core::error_codes;
use cut_core::{CutError, VerbResult};
use record_core::RecordError;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::AtomicBool;
#[cfg(all(not(windows), not(target_os = "linux")))]
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A driver-side WASAPI shutdown must not hold the recording coordinator hostage.
/// The worker remains detached when this grace period expires and owns its partial
/// artifact cleanup until it eventually returns.
pub(crate) const SYSTEM_AUDIO_FINALIZE_GRACE: Duration = Duration::from_secs(2);
/// A continuous audio capture is explicitly stopped by the recording lifecycle,
/// but still has an operation ceiling so a lost stop signal cannot retain ffmpeg
/// forever.
#[cfg(all(not(windows), not(target_os = "linux")))]
const SYSTEM_AUDIO_MAX_RUNTIME: Duration = Duration::from_secs(24 * 60 * 60);
const PROBE_WORKER_TIMEOUT: Duration = Duration::from_secs(17);

pub(crate) fn reserve(enabled: bool) -> Result<Option<record_capture::SystemAudioLease>, CutError> {
    enabled
        .then(record_capture::reserve_system_audio)
        .transpose()
        .map_err(super::record_err)
}

/// Explicit short audio-delivery test. Unlike passive Doctor, this user action
/// may open an OS permission prompt, but it creates no project or screen stream.
pub(crate) async fn probe_handler(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        max_ms: Option<u64>,
    }

    let args: Args = parse_args(args)?;
    let max_ms = args.max_ms.unwrap_or(record_capture::DEFAULT_WINDOW_MS);
    let worker = tokio::task::spawn_blocking(move || record_capture::probe_system_audio(max_ms));
    let probe = match tokio::time::timeout(PROBE_WORKER_TIMEOUT, worker).await {
        Ok(Ok(Ok(probe))) => probe,
        Ok(Ok(Err(error))) => return Err(super::record_err(error)),
        Ok(Err(error)) => {
            return Err(CutError::new(
                cut_core::error_codes::JOB_FAILED,
                "system audio test worker failed",
                error.to_string(),
            ));
        }
        Err(_) => {
            return Err(CutError::new(
                cut_core::error_codes::SIDECAR,
                "system audio test timed out",
                "the bounded native audio worker did not return within 17 seconds",
            )
            .with_suggested_action(
                "check the OS audio-capture permission, restart Cut if permission was just granted, then test again",
            ));
        }
    };
    Ok(VerbResult::ok(json!(probe)))
}

/// Poll a system-audio worker without consuming it so a caller can join only after
/// completion is already known. This avoids an unbounded `JoinHandle::join` on a
/// driver call such as WASAPI `Stop` or COM release.
pub(crate) fn worker_finished_within(worker: &JoinHandle<()>, grace: Duration) -> bool {
    let deadline = Instant::now() + grace;
    while !worker.is_finished() {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
    true
}

/// Complete the optional system-audio sidecar without an unbounded native join.
/// A timed-out worker owns its cleanup, while the caller refuses to mark the
/// recording Complete or publish a normal project that would imply its audio
/// sidecar is final.
pub(crate) fn finalize_worker(
    worker: Option<JoinHandle<()>>,
    log_path: &Path,
) -> Result<(), RecordError> {
    let Some(worker) = worker else {
        return Ok(());
    };
    if worker_finished_within(&worker, SYSTEM_AUDIO_FINALIZE_GRACE) {
        let _ = worker.join();
        return Ok(());
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        use std::io::Write;
        let _ = writeln!(
            file,
            "system-audio finalization exceeded {}ms; capture is not marked Complete",
            SYSTEM_AUDIO_FINALIZE_GRACE.as_millis(),
        );
    }
    Err(RecordError::new(
        "capture",
        "system-audio finalization exceeded its bounded grace period",
        "the capture is not marked Complete while a native sidecar may still be open",
    )
    .with_action(
        "retry after the system-audio device returns; finalized checkpoints remain recoverable",
    ))
}

pub(crate) fn capture_system_audio_until(
    out: &Path,
    duration_ms: Option<u64>,
    stop: Arc<AtomicBool>,
    capture_started: Instant,
) -> Result<Option<record_capture::SystemAudioCapture>, CutError> {
    #[cfg(windows)]
    {
        #[allow(clippy::needless_return)]
        return record_capture::capture_system_loopback(
            &out.to_string_lossy(),
            duration_ms,
            stop,
            capture_started,
        )
        .map(Some)
        .map_err(super::record_err);
    }
    #[cfg(target_os = "linux")]
    {
        #[allow(clippy::needless_return)]
        return record_capture::capture_system_pipewire(out, duration_ms, stop, capture_started)
            .map(Some)
            .map_err(super::record_err);
    }
    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        let _ = capture_started;
        if let Some(ms) = duration_ms {
            super::capture_system_audio(out, ms)?;
            return Ok(None);
        }
        super::align_ffmpeg_env();
        let (fmt, input) = super::system_audio_source();
        let mut command = std::process::Command::new(cut_media::toolpath::ffmpeg());
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                fmt,
                "-i",
                input,
                "-ac",
                "2",
                "-ar",
                "48000",
                "-y",
            ])
            .arg(out);
        let stop_for_owner = stop.clone();
        let control =
            cut_media::ffmpeg::OwnedProcessControl::bounded(SYSTEM_AUDIO_MAX_RUNTIME, move || {
                stop_for_owner.load(Ordering::Relaxed)
            });
        let result = cut_media::ffmpeg::run_owned_command(
            &mut command,
            &control,
            "capture screen-record system audio",
        );
        let wrote = out
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);
        if stop.load(Ordering::Relaxed) && wrote {
            return Ok(None);
        }
        let status = result.map_err(|error| {
            CutError::new(
                error_codes::IO,
                "system-audio capture process stopped unexpectedly",
                error.to_string(),
            )
        })?;
        if !status.status.success() && !wrote {
            return Err(CutError::new(
                error_codes::IO,
                "system-audio capture failed",
                "no desktop-audio loopback is available on this OS; verify the platform capture permission and audio device",
            ));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{finalize_worker, worker_finished_within, PROBE_WORKER_TIMEOUT};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn probe_timeout_covers_activation_plus_the_largest_window() {
        assert_eq!(PROBE_WORKER_TIMEOUT.as_secs(), 17);
        assert!(PROBE_WORKER_TIMEOUT.as_millis() > 10_000 + 5_000);
    }

    #[test]
    fn bounded_worker_wait_does_not_block_a_stalled_native_shutdown() {
        let release = Arc::new(AtomicBool::new(false));
        let worker_release = release.clone();
        let worker = std::thread::spawn(move || {
            while !worker_release.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
        });

        assert!(!worker_finished_within(&worker, Duration::from_millis(5)));
        release.store(true, Ordering::Relaxed);
        assert!(worker_finished_within(&worker, Duration::from_secs(1)));
        worker.join().unwrap();
    }

    #[test]
    fn incomplete_worker_refuses_a_complete_receipt() {
        let release = Arc::new(AtomicBool::new(false));
        let worker_release = release.clone();
        let worker = std::thread::spawn(move || {
            while !worker_release.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let temp = tempfile::tempdir().unwrap();
        assert!(finalize_worker(Some(worker), &temp.path().join("record.log")).is_err());
        assert!(temp.path().join("record.log").is_file());
        release.store(true, Ordering::Relaxed);
    }
}
