//! Disposable native rehearsal take entry points.
//!
//! This module admits one short source check and delegates process-local
//! capability ownership/retention to `owner`. It intentionally never enters
//! project, recovery, RecordingProject, Studio, or timeline code.

use super::{
    align_ffmpeg_env, doctor, monitor_start_admission, record_err, start_readiness,
    CaptureSessionControl,
};
use crate::dispatch::parse_args;
use cut_core::{error_codes, CutError, VerbResult};
use serde_json::{json, Value};
use std::path::PathBuf;

mod owner;
#[cfg(test)]
mod tests;

pub(crate) const MIN_DURATION_MS: u64 = 3_000;
pub(crate) const MAX_DURATION_MS: u64 = 5_000;
const DEFAULT_DURATION_MS: u64 = MIN_DURATION_MS;

fn bounded_duration(duration_ms: Option<u64>) -> Result<u64, CutError> {
    let duration_ms = duration_ms.unwrap_or(DEFAULT_DURATION_MS);
    if !(MIN_DURATION_MS..=MAX_DURATION_MS).contains(&duration_ms) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "rehearsal duration must be between {MIN_DURATION_MS} and {MAX_DURATION_MS} ms"
            ),
            format!("received duration_ms={duration_ms}"),
        )
        .with_suggested_action("choose a 3- to 5-second rehearsal take"));
    }
    Ok(duration_ms)
}

/// Discard temporary rehearsal bytes before ordinary `screen_record.start`
/// creates a project-local capture/recovery owner.
pub(crate) fn discard_for_recording() -> Result<(), CutError> {
    owner::discard_for_recording()
}

/// Start and wait for a capped native source rehearsal. The worker owns its
/// TempDir and completion guard after admission, so dropping this async caller
/// cannot delete bytes below a live backend or leave the process owner Running.
pub(crate) async fn start(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        duration_ms: Option<u64>,
        fps: Option<f64>,
        monitor: Option<u32>,
        monitor_id: Option<String>,
        window: Option<String>,
    }

    let args: Args = parse_args(args)?;
    let duration_ms = bounded_duration(args.duration_ms)?;
    let fps = args.fps.unwrap_or(30.0);
    super::validate_capture_settings(Some(duration_ms), fps)?;
    let recorder_doctor = doctor();
    start_readiness::ensure_start_ready(&recorder_doctor.cards)?;
    let target = monitor_start_admission::admit(
        args.monitor,
        args.monitor_id.as_deref(),
        args.window.is_some(),
        &recorder_doctor.monitors,
    )?;
    let root = owner::owned_root()?;
    let handle = owner::mint_handle()?;
    let capture_id = super::new_capture_id();
    let control = CaptureSessionControl::new(Some(duration_ms), false, false, false);
    let reservation = reserve_rehearsal_capture(
        capture_id.clone(),
        control.clone(),
        &handle,
        super::source_preview::release_for_recording,
    )?;

    let output_dir = root.path().to_path_buf();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // This OS worker is deliberately detached from the request future. Unlike
    // a request-owned async task, it cannot be cancelled by dropping the verb
    // future: it owns the TempDir and `RunningTakeGuard` until native capture
    // has ended and the owner reaches Ready, Idle, or Revoked.
    let worker = std::thread::Builder::new()
        .name("cut-rehearsal-capture".into())
        .spawn({
            let worker_handle = handle.clone();
            move || {
                let _owner_guard = owner::RunningTakeGuard::new(worker_handle.clone());
                let capture_result = capture_once(
                    duration_ms,
                    fps,
                    target.legacy_index,
                    target.exact_id,
                    args.window,
                    output_dir,
                    control,
                    reservation,
                );
                let result = owner::finish_take(&worker_handle, root, capture_result);
                let _ = sender.send(result);
            }
        });
    if let Err(error) = worker {
        owner::abandon_take(&handle);
        return Err(CutError::new(
            error_codes::IO,
            "could not start the detached rehearsal worker",
            error.to_string(),
        ));
    }

    receiver.await.map_err(|_| {
        CutError::new(
            error_codes::IO,
            "the rehearsal worker ended before reporting its outcome",
            "the owned worker panicked or the recorder runtime stopped",
        )
    })??;
    Ok(VerbResult::ok(json!({
        "playback_handle": handle,
        "playback_url": playback_url(&handle),
        "duration_ms": duration_ms,
        "discard_on_start_recording": true,
        "note": "This disposable rehearsal is not a project recording and expires after 90 seconds, or sooner when you close it or start recording.",
    })))
}

/// Admit the rehearsal owner while the capture handoff reservation is held,
/// before releasing any preview. A rejected owner transition (for example, a
/// locked prior rehearsal root) therefore leaves an existing preview intact.
/// If preview teardown itself fails after the owner transition, clear the
/// provisional Running state before the reservation drops.
fn reserve_rehearsal_capture<F>(
    capture_id: String,
    control: CaptureSessionControl,
    handle: &str,
    release_preview: F,
) -> Result<super::capture_registry::CaptureReservation, CutError>
where
    F: FnOnce() -> Result<(), CutError>,
{
    super::capture_registry::reserve_capture_after_preview_release(
        capture_id,
        control.clone(),
        None,
        || {
            owner::begin_take(handle.to_owned(), control)?;
            if let Err(error) = release_preview() {
                owner::abandon_take(handle);
                return Err(error);
            }
            Ok(())
        },
    )
}

fn capture_once(
    duration_ms: u64,
    fps: f64,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    window: Option<String>,
    out_dir: PathBuf,
    control: CaptureSessionControl,
    reservation: super::capture_registry::CaptureReservation,
) -> Result<String, CutError> {
    align_ffmpeg_env();
    let _terminal_guard = control.terminal_guard();
    if !control.claim_native_launch() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the rehearsal was stopped before native capture began",
            "the disposable capture no longer owns the native launch boundary",
        ));
    }
    let capture = record_capture::live_capture().ok_or_else(|| {
        CutError::new(
            error_codes::UNIMPLEMENTED,
            "native rehearsal is unavailable in this build",
            "no live screen-capture backend was compiled for this platform",
        )
        .with_suggested_action("use a desktop Cut build with native screen capture enabled")
    })?;
    let config = record_capture::CaptureConfig {
        duration_ms: Some(duration_ms),
        fps,
        quality: None,
        capture_cursor: false,
        monitor,
        monitor_id,
        region: None,
        window,
        // Existing explicit microphone/system-audio checks own audio admission.
        // This immediate playback take proves the selected native VIDEO source.
        audio: false,
        microphone_source: record_capture::MicrophoneSource::SystemDefault,
        microphone_level: None,
        system_audio: false,
        capture_keys: false,
        out_dir: out_dir.to_string_lossy().into_owned(),
        checkpoint: None,
        clock: None,
        readiness: None,
        controller_placement: None,
        source_lifecycle: None,
        active_preview: None,
    };
    let result = capture
        .capture(&config, control.stop_signal())
        .map_err(record_err)
        .map(|output| output.source_video);
    let terminal = control.terminalize();
    drop(reservation);
    terminal.map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not close the rehearsal capture",
            error.to_string(),
        )
    })?;
    result
}

fn playback_url(handle: &str) -> String {
    format!("/api/recording-rehearsal/{handle}")
}

/// Resolve only an opaque server-issued capability for the HTTP route.
pub(crate) fn playback_media(handle: &str) -> Option<PathBuf> {
    owner::playback_media(handle)
}

/// Idempotent unmount/manual cleanup. It cannot name project media or a path.
pub(crate) fn discard(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        handle: Option<String>,
    }
    let args: Args = parse_args(args)?;
    let (handle, discarded, state) = owner::discard(args.handle)?;
    Ok(VerbResult::ok(json!({
        "handle": handle,
        "discarded": discarded,
        "state": state,
    })))
}

#[cfg(test)]
pub(crate) fn rehearsal_test_lock() -> &'static tokio::sync::Mutex<()> {
    owner::rehearsal_test_lock()
}

#[cfg(test)]
pub(crate) fn install_test_playback() -> String {
    owner::install_test_playback()
}
