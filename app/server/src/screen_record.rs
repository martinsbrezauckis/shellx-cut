//! screen_record.rs — IN-PROCESS bridge to the integrated Cut recorder crates
//! and replaces the former spawned-CLI sidecar.
//!
//! Role: cutd LINKS the `record-*` crates (record-core / -engine / -render /
//! -capture, same Cargo workspace) and calls their public APIs DIRECTLY — there is
//! no child process, no recorder binary to resolve or stage, no stdout parsing.
//! This follows the matte pattern: the polish
//! pass bakes a content-addressed clip (dispatch.rs orchestrates bake → media.import
//! → edit.insert); every actual record-crate call lives here so the rest of cutd
//! stays recorder-agnostic.
//!
//! What this module exposes (all in-process, no spawn):
//!   - `doctor()`                      → `record_capture::doctor()` capability cards
//!   - `autoedit(track, out)`          → `record_engine::autoedit` (EventTrack→EditPlan)
//!   - `render(source, plan, out, audio?)` → `record_render::render_video_audio`
//!   - `gif(source, out, fps, width)`  → `record_render::ffmpeg::mp4_to_gif`
//!   - `start_capture(...)`            → `record_capture::live_capture()` on a
//!                                       background thread, bounded or explicit-stop
//!
//! LIVE CAPTURE NOTE: the live backend (`capture-{linux,windows,macos}`, enabled
//! per-target in Cargo.toml) needs a real desktop session at RUNTIME (Linux XDG
//! ScreenCast portal / Windows WGC / macOS ScreenCaptureKit) — it can't capture on
//! a headless server. `start_capture` runs the blocking capture on a background
//! thread and finalizes `project.json` at the duration bound or explicit stop.
//! Unlike the old detached sidecar PROCESS, an in-process capture THREAD does NOT
//! survive a cutd restart — acceptable for the bounded model (a 15s capture rarely
//! outlives a restart) and the explicit cost of dropping the separate process.
//!
//! Dependencies: record-core/-engine/-render/-capture, cut_core (CutError). Primary
//! callers: dispatch.rs (`screen_record_doctor`/`_start`/`_autoedit`/`_polish`/`_export`).
use cut_core::{error_codes, CutError, VerbResult};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
mod audio_meters;
#[cfg(test)]
mod audio_meters_tests;
mod autoedit_args;
mod cadence;
mod camera_capture_sidecar;
mod camera_public;
mod capture_artifacts;
// REC-CAMERA-01d is the private Windows server/project owner. It is not
// selected by screen_record.start or any verb/UI/Doctor route until installed
// native proof explicitly admits that separate product surface.
#[cfg(windows)]
#[allow(dead_code)]
mod camera_server_owner;
mod capture_files;
mod capture_registry;
mod capture_session_control;
mod capture_terminal;
mod containment;
mod doctor_projection;
mod export_audio;
mod export_job;
mod export_progress;
pub(crate) mod finalization_budget;
pub(crate) mod microphone;
mod monitor_start_admission;
// The macOS-only Region start path receives a ticket only from the private
// foreground desktop bridge. It still enters the ordinary capture reservation
// below and has no schema/verb/UI representation.
#[cfg(target_os = "macos")]
mod macos_pause_capture;
#[cfg(target_os = "macos")]
mod macos_pause_session_start;
#[cfg(target_os = "macos")]
mod macos_pause_transition;
#[cfg(target_os = "macos")]
pub(crate) mod macos_region_bridge;
#[cfg(target_os = "macos")]
pub(crate) mod macos_region_start;
// The Windows-only Region handoff proves foreground ownership and one-use
// DisplayConfig admission before it enters the ordinary recorder reservation.
// The live WGC backend crops Direct3D frames on-GPU and maps input against the
// same physical subsurface; it has no schema, UI, or public capability
// representation.
#[cfg(windows)]
pub(crate) mod windows_region_bridge;
#[cfg(windows)]
mod windows_region_parent;
#[cfg(windows)]
pub(crate) mod windows_region_start;

/// Synchronously retain and scrub the foreground-desktop handoff before the
/// async runtime exists. Only the host-native private bridge is eligible.
pub(crate) fn initialize_private_foreground_region_bridge(allow_foreground_bridge: bool) {
    #[cfg(target_os = "macos")]
    macos_region_bridge::initialize_from_child_environment(allow_foreground_bridge);
    #[cfg(windows)]
    windows_region_bridge::initialize_from_child_environment(allow_foreground_bridge);
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = allow_foreground_bridge;
}
// Linux has a real portal/PipeWire capture backend, but it does not yet expose
// a pause-safe owner for the portal-selected screen or per-run audio sidecars.
// Keep its private admission fail-closed until those native boundaries can be
// sealed and projected without re-entering portal consent.
#[allow(dead_code)]
mod linux_pause_private_admission;
pub(crate) mod live_controls;
mod polish;
// REC-PAUSE-01 is a pure internal projection contract. It intentionally has no
// verb, filesystem writer, or live-worker caller until the pause-aware capture
// coordinator can produce the journal and sealed run inputs it requires.
#[allow(dead_code)]
mod pause_projection;
// REC-PAUSE-01 strict CFR qualification remains a private pure plan. It has
// no writer, verb, live capture, or public product surface.
#[allow(dead_code)]
mod pause_projection_frame_grid;
// REC-PAUSE-01 execution remains internal. The bounded macOS pause bridge
// reaches it only after the native owner seals the durable journal; Windows
// and Linux have no public pause/resume routing.
#[allow(dead_code)] // Linux builds retain the private Windows-only owner seam.
mod pause_projection_executor;
// REC-PAUSE-01 private session ownership. Windows WGC and macOS
// ScreenCaptureKit share this durable owner; only the bounded macOS bridge
// exposes public controls after its durable transitions.
#[allow(dead_code)] // Linux builds retain the private Windows-only owner seam.
mod pause_session_owner;
#[cfg(test)]
mod pause_session_owner_tests;
// REC-PAUSE-02 is a pure private command/fact protocol. It has no native
// worker registry, journal, public verb, or lifecycle caller in this slice.
#[allow(dead_code)]
mod pause_worker_protocol;
#[cfg(test)]
mod pause_worker_protocol_tests;
// The raw-capture stop path calls this muxer when `mux_raw:true`; it combines
// the sealed video and optional mic/system tracks without inventing an edit.
#[cfg(target_os = "macos")]
mod macos_pause_control;
mod raw_mux;
mod recording_controls;
mod recording_scenes;
pub(crate) mod recovery;
pub(crate) mod rehearsal;
// Region tickets and geometry remain a private admission foundation pending
// compiled/native foreground qualification. Do not expose a coordinate-taking
// verb or an abstract overlay in the meantime.
#[allow(dead_code)]
mod region_selection;
#[cfg(test)]
mod region_selection_tests;
#[allow(dead_code)]
mod region_selection_value;
mod run_seal_coordinator;
#[cfg(test)]
mod run_seal_coordinator_tests;
// Historical private Screen-only receipt seam. Public Recording Scenes uses
// recording_scenes + scene_projection_start instead; retain this module for
// its focused legacy tests without letting it surface a second receipt.
#[cfg(any(windows, target_os = "macos"))]
mod private_pause_capture_projection;
#[allow(dead_code)]
mod scene_live_start;
mod scene_projection_receipt_support;
mod scene_projection_start;
#[cfg(test)]
mod scene_projection_start_tests;
mod screenshot;
mod source_preview;
mod source_preview_handlers;
mod start_handler;
mod start_readiness;
pub(crate) mod system_audio;
pub(crate) mod system_audio_capture;
mod windows_path;
// REC-PAUSE-01 server-only translation. It has no public caller: a later
// native lifecycle hook must supply a verified screen fragment/event factory.
#[allow(dead_code)]
mod windows_pause_adapter;
#[allow(dead_code)]
mod windows_pause_adapter_events;
// REC-PAUSE-01 private owner composition. It is Windows-only in production and
// has injected Linux ordering tests; it remains absent from every public path.
#[allow(dead_code)]
mod windows_pause_evidence;
// Private immutable per-run input evidence shared by native pause owners and
// recovery/projection seams; no verb or project schema names these sidecars.
#[allow(dead_code)]
mod windows_pause_evidence_artifacts;
#[allow(dead_code)]
mod windows_pause_evidence_contract;
#[allow(dead_code)]
mod windows_pause_input_sidecar;
#[allow(dead_code)] // Linux builds retain the private Windows-only owner seam.
mod windows_pause_session;
mod windows_pause_session_types;
// The private Windows admission accepts only exact screen plus owned
// microphone/system-audio streams; every other selected stream is refused.
#[allow(dead_code)] // It is called only by the Windows-private owner.
mod windows_pause_private_admission;
// Windows-only internal capture owner. It uses the ordinary reservation, Stop,
// project, and recovery path, but remains absent from public start routing.
#[cfg(windows)]
mod windows_pause_capture;
#[cfg(windows)]
mod windows_pause_session_start;
pub(crate) use autoedit_args::for_capture as autoedit_args_for_capture;
pub(crate) use capture_artifacts::{camera_artifact_for_capture, resolve_stop_artifacts};
pub(crate) use capture_files::{
    optional_plain_file_in_dir, plain_existing_file_under_dir, plain_existing_file_under_project,
};
#[cfg(test)]
pub(crate) use capture_registry::capture_test_lock;
pub use capture_registry::stop_capture;
use capture_registry::{capture_sessions, reserve_capture};
use capture_session_control::CaptureSessionControl;
pub(crate) use capture_terminal::read_failure as capture_terminal_failure;
pub(crate) use containment::{
    capture_file, create_capture_dir, existing_capture_dir, publish_marker,
};
use doctor_projection::required_capture_card;
pub(crate) use doctor_projection::RecordStartAdmission;
#[cfg(test)]
use doctor_projection::{apply_capture_access_failure, ready_rollup, record_card};
pub use doctor_projection::{MonitorInfo, RecordCard, RecordDoctor};
// Preserves the module-level public WindowInfo path from doctor_projection.
#[allow(unused_imports)]
pub use doctor_projection::WindowInfo;
pub(crate) use export_audio::{for_source as export_audio_for_source, CaptureExportAudio};
pub(crate) use export_job::{retry_screen_record_export, screen_record_export};
#[cfg(test)]
use polish::{autoedit, parse_autoedit_config, read_bounded_json};
pub(crate) use polish::{
    gif_with_control, mux_raw_with_control, plan_cache_tag, render_with_control,
    render_with_control_progress, screen_record_autoedit,
};
pub(crate) use raw_mux::mux_raw_sources;
pub(crate) use recovery::recovery_status_handler;
pub(crate) use scene_projection_start::completed_receipt_for_stop;
pub use screenshot::capture_screenshot_png;
pub(crate) use source_preview_handlers::{
    capability_handler as source_preview_capability_handler,
    frame_handler as source_preview_frame_handler, hide_handler as source_preview_hide_handler,
    pause_handler as source_preview_pause_handler, resume_handler as source_preview_resume_handler,
    start_handler as source_preview_start_handler, status_handler as source_preview_status_handler,
    stop_handler as source_preview_stop_handler,
};
pub(crate) use start_handler::{readiness_status_handler, screen_record_start};

const CAPTURE_FPS_RANGE: std::ops::RangeInclusive<f64> = 1.0..=240.0;

/// Whether this server build configures the live recorder with a passive
/// cursor/click/scroll source. The target-specific dependency declarations
/// enable exactly one such backend on desktop targets; unsupported targets must
/// not advertise `InputEvents` merely because key capture was requested.
const fn configured_passive_input_capture() -> bool {
    cfg!(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux"
    ))
}

fn validate_capture_settings(duration_ms: Option<u64>, fps: f64) -> Result<(), CutError> {
    if duration_ms == Some(0) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "duration_ms must be at least 1 when provided",
            "omit duration_ms for an open-ended recording",
        ));
    }
    if !fps.is_finite() || !CAPTURE_FPS_RANGE.contains(&fps) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "fps must be finite and between 1 and 240",
            format!("received fps {fps}"),
        ));
    }
    Ok(())
}

/// Monotonic per-process counter that disambiguates two `screen_record.start`
/// calls landing in the same nanosecond (combined with pid + nanos → a unique,
/// filesystem-safe `capture_id`).
static CAPTURE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Mint a fresh, filesystem-safe, unique capture id: `cap_<pid>_<nanos>_<seq>`.
pub fn new_capture_id() -> String {
    let seq = CAPTURE_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("cap_{}_{nanos}_{seq}", std::process::id())
}

/// Map a record-core `RecordError` to a cut-core `CutError`. The two are
/// field-identical ({code, message, cause, suggested_action}); the orphan rule
/// forbids a blanket `From` impl in this crate (both types are foreign), so this
/// helper does the 1:1 mapping at the call sites that surface a record error.
pub fn record_err(e: record_core::RecordError) -> CutError {
    let err = CutError::new(&e.code, e.message, e.cause);
    match e.suggested_action {
        Some(a) => err.with_suggested_action(a),
        None => err,
    }
}

/// Point the record crate's ffmpeg/ffprobe resolution (it reads
/// `SHELLX_RECORD_FFMPEG` / `SHELLX_RECORD_FFPROBE`, else PATH) at the SAME
/// binaries cutd resolved via the cut-media toolpath ladder (env → manual pick →
/// beside-exe → app-data → PATH). Without this, `screen_record.doctor` and the
/// in-process render probe ffmpeg differently from `system.doctor` and disagree
/// under a non-login shell where Homebrew isn't on PATH (macOS QA host finding,
///. Idempotent; respects an explicit pre-set override.
fn align_ffmpeg_env() {
    if std::env::var_os("SHELLX_RECORD_FFMPEG").is_none() {
        std::env::set_var("SHELLX_RECORD_FFMPEG", cut_media::toolpath::ffmpeg());
    }
    if std::env::var_os("SHELLX_RECORD_FFPROBE").is_none() {
        std::env::set_var("SHELLX_RECORD_FFPROBE", cut_media::toolpath::ffprobe());
    }
}

/// In-process capability cards (`record_capture::doctor()`). Honestly reports
/// what this build compiled plus what the runtime proves.
pub fn doctor() -> RecordDoctor {
    doctor_projection::doctor()
}

pub(crate) async fn screen_record_doctor(args: Value) -> Result<VerbResult, CutError> {
    doctor_projection::screen_record_doctor(args).await
}

/// Voiceover shares the admitted native capture build/readiness model with the
/// Record workspace. This deliberately does not warm or open a microphone:
/// actual input readiness starts only after an authenticated voiceover request
/// reserves its fixed private source.
pub(crate) fn ensure_voiceover_ready() -> Result<(), CutError> {
    let recorder_doctor = doctor();
    start_readiness::ensure_start_ready(&recorder_doctor.cards)
}

/// Resolve a `<cutproj>/cache/screen_record/` path, creating the dir.
pub(crate) fn screen_record_cache_dir(project_dir: &Path) -> Result<PathBuf, CutError> {
    containment::cache_dir(project_dir)
}

/// Scan on daemon/project open and again immediately before a fresh capture. A scan
/// only ever promotes independently verified finalized checkpoints; live or PID-
/// ambiguous owners are reported as deferred and never signalled.
/// Start a live capture on a background thread (in-process). The thread runs
/// `record_capture::live_capture().capture(cfg, stop)` — which finalizes the source
/// video + EventTrack and writes the `RecordingProject` JSON to `project_path` once
/// the capture ends. Returns IMMEDIATELY; the caller (`screen_record.stop`) polls
/// `project_path` for completion. A failure inside the thread is appended to
/// `log_path` so a stuck/failed capture is diagnosable.
///
/// OPEN-ENDED: when `duration_ms` is `None` the capture runs UNTIL STOPPED —
/// `screen_record.stop` calls [`stop_capture`] which sets this capture's registered
/// flag, ending the recording promptly. When `duration_ms` is `Some(ms)` it is an
/// upper bound (whichever fires first — the deadline or the stop flag — ends it). The
/// flag is registered in [`CAPTURE_STOPS`] under `capture_id` so `stop` can find it.
///
/// Returns an error UP FRONT only when no live-capture backend is compiled for this
/// OS/build (a headless/server build) — so the caller can report a clean
/// `not recording` instead of a thread that silently never produces a file. The
/// desktop-permission prompt (first Linux portal consent / macOS TCC) happens
/// inside `cap.capture()` on the running desktop.
#[allow(clippy::too_many_arguments)]
/// Strip `\\?\` before passing an otherwise-valid path to the Windows capture
/// backend or ffmpeg; `std::fs::canonicalize` returns that prefix on Windows.
/// This is a no-op for ordinary and Unix paths.
fn strip_verbatim_prefix(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        p.to_path_buf()
    }
}

#[cfg(any(windows, target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
pub fn start_capture(
    capture_id: String,
    duration_ms: Option<u64>,
    fps: f64,
    quality: Option<record_core::CaptureQualityRequest>,
    audio: bool,
    microphone_source: record_capture::MicrophoneSource,
    system_audio: bool,
    keys: bool,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    window: Option<String>,
    region: Option<record_capture::CaptureRegion>,
    camera_id: Option<String>,
    project_dir: PathBuf,
    out_dir: PathBuf,
    project_path: PathBuf,
    log_path: PathBuf,
) -> Result<(), CutError> {
    start_capture_with_recording_scenes(
        capture_id,
        duration_ms,
        fps,
        quality,
        audio,
        microphone_source,
        system_audio,
        keys,
        monitor,
        monitor_id,
        window,
        region,
        camera_id,
        None,
        false,
        project_dir,
        out_dir,
        project_path,
        log_path,
    )
}

/// Ordinary `screen_record.start` supplies an accepted frozen public scene
/// snapshot here. Private region/pause callers deliberately pass `None`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_capture_with_recording_scenes(
    capture_id: String,
    duration_ms: Option<u64>,
    fps: f64,
    quality: Option<record_core::CaptureQualityRequest>,
    audio: bool,
    microphone_source: record_capture::MicrophoneSource,
    system_audio: bool,
    keys: bool,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    window: Option<String>,
    region: Option<record_capture::CaptureRegion>,
    camera_id: Option<String>,
    recording_scene_snapshot: Option<record_core::AcceptedStartSnapshot>,
    recording_scene_camera_admitted: bool,
    project_dir: PathBuf,
    out_dir: PathBuf,
    project_path: PathBuf,
    log_path: PathBuf,
) -> Result<(), CutError> {
    start_capture_with_backend(
        capture_id,
        duration_ms,
        fps,
        quality,
        audio,
        microphone_source,
        system_audio,
        keys,
        monitor,
        monitor_id,
        window,
        region,
        camera_id,
        recording_scene_snapshot,
        recording_scene_camera_admitted,
        project_dir,
        out_dir,
        project_path,
        log_path,
        configured_passive_input_capture(),
        || {
            record_capture::live_capture().ok_or_else(|| {
                record_core::RecordError::new(
                    "capture",
                    "no live capture backend",
                    "live_capture() returned None inside the capture thread",
                )
            })
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn start_capture_with_backend<F>(
    capture_id: String,
    duration_ms: Option<u64>,
    fps: f64,
    quality: Option<record_core::CaptureQualityRequest>,
    audio: bool,
    microphone_source: record_capture::MicrophoneSource,
    system_audio: bool,
    keys: bool,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    window: Option<String>,
    region: Option<record_capture::CaptureRegion>,
    camera_id: Option<String>,
    recording_scene_snapshot: Option<record_core::AcceptedStartSnapshot>,
    recording_scene_camera_admitted: bool,
    project_dir: PathBuf,
    out_dir: PathBuf,
    project_path: PathBuf,
    log_path: PathBuf,
    passive_input_capture_active: bool,
    backend: F,
) -> Result<(), CutError>
where
    F: FnOnce() -> record_core::Result<Box<dyn record_capture::Capture>> + Send + 'static,
{
    align_ffmpeg_env();
    validate_capture_settings(duration_ms, fps)?;
    let capture_cadence = cadence::from_server_fps(fps)?;
    // Normalize the `\\?\` verbatim prefix off every runtime capture path before
    // constructing the scene owner or entering a native backend. On Windows the
    // caller canonicalizes these paths, while windows-capture and ffmpeg reject the
    // verbatim spelling (os error 123). Keeping only `out_dir` plain made the same
    // capture directory compare unequal to the verbatim project-owned path during
    // Recording Scenes receipt publication.
    let project_dir = strip_verbatim_prefix(&project_dir);
    let out_dir = strip_verbatim_prefix(&out_dir);
    let project_path = strip_verbatim_prefix(&project_path);
    let log_path = strip_verbatim_prefix(&log_path);
    windows_path::ensure_wgc_checkpoint_path_supported(&out_dir)?;
    let public_recording_scenes = recording_scene_snapshot.is_some();
    let control = match recording_scene_snapshot {
        Some(snapshot) => CaptureSessionControl::new_recording_scenes_live(
            duration_ms,
            audio,
            system_audio,
            passive_input_capture_active,
            &project_dir,
            &capture_id,
            snapshot,
            recording_scene_camera_admitted,
        )
        .map_err(|error| {
            CutError::new(
                error_codes::IO,
                "could not create durable recording scene journal",
                error.to_string(),
            )
        })?,
        None => CaptureSessionControl::new(
            duration_ms,
            audio,
            system_audio,
            passive_input_capture_active,
        ),
    };
    let clock = record_capture::CaptureClock::new();
    let cfg = record_capture::CaptureConfig {
        // Pass `None` straight through for OPEN-ENDED ("record until I stop").
        // The backend treats None as "run until the external stop flag is set".
        duration_ms,
        fps,
        quality,
        capture_cursor: false, // hide the OS cursor; polish re-renders a synthetic one
        monitor,
        // When set, this exact opaque Doctor identity is authoritative over the
        // legacy ordinal. Native backends re-enumerate and refuse rather than
        // substituting a display if it disappeared.
        monitor_id,
        window, // exact opaque app-window id from Doctor (None = whole screen)
        region,
        audio,
        microphone_source,
        microphone_level: control.microphone_meter(),
        // On macOS the SCK backend captures desktop/system audio inside the same
        // stream (the avfoundation `:default` loopback recorded the MIC, not system audio).
        // Linux/Windows ignore this field and capture system audio via their parallel loopback
        // path above; macOS takes the SCK route and the split happens after capture (below).
        system_audio,
        capture_keys: keys,
        out_dir: out_dir.to_string_lossy().into_owned(),
        checkpoint: Some(record_capture::CheckpointConfig {
            manifest_dir: out_dir.to_string_lossy().into_owned(),
            interval_ms: recovery::CHECKPOINT_INTERVAL_MS,
        }),
        clock: Some(clock.clone()),
        readiness: Some(control.readiness()),
        controller_placement: Some(control.controller_placement()),
        source_lifecycle: Some(control.source_lifecycle()),
    };
    let system_audio_lease = system_audio_capture::reserve(system_audio)?;
    let reservation = reserve_capture(capture_id.clone(), control.clone())?;
    if let Err(error) = control.observe_backend_start(clock) {
        let _ = control.terminalize();
        return Err(CutError::new(
            error_codes::IO,
            "could not observe screen-record backend start",
            error.to_string(),
        ));
    }
    let control_for_thread = control.clone();
    let worker = std::thread::Builder::new()
        .name(format!("cut-capture-{capture_id}"))
        .spawn(move || {
            let _reservation = reservation;
            let _terminal_guard = control_for_thread.terminal_guard();
            let _system_audio_lease = system_audio_lease;
            let result: Result<(), record_core::RecordError> = if !control_for_thread
                .claim_native_launch()
            {
                // `screen_record.stop` linearized before this queued worker
                // reached its native handoff. Do not open a portal/WGC/SCK
                // session or start input/audio sidecars after that terminal
                // request; publish the usual typed terminal evidence instead.
                Err(record_core::RecordError::new(
                    record_core::error_codes::CAPTURE,
                    "screen capture stopped before native start",
                    "screen_record.stop won before the recorder opened its native capture backend",
                )
                .with_action("start a new recording when you are ready to capture"))
            } else {
                let camera_sidecar = camera_capture_sidecar::CameraCaptureSidecar::start(
                    camera_id,
                    &capture_id,
                    &out_dir,
                    cfg.clock.clone(),
                    control_for_thread.stop_signal(),
                );
                match camera_sidecar {
                    Err(error) => Err(error),
                    Ok(camera_sidecar) => {
                // Linux/Windows capture system audio beside the screen backend and
                // join it before completion; macOS owns its tap inside native capture.
                let system_audio_worker = if system_audio && !cfg!(target_os = "macos") {
                    let sys_out = out_dir.join("system.wav");
                    let sys_log = log_path.clone();
                    let sys_stop = control_for_thread.stop_signal();
                    let sys_meter = control_for_thread.system_audio_meter();
                    let clock = cfg.clock.clone();
                    std::thread::Builder::new()
                        .name("cut-system-audio".into())
                        .spawn(move || {
                            let Some(capture_started) = clock
                                .as_ref()
                                .and_then(|clock| clock.wait_started(&sys_stop))
                            else {
                                return;
                            };
                            if let Err(e) = system_audio::capture_system_audio_artifact(
                                &sys_out,
                                duration_ms,
                                sys_stop,
                                capture_started,
                                sys_meter,
                            ) {
                                if let Ok(mut f) = std::fs::OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(&sys_log)
                                {
                                    use std::io::Write;
                                    let _ = writeln!(f, "system-audio capture skipped: {e}");
                                }
                            }
                        })
                        .ok()
                } else {
                    None
                };
                // Resolve the backend INSIDE the thread (the trait object is not moved across
                // threads; only this factory crosses the spawn boundary). Stop can
                // still race after the launch claim, so check it once more before
                // entering native code; once native code is entered, all existing
                // platform backends share this same stop signal.
                let captured = (|| {
                    let cap = backend()?;
                    if control_for_thread.stop_requested() {
                        return Err(record_core::RecordError::new(
                            record_core::error_codes::CAPTURE,
                            "screen capture stopped before native start",
                            "screen_record.stop arrived while the recorder was preparing its native capture backend",
                        )
                        .with_action("start a new recording when you are ready to capture"));
                    }
                    let prepublished_project = cap.prepublished_project();
                    admit_public_scene_capture_owner(
                        public_recording_scenes,
                        prepublished_project,
                    )?;
                    cap.capture(&cfg, control_for_thread.stop_signal())
                        .map(|output| (output, prepublished_project))
                })();
                // A natural deadline and a backend error both stop the private
                // lifecycle before they wake sidecars for finalization.
                let scene_terminal = match captured.as_ref() {
                    Ok((output, _)) => control_for_thread
                        .terminalize_after_capture(output.events.duration_ms),
                    Err(_) => control_for_thread.terminalize(),
                }
                .map_err(|error| {
                    record_core::RecordError::new(
                        record_core::error_codes::IO,
                        "seal private Screen-only scene receipt",
                        error.to_string(),
                    )
                });
                // Join even when screen capture failed: the camera owner must
                // close its native writer before this exact capture can publish
                // terminal evidence or release its reservation.
                let camera_artifact = camera_capture_sidecar::CameraCaptureSidecar::finish_for_screen(
                    camera_sidecar,
                    captured.is_ok(),
                );
                system_audio_capture::finalize_worker(system_audio_worker, &log_path)
                    .and(captured)
                    .and_then(|(mut out, prepublished_project)| {
                        scene_terminal?;
                        out.camera_artifact = camera_artifact?;
                        out.webcam_video = out
                            .camera_artifact
                            .as_ref()
                            .map(|artifact| artifact.video.clone());
                        // Current macOS capture writes Core Audio system.wav beside a
                        // video-only source. Keep the compatibility normalizer for an
                        // older source that still carries embedded SCK audio.
                        #[cfg(target_os = "macos")]
                        if system_audio {
                            split_mac_system_audio(std::path::Path::new(&out.source_video));
                        }
                        microphone::persist_capture_outcome(&out_dir, out.microphone_outcome)?;
                        let project = out.into_project_with_capture_cadence(capture_cadence);
                        let bytes = serde_json::to_vec_pretty(&project).map_err(|e| {
                            record_core::RecordError::new(
                                "io",
                                "serialize RecordingProject",
                                e.to_string(),
                            )
                        })?;
                        // A public Recording Scenes receipt commits the exact
                        // source and serialized project bytes first. Stop only
                        // polls project.json, so it cannot observe a finalized
                        // project before its scene replay is durable.
                        scene_projection_start::publish_completed_projection(
                            &control_for_thread,
                            &project_dir,
                            &capture_id,
                            &out_dir,
                            &project_path,
                            &project,
                            &bytes,
                        )?;
                        // The final project projection is published before the manifest's
                        // authoritative Complete receipt. A private pause owner may already
                        // have atomically published the same checked projection alongside its
                        // source/events receipt; never replace that no-replace evidence.
                        if prepublished_project {
                            verify_prepublished_project(&project_path, &bytes)?;
                        } else {
                            record_recovery::replace_synced(&project_path, &bytes).map_err(
                                |e| {
                                    record_core::RecordError::new(
                                        "io",
                                        "write project.json",
                                        e.to_string(),
                                    )
                                },
                            )?;
                        }
                        recovery::complete(&out_dir, Path::new(&project.source_video)).map_err(
                            |e| {
                                record_core::RecordError::new(
                                    "io",
                                    "publish recording receipt",
                                    e.to_string(),
                                )
                            },
                        )?;
                        Ok(())
                    })
                    }
                }
            };
            // Covers a Stop-before-launch rejection as well as the ordinary
            // native completion path. The operation is idempotent and preserves
            // the one terminal state before recovery/error publication.
            let _ = control_for_thread.terminalize();
            if let Err(e) = result {
                if let Err(terminal_error) =
                    capture_terminal::publish_failure(&project_dir, &capture_id, &e)
                {
                    eprintln!(
                        "warning: could not publish capture terminal failure: {terminal_error}"
                    );
                }
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                {
                    use std::io::Write;
                    let _ = writeln!(
                        f,
                        "capture failed [{}]: {} — {}",
                        e.code, e.message, e.cause
                    );
                }
            }
        });
    if let Err(error) = worker {
        let _ = control.terminalize();
        return Err(CutError::new(
            error_codes::IO,
            "could not start the screen-record worker",
            error.to_string(),
        ));
    }
    Ok(())
}

/// A prepublished owner makes `project.json` visible inside its native capture
/// transaction. Public scenes instead seal a create-only receipt first, so it
/// is refused before that owner can enter `Capture::capture` until it can join
/// both publications atomically.
fn admit_public_scene_capture_owner(
    public_recording_scenes: bool,
    prepublished_project: bool,
) -> record_core::Result<()> {
    if public_recording_scenes && prepublished_project {
        return Err(record_core::RecordError::new(
            record_core::error_codes::CAPTURE,
            "Recording Scenes cannot start on a prepublished capture owner",
            "this capture owner would make project.json visible before the required Recording Scenes receipt",
        )
        .with_action("start an ordinary screen capture or use a capture owner that atomically publishes the public scene receipt"));
    }
    Ok(())
}

fn verify_prepublished_project(project_path: &Path, expected: &[u8]) -> record_core::Result<()> {
    if !record_recovery::is_plain_regular_file(project_path).unwrap_or(false) {
        return Err(record_core::RecordError::new(
            "io",
            "verify prepublished project.json",
            "the private capture owner did not publish a local project projection",
        ));
    }
    let actual = std::fs::read(project_path).map_err(|error| {
        record_core::RecordError::new("io", "read prepublished project.json", error.to_string())
    })?;
    if actual != expected {
        return Err(record_core::RecordError::new(
            "io",
            "verify prepublished project.json",
            "the private capture projection differs from its returned recording project",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_capture_with_test_backend<F>(
    capture_id: String,
    duration_ms: Option<u64>,
    fps: f64,
    audio: bool,
    system_audio: bool,
    keys: bool,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    window: Option<String>,
    region: Option<record_capture::CaptureRegion>,
    project_dir: PathBuf,
    out_dir: PathBuf,
    project_path: PathBuf,
    log_path: PathBuf,
    backend: F,
) -> Result<(), CutError>
where
    F: FnOnce() -> record_core::Result<Box<dyn record_capture::Capture>> + Send + 'static,
{
    start_capture_with_backend(
        capture_id,
        duration_ms,
        fps,
        None,
        audio,
        record_capture::MicrophoneSource::SystemDefault,
        system_audio,
        keys,
        monitor,
        monitor_id,
        window,
        region,
        None,
        None,
        false,
        project_dir,
        out_dir,
        project_path,
        log_path,
        configured_passive_input_capture(),
        backend,
    )
}

/// Normalize a legacy macOS source that still embeds system audio.
/// Current Core Audio capture writes `system.wav` beside a video-only source;
/// this best-effort compatibility path preserves the separate-`a_system` contract.
#[cfg(target_os = "macos")]
fn split_mac_system_audio(source_mp4: &Path) {
    use std::process::Command;
    let ffmpeg = cut_media::toolpath::ffmpeg();
    let ffprobe = cut_media::toolpath::ffprobe();
    let mut command = Command::new(&ffprobe);
    command
        .args([
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
        ])
        .arg(source_mp4);
    let has_audio = crate::dispatch::run_bounded_foreground_command(
        &mut command,
        "probe screen-record system audio",
    )
    .map(|o| o.stdout.iter().any(u8::is_ascii_digit))
    .unwrap_or(false);
    if !has_audio {
        return; // no muxed desktop audio — leave the recording as video + mic
    }
    let dir = source_mp4.parent().unwrap_or_else(|| Path::new("."));
    let system_wav = dir.join("system.wav");
    // 1) system.wav source: the recorder's SCStream `Audio` output handler now writes the
    //    AUTHORITATIVE 48 kHz system.wav directly from PCM. Only fall back to extracting
    //    the muxed desktop audio here if that file is absent (older engine / no handler output) —
    //    extracting unconditionally would CLOBBER the handler's file with the muxed copy.
    if !system_wav.is_file() {
        let mut command = Command::new(&ffmpeg);
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(source_mp4)
            .args(["-map", "0:a:0", "-ac", "2", "-ar", "48000"])
            .arg(&system_wav);
        let _ = crate::dispatch::run_bounded_foreground_command(
            &mut command,
            "extract screen-record system audio",
        );
    }
    // 2) strip source.mp4 → VIDEO-ONLY (fast remux, no re-encode) via a temp file then rename.
    let tmp = dir.join("source.video.mp4");
    let mut command = Command::new(&ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(source_mp4)
        .args(["-map", "0:v:0", "-c", "copy", "-an"])
        .arg(&tmp);
    let stripped = crate::dispatch::run_bounded_foreground_command(
        &mut command,
        "strip screen-record system audio",
    );
    if matches!(stripped, Ok(output) if output.status.success()) && tmp.is_file() {
        let _ = std::fs::rename(&tmp, source_mp4);
    }
}

/// The per-OS ffmpeg loopback source for DESKTOP/SYSTEM audio (the game-recording
/// 2nd track). Returns `(format, input)` for `ffmpeg -f <format> -i <input>`.
///
/// PulseAudio-compatible hosts expose the default sink monitor as
/// `@DEFAULT_MONITOR@`; this is distinct from the default microphone source.
#[cfg(all(not(windows), not(target_os = "linux")))] // Windows/Linux use native loopback capture.
fn system_audio_source() -> (&'static str, &'static str) {
    #[cfg(target_os = "macos")]
    {
        // Superseded on the recording path: macOS captures desktop/system audio
        // INSIDE the ScreenCaptureKit stream (see `split_mac_system_audio`), so `start_capture`
        // skips this parallel ffmpeg path on Mac. avfoundation `:default` is the MIC, not system
        // audio — kept only as the last-resort source for the (ignored) loopback unit test.
        ("avfoundation", ":default")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        ("pulse", "@DEFAULT_MONITOR@")
    }
}

#[cfg(all(test, not(windows), unix))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SystemAudioStopStrategy {
    Sigint(i32),
    Kill,
}

#[cfg(all(test, not(windows), unix))]
fn system_audio_stop_strategy(pid: u32) -> SystemAudioStopStrategy {
    match i32::try_from(pid) {
        Ok(pid) => SystemAudioStopStrategy::Sigint(pid),
        Err(_) => SystemAudioStopStrategy::Kill,
    }
}

/// Capture DESKTOP/SYSTEM audio for `duration_ms` to `out` as a 48 kHz stereo WAV via
/// ffmpeg's per-OS loopback ([`system_audio_source`]). This is the recording's SYSTEM track
/// (the game/app sound); the MIC stays the recorder's own synced track, so polish can place
/// them as two independently mixable Cut audio tracks. Duration-bounded (the Record UI path).
#[cfg(all(not(windows), not(target_os = "linux")))] // Windows/Linux use native loopback capture.
pub fn capture_system_audio(out: &Path, duration_ms: u64) -> Result<(), CutError> {
    align_ffmpeg_env();
    let (fmt, input) = system_audio_source();
    let dur = format!("{:.3}", duration_ms as f64 / 1000.0);
    // Use the RESOLVED ffmpeg, not a bare "ffmpeg": on Windows/macOS ffmpeg isn't on
    // PATH (it's in the app-data tools dir), so a bare spawn can fail before
    // capture produces a finalized system-audio file.
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
            "-t",
            &dur,
            "-ac",
            "2",
            "-ar",
            "48000",
            "-y",
        ])
        .arg(out);
    let status = crate::dispatch::run_bounded_foreground_command(
        &mut command,
        "capture screen-record system audio",
    )?
    .status;
    if !status.success() {
        return Err(CutError::new(
            error_codes::IO,
            "system-audio capture failed",
            "no desktop-audio loopback on this OS (Linux: PulseAudio/PipeWire monitor; Windows: WASAPI loopback; macOS: ScreenCaptureKit)",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every runtime capture path uses the same plain spelling before native
    /// capture and Recording Scenes ownership checks. Plain and Unix paths pass
    /// through unchanged.
    #[test]
    fn capture_runtime_paths_strip_verbatim_prefix() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(
                r"\\?\C:\Example\User\Documents\ShellX Cut Projects\rec.cutproj\cache"
            )),
            PathBuf::from(r"C:\Example\User\Documents\ShellX Cut Projects\rec.cutproj\cache"),
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share\rec.cutproj\cache")),
            PathBuf::from(r"\\server\share\rec.cutproj\cache"),
        );
        // Already-plain and Unix paths are untouched.
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"C:\Example\User\rec.cutproj\cache")),
            PathBuf::from(r"C:\Example\User\rec.cutproj\cache"),
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new("/home/u/rec.cutproj/cache")),
            PathBuf::from("/home/u/rec.cutproj/cache"),
        );
    }

    #[test]
    fn autoedit_config_overrides_engine_plan() {
        let dir = tempfile::tempdir().unwrap();
        let track = dir.path().join("events.json");
        let plan = dir.path().join("plan.json");
        std::fs::write(
            &track,
            serde_json::to_vec(&json!({
                "duration_ms": 4000,
                "screen_w": 1920,
                "screen_h": 1080,
                "clicks": [
                    {"t_ms": 1000, "x": 960.0, "y": 540.0, "button": "left", "down": true}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let cfg = parse_autoedit_config(Some(json!({"max_zoom": 3.25}))).unwrap();
        autoedit(&track, &plan, &cfg).unwrap();

        let written: Value = serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
        let max_scale = written["zoom"]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|key| key["scale"].as_f64())
            .fold(0.0_f64, f64::max);
        assert!(
            (max_scale - 3.25).abs() < 1e-9,
            "config.max_zoom should drive the generated plan, got {max_scale}"
        );
    }

    #[test]
    fn autoedit_config_rejects_unknown_keys() {
        let err = parse_autoedit_config(Some(json!({"max_zom": 3.0}))).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(
            err.message.contains("max_zom"),
            "unknown key should be named: {err:?}"
        );
    }

    #[test]
    fn bounded_json_reader_rejects_oversized_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.json");
        std::fs::write(&path, b"12345").unwrap();

        let err = read_bounded_json(&path, "EventTrack", 4, "regenerate it").unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(err.message.contains("exceeds"));
    }

    #[test]
    #[cfg(all(unix, not(windows)))]
    fn system_audio_stop_strategy_falls_back_when_pid_is_too_large_for_sigint() {
        assert_eq!(
            system_audio_stop_strategy(1234),
            SystemAudioStopStrategy::Sigint(1234)
        );
        assert_eq!(
            system_audio_stop_strategy(i32::MAX as u32 + 1),
            SystemAudioStopStrategy::Kill
        );
    }

    #[test]
    fn ready_rollup_requires_core_and_platform_cards() {
        let mk = |name: &str, status: &str| RecordCard {
            name: name.into(),
            status: status.into(),
            detail: String::new(),
            start_admission: RecordStartAdmission::Strict,
        };
        let all_ok = vec![
            mk("ffmpeg", "ok"),
            mk("screen_capture", "ok"),
            mk("input_hook", "ok"),
            mk("webcam", "missing"),
        ];
        assert!(ready_rollup(&all_ok));
        let missing_one = vec![
            mk("ffmpeg", "ok"),
            mk("screen_capture", "missing"),
            mk("input_hook", "ok"),
        ];
        assert!(!ready_rollup(&missing_one));
        let unverified_screen = vec![
            mk("ffmpeg", "ok"),
            mk("screen_capture", "unknown"),
            mk("input_hook", "ok"),
        ];
        assert!(
            !ready_rollup(&unverified_screen),
            "unknown delivery evidence must never make recording ready"
        );
        let linux_missing_gstreamer = vec![
            mk("ffmpeg", "ok"),
            mk("screen_capture", "ok"),
            mk("input_hook", "ok"),
            mk("gstreamer", "missing"),
            mk("wayland_input", "ok"),
        ];
        assert!(!ready_rollup(&linux_missing_gstreamer));
        let linux_missing_input = vec![
            mk("ffmpeg", "ok"),
            mk("screen_capture", "ok"),
            mk("input_hook", "ok"),
            mk("gstreamer", "ok"),
            mk("wayland_input", "degraded"),
        ];
        assert!(!ready_rollup(&linux_missing_input));
        assert!(!ready_rollup(&[]));
    }

    #[test]
    fn canonical_linux_portal_card_gets_typed_start_admission() {
        let prompt = record_card(record_capture::Card {
            id: "screen_capture".into(),
            kind: "capture".into(),
            status: "unknown".into(),
            detail: record_capture::LINUX_PORTAL_PROMPT_DEFERRED_DETAIL.into(),
        });
        assert_eq!(
            prompt.start_admission,
            if cfg!(target_os = "linux") {
                RecordStartAdmission::LinuxPortalPromptDeferred
            } else {
                RecordStartAdmission::Strict
            }
        );
        let arbitrary = record_card(record_capture::Card {
            id: "screen_capture".into(),
            kind: "capture".into(),
            status: "unknown".into(),
            detail: "an unrelated prompt-deferred backend".into(),
        });
        assert_eq!(arbitrary.start_admission, RecordStartAdmission::Strict);
    }

    #[test]
    fn capture_access_failure_degrades_ready_card_with_recovery_guidance() {
        let mut cards = vec![
            RecordCard {
                name: "ffmpeg".into(),
                status: "ok".into(),
                detail: String::new(),
                start_admission: RecordStartAdmission::Strict,
            },
            RecordCard {
                name: "screen_capture".into(),
                status: "ok".into(),
                detail: "compiled backend".into(),
                start_admission: RecordStartAdmission::Strict,
            },
            RecordCard {
                name: "input_hook".into(),
                status: "ok".into(),
                detail: String::new(),
                start_admission: RecordStartAdmission::Strict,
            },
        ];
        apply_capture_access_failure(&mut cards);
        let capture = cards
            .iter()
            .find(|card| card.name == "screen_capture")
            .unwrap();
        assert_eq!(capture.status, "degraded");
        assert!(capture.detail.contains("Privacy & Security"));
        assert!(capture.detail.contains("quit and reopen"));
        assert!(!ready_rollup(&cards));
    }

    #[test]
    fn capture_settings_reject_invalid_duration_and_fps() {
        assert!(validate_capture_settings(None, 30.0).is_ok());
        assert!(validate_capture_settings(Some(1), 1.0).is_ok());
        assert!(validate_capture_settings(Some(1), 240.0).is_ok());
        for fps in [0.0, -1.0, 240.1, f64::INFINITY, f64::NAN] {
            let error = validate_capture_settings(None, fps).unwrap_err();
            assert_eq!(error.code, error_codes::INVALID_ARGS);
        }
        let error = validate_capture_settings(Some(0), 30.0).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
    }

    #[test]
    fn record_err_maps_all_fields() {
        let re = record_core::RecordError::new("ffmpeg", "boom", "bad pipe")
            .with_action("install ffmpeg");
        let ce = record_err(re);
        // CutError serializes {code,message,cause,suggested_action} — round-trip check.
        let v = serde_json::to_value(&ce).unwrap();
        assert_eq!(v["code"], "ffmpeg");
        assert_eq!(v["message"], "boom");
        assert_eq!(v["cause"], "bad pipe");
        assert_eq!(v["suggested_action"], "install ffmpeg");
    }

    /// cutd-RESTART fallback: stopping a capture id that was never registered (or
    /// was lost when cutd restarted mid-capture) is a harmless no-op returning false —
    /// the caller then falls back to the file poll for a capture that finalized on its
    /// own bound. Must NOT panic.
    #[test]
    fn stop_capture_unknown_id_is_a_noop() {
        let unknown = format!("cap_never_started_{}", std::process::id());
        assert!(
            !stop_capture(&unknown),
            "an unknown capture id returns false (file-poll fallback path)"
        );
    }
}
