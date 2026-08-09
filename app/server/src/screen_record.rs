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
use crate::dispatch::{parse_args, snapshot};
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
mod autoedit_args;
mod capture_artifacts;
mod capture_files;
mod containment;
mod export_audio;
mod export_job;
pub(crate) mod finalization_budget;
mod polish;
mod raw_mux;
pub(crate) mod recovery;
mod start_readiness;
pub(crate) mod system_audio;
mod system_audio_capture;
mod windows_path;
pub(crate) use autoedit_args::for_capture as autoedit_args_for_capture;
pub(crate) use capture_artifacts::resolve_stop_artifacts;
pub(crate) use capture_files::{
    optional_plain_file_in_dir, plain_existing_file_under_dir, plain_existing_file_under_project,
};
pub(crate) use containment::{
    capture_file, create_capture_dir, existing_capture_dir, publish_marker,
};
pub(crate) use export_audio::{for_source as export_audio_for_source, CaptureExportAudio};
pub(crate) use export_job::screen_record_export;
#[cfg(test)]
use polish::{autoedit, parse_autoedit_config, read_bounded_json};
pub(crate) use polish::{
    gif_with_control, mux_raw_with_control, plan_cache_tag, render_with_control,
    screen_record_autoedit,
};
pub(crate) use raw_mux::mux_raw_sources;
pub(crate) use recovery::recovery_status_handler;

/// OPEN-ENDED CAPTURE registry: process-global map from `capture_id` → the
/// external stop flag for that running capture. `start_capture` inserts the flag
/// (cloned into the backend's `capture()` call); `stop_capture` sets it so the
/// backend's poll loop ends the recording PROMPTLY (instead of running to a fixed
/// deadline), then the caller's file-poll finalizes as before. Stored behind a
/// `OnceLock<Mutex<…>>` so it needs no external dep and is lazily initialized.
static CAPTURE_STOPS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

const MIN_CAPTURE_FPS: f64 = 1.0;
const MAX_CAPTURE_FPS: f64 = 240.0;

fn capture_stops() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    CAPTURE_STOPS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct CaptureReservation {
    capture_id: String,
}

impl Drop for CaptureReservation {
    fn drop(&mut self) {
        release_capture(&self.capture_id);
    }
}

fn reserve_capture(
    capture_id: String,
    stop: Arc<AtomicBool>,
) -> Result<CaptureReservation, CutError> {
    let mut map = match capture_stops().lock() {
        Ok(m) => m,
        Err(p) => p.into_inner(),
    };
    if let Some(active_id) = map.keys().next() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "a screen recording is already active",
            format!("capture {active_id} still owns the recording devices"),
        )
        .with_suggested_action(
            "stop the active recording and wait for it to finalize before starting another",
        ));
    }
    map.insert(capture_id.clone(), stop);
    Ok(CaptureReservation { capture_id })
}

fn release_capture(capture_id: &str) {
    let mut map = match capture_stops().lock() {
        Ok(m) => m,
        Err(p) => p.into_inner(),
    };
    map.remove(capture_id);
}

/// Signal the running capture `capture_id` to stop EARLY. Sets its registry
/// flag (if present) so an OPEN-ENDED capture ends now. The worker keeps its registry
/// reservation until every capture thread has finished. Returns `true` if a flag was found+set (the
/// capture was tracked in THIS cutd process), `false` if not — e.g. cutd restarted
/// mid-capture and lost the in-memory flag, in which case the caller still falls back
/// to the duration-bounded file poll. Idempotent.
pub fn stop_capture(capture_id: &str) -> bool {
    let map = match capture_stops().lock() {
        Ok(m) => m,
        Err(p) => p.into_inner(), // a poisoned lock still lets us signal stop
    };
    if let Some(flag) = map.get(capture_id) {
        flag.store(true, Ordering::Relaxed);
        true
    } else {
        false
    }
}

fn validate_capture_settings(duration_ms: Option<u64>, fps: f64) -> Result<(), CutError> {
    if duration_ms == Some(0) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "duration_ms must be at least 1 when provided",
            "omit duration_ms for an open-ended recording",
        ));
    }
    if !fps.is_finite() || !(MIN_CAPTURE_FPS..=MAX_CAPTURE_FPS).contains(&fps) {
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

/// `debug.screenshot` (server-side): capture a single still of the primary display (or a
/// chosen `monitor`/`window`) to `out_png`, returning `(width, height)`. Unlike
/// `ui.screenshot` (which relays to a connected WebView and fails headless), this grabs the
/// ACTUAL screen via the in-process OS recorder, so it works regardless of UI-client state —
/// the tool for visually verifying the app, dialogs and menus while driving cutd over the
/// debug API. Implementation: record a sub-second clip via the proven per-OS capture
/// (ScreenCaptureKit / WGC / portal), then extract its FIRST frame to PNG. No audio, no input
/// capture, cursor shown (debug shots want the pointer). Best-effort temp cleanup.
pub fn capture_screenshot_png(
    out_png: &Path,
    monitor: Option<u32>,
    window: Option<String>,
) -> Result<(u32, u32), CutError> {
    align_ffmpeg_env();
    let cap = record_capture::live_capture().ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "no screen-capture backend on this build/OS",
            "debug.screenshot needs a desktop session built with the capture feature",
        )
    })?;
    let stop = Arc::new(AtomicBool::new(false));
    let _reservation = reserve_capture(new_capture_id(), stop.clone())?;
    // Unique scratch directory for this throwaway capture.
    let uniq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("cutd_shot_{uniq}"));
    std::fs::create_dir_all(&tmp).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "create screenshot scratch dir",
            e.to_string(),
        )
    })?;
    let cfg = record_capture::CaptureConfig {
        duration_ms: Some(220), // long enough for ≥1 frame on a static screen
        fps: 4.0,
        capture_cursor: true,
        monitor,
        window,
        audio: false,
        system_audio: false,
        capture_keys: false,
        out_dir: tmp.to_string_lossy().into_owned(),
        checkpoint: None,
        clock: None,
    };
    let capture_res = cap.capture(&cfg, stop).map_err(record_err);
    let result = capture_res.and_then(|out| {
        let src = &out.source_video;
        // Extract frame 0 → PNG (resolved ffmpeg; Win/macOS keep ffmpeg in app-data, not PATH).
        let mut command = std::process::Command::new(cut_media::toolpath::ffmpeg());
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(src)
            .args(["-frames:v", "1", "-update", "1"])
            .arg(out_png);
        let status = crate::dispatch::run_bounded_foreground_command(
            &mut command,
            "extract screen-record screenshot frame",
        )?
        .status;
        if !status.success() || !out_png.is_file() {
            return Err(CutError::new(
                error_codes::IO,
                "screenshot frame extract failed",
                "ffmpeg could not write the PNG from the captured frame",
            ));
        }
        // Probe the PNG dimensions (best-effort; 0×0 if ffprobe is unavailable).
        let mut command = std::process::Command::new(cut_media::toolpath::ffprobe());
        command
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height",
                "-of",
                "csv=p=0:s=x",
            ])
            .arg(out_png);
        let dims = crate::dispatch::run_bounded_foreground_command(
            &mut command,
            "probe screen-record screenshot dimensions",
        )
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            let s = s.trim();
            let (w, h) = s.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or((0u32, 0u32));
        Ok(dims)
    });
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// One capability card (kept field-stable for the `screen_record.doctor` result
/// the UI/agent already consume): `name` = the record card id, `status` verbatim
/// (`ok`|`missing`|`degraded`|`unknown`), `detail` the human hint. `unknown`
/// means the backend is present but Cut deliberately avoided a prompt-prone proof;
/// it is never ready/green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordStartAdmission {
    Strict,
    LinuxPortalPromptDeferred,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecordCard {
    pub name: String,
    pub status: String,
    pub detail: String,
    /// Server-only action provenance; clients receive explicit `start_allowed`, never card prose.
    #[serde(skip_serializing)]
    pub(crate) start_admission: RecordStartAdmission,
}

/// One display the user can pick as the capture target (mirrors
/// `record_capture::MonitorInfo`, kept field-stable for the
/// `screen_record.doctor` result the UI's monitor PICKER consumes). The 1-based
/// `index` is what `screen_record.start{monitor}` expects.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MonitorInfo {
    pub index: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

/// One application window the user can pick for the in-app WINDOW picker (mirror of
/// `record_capture::WindowInfo`). The `title` is what `screen_record.start{window}`
/// re-resolves at capture time (so the UI passes a `WindowInfo.title` straight back).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app: String,
}

/// The doctor result: every capability card plus a strict `ready` health rollup and
/// action-specific `start_allowed` admission. The latter can be true only for the
/// user-initiated Linux portal picker while `ready` remains false/unknown.
///
/// `ready` is true iff the
/// three capabilities a recording NEEDS (`ffmpeg`, `screen_capture`, `input_hook`)
/// are all `ok`. Optional cards (`webcam`, …) don't gate `ready`.
///
/// `monitors` lists the displays the user can pick from for the in-app monitor
/// PICKER. It is populated on Windows and macOS, and empty on Linux because the
/// XDG portal owns source selection. The UI shows a `<select>` only when there
/// are at least two entries.
#[derive(Debug, Clone, Serialize)]
pub struct RecordDoctor {
    pub cards: Vec<RecordCard>,
    pub ready: bool,
    pub start_allowed: bool,
    pub monitors: Vec<MonitorInfo>,
    /// Application windows for the in-app WINDOW picker (record one app, not the whole
    /// screen). Populated on Windows and macOS; empty on Linux/headless. The UI
    /// offers a window option per entry.
    pub windows: Vec<WindowInfo>,
}

fn required_capture_card(cards: &[RecordCard], name: &str) -> bool {
    matches!(name, "ffmpeg" | "screen_capture" | "input_hook")
        || (matches!(name, "gstreamer" | "wayland_input")
            && cards.iter().any(|card| card.name == name))
}

/// `ready` rollup from a card list. Linux emits additional required cards for
/// its GStreamer encode path and session-specific input hook.
fn ready_rollup(cards: &[RecordCard]) -> bool {
    [
        "ffmpeg",
        "screen_capture",
        "input_hook",
        "gstreamer",
        "wayland_input",
    ]
    .into_iter()
    .filter(|name| required_capture_card(cards, name))
    .all(|name| {
        cards
            .iter()
            .any(|card| card.name == name && card.status == "ok")
    })
}

fn apply_capture_access_failure(cards: &mut [RecordCard]) {
    let Some(card) = cards
        .iter_mut()
        .find(|card| card.name == "screen_capture" && card.status == "ok")
    else {
        return;
    };
    card.status = "degraded".into();
    card.detail = "Screen capture permission is unavailable — allow ShellX Cut in System Settings > Privacy & Security > Screen & System Audio Recording (Screen Recording on older macOS), then quit and reopen the app".into();
}

/// In-process capability cards (`record_capture::doctor()`). Honestly reports what
/// this build compiled + what the runtime environment proves. A native backend is
/// `ok` only after it delivers a discarded frame to Cut; `unknown` is not ready.
pub fn doctor() -> RecordDoctor {
    align_ffmpeg_env();
    let mut cards: Vec<RecordCard> = record_capture::doctor()
        .into_iter()
        .map(record_card)
        .collect();
    // Preserve ScreenCaptureKit/TCC enumeration failures. The old Vec-only API
    // collapsed permission denial to an empty picker while leaving ready=true.
    let monitor_probe = record_capture::list_monitors_checked();
    if monitor_probe.is_err() {
        apply_capture_access_failure(&mut cards);
    }
    let ready = ready_rollup(&cards);
    let start_allowed = start_readiness::start_allowed(&cards);
    // Enumerate displays for the in-app picker. Linux deliberately returns an
    // empty successful result because its portal owns source selection.
    let monitors = monitor_probe
        .unwrap_or_default()
        .into_iter()
        .map(|m| MonitorInfo {
            index: m.index,
            name: m.name,
            width: m.width,
            height: m.height,
            primary: m.primary,
        })
        .collect();
    // Enumerate application windows for the in-app picker (Windows/macOS; empty
    // on Linux). Mirror record_capture::WindowInfo 1:1.
    let windows = record_capture::list_windows()
        .into_iter()
        .map(|w| WindowInfo {
            id: w.id,
            title: w.title,
            app: w.app,
        })
        .collect();
    RecordDoctor {
        cards,
        ready,
        start_allowed,
        monitors,
        windows,
    }
}

fn record_card(c: record_capture::Card) -> RecordCard {
    let start_admission = if record_capture::is_linux_portal_prompt_deferred(&c.status, &c.detail) {
        RecordStartAdmission::LinuxPortalPromptDeferred
    } else {
        RecordStartAdmission::Strict
    };
    RecordCard {
        name: c.id,
        status: c.status,
        detail: c.detail,
        start_admission,
    }
}

/// screen_record.doctor{} — report the recorder's capability cards (the environment
/// doctor, the recorder analog of system.doctor). Calls `record_capture::doctor()`
/// IN-PROCESS (no child process), maps each card, and rolls up a `ready` flag.
/// The bounded screen probe stores no image data and will not use a portal picker.
///
/// No project is required.
pub(crate) async fn screen_record_doctor(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        // The Record surface passes warm_mic:true on mount when mic capture is on, so
        // we answer the OS permission prompt before the user's first short recording.
        #[serde(default)]
        warm_mic: bool,
    }
    let a: Args = parse_args(args)?;
    let d = doctor();
    let mic_warm = if a.warm_mic {
        // cpal may enter an unbounded native-driver call on Windows. Keep that
        // blocking work off the async request runtime, and return an honest
        // bounded result even if the driver ignores the low-level stop flag.
        let task = tokio::task::spawn_blocking(warm_mic);
        Some(
            match tokio::time::timeout(std::time::Duration::from_secs(4), task).await {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => json!({
                    "live": false,
                    "device": null,
                    "supported": true,
                    "error": format!("microphone warm-up worker failed: {error}"),
                }),
                Err(_) => json!({
                    "live": false,
                    "device": null,
                    "supported": true,
                    "timed_out": true,
                    "error": "microphone warm-up exceeded 4 seconds",
                }),
            },
        )
    } else {
        None
    };
    Ok(VerbResult::ok(json!({
        "cards": d.cards,
        "ready": d.ready,
        "start_allowed": d.start_allowed,
        "monitors": d.monitors,
        "windows": d.windows,
        "mic_warm": mic_warm,
    })))
}

/// screen_record.start{duration_ms?, fps?, audio?, system_audio?, studio?,
/// keys?, monitor?, window?, rationale?} —
/// kick off a live, duration-bounded or open-ended capture in the background.
pub(crate) async fn screen_record_start(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // `rationale` is operator metadata only (capture is not an op).
    struct Args {
        duration_ms: Option<u64>,
        fps: Option<f64>,
        #[serde(default)]
        audio: bool,
        #[serde(default)]
        system_audio: bool,
        studio: Option<Value>,
        #[serde(default)]
        keys: bool,
        monitor: Option<u32>,
        window: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let duration_ms: Option<u64> = a.duration_ms;
    let fps = a.fps.unwrap_or(30.0);
    validate_capture_settings(duration_ms, fps)?;
    let (_project, _edl, dir, _at) = snapshot(state).await?;
    let recorder_doctor = doctor();
    start_readiness::ensure_start_ready(&recorder_doctor.cards)?;

    let capture_id = new_capture_id();
    windows_path::ensure_pre_marker_path(&dir, &capture_id)?;
    let recovery_scan = recovery::scan_recovery_for_project(&dir)?;
    let out_dir = create_capture_dir(&dir, &capture_id)?;
    recovery::begin(&out_dir, &capture_id)?;
    let project_path = capture_file(&dir, &capture_id, "project.json")?;
    let pid = std::process::id();
    let marker_body = json!({
        "pid": pid,
        "duration_ms": duration_ms,
        "open_ended": duration_ms.is_none(),
        "fps": fps,
        "audio": a.audio,
        "system_audio": a.system_audio,
        "studio": a.studio,
        "keys": a.keys,
    });
    publish_marker(
        &dir,
        &capture_id,
        &serde_json::to_vec_pretty(&marker_body).unwrap_or_default(),
    )?;

    let record_log = out_dir.join("record.log");
    if let Err(error) = start_capture(
        capture_id.clone(),
        duration_ms,
        fps,
        a.audio,
        a.system_audio,
        a.keys,
        a.monitor,
        a.window,
        out_dir.clone(),
        project_path.clone(),
        record_log,
    ) {
        let _ = std::fs::remove_dir_all(&out_dir);
        return Err(error);
    }

    Ok(VerbResult::ok(json!({
        "capture_id": capture_id,
        "out_dir": out_dir,
        "status": "recording",
        "duration_ms": duration_ms,
        "open_ended": duration_ms.is_none(),
        "studio_events": crate::screen_record_studio::studio_events_path(&out_dir),
        "recovery_scan": { "recovered": recovery_scan.recovered, "deferred": recovery_scan.deferred, "failed_closed": recovery_scan.failed_closed },
        "note": if duration_ms.is_none() {
            "OPEN-ENDED capture: runs until screen_record.stop. The first capture pops a one-time XDG ScreenCast consent dialog on the desktop"
        } else {
            "capture runs up to duration_ms (or until screen_record.stop). The first capture pops a one-time XDG ScreenCast consent dialog on the desktop"
        },
    })))
}

/// Resolve a `<cutproj>/cache/screen_record/` path, creating the dir.
pub(crate) fn screen_record_cache_dir(project_dir: &Path) -> Result<PathBuf, CutError> {
    containment::cache_dir(project_dir)
}

/// Scan on daemon/project open and again immediately before a fresh capture. A scan
/// only ever promotes independently verified finalized checkpoints; live or PID-
/// ambiguous owners are reported as deferred and never signalled.
/// Warm the default mic on entering the Record surface (see
/// `record_capture::warm_mic`) — opens it briefly via the recorder's own cpal path so
/// the OS permission prompt + stream init happen BEFORE the user records. Returns
/// `{live, device, supported}`, surfaced by `screen_record.doctor{warm_mic:true}`.
pub fn warm_mic() -> serde_json::Value {
    let w = record_capture::warm_mic(1500);
    serde_json::json!({ "live": w.live, "device": w.device, "supported": w.supported })
}

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

pub fn start_capture(
    capture_id: String,
    duration_ms: Option<u64>,
    fps: f64,
    audio: bool,
    system_audio: bool,
    keys: bool,
    monitor: Option<u32>,
    window: Option<String>,
    out_dir: PathBuf,
    project_path: PathBuf,
    log_path: PathBuf,
) -> Result<(), CutError> {
    align_ffmpeg_env();
    validate_capture_settings(duration_ms, fps)?;
    // Normalize the `\\?\` verbatim prefix off the capture dir BEFORE deriving
    // any path. On Windows the caller canonicalizes the project dir → verbatim path;
    // the windows-capture backend + ffmpeg reject it (os error 123 — "filename,
    // directory name, or volume label syntax is incorrect"), so Windows recording
    // produced only a record.log and no video until this strip. `source.mp4` and
    // `system.wav` are both derived from `out_dir`, so one strip fixes both.
    let out_dir = strip_verbatim_prefix(&out_dir);
    windows_path::ensure_wgc_checkpoint_path_supported(&out_dir)?;
    let cfg = record_capture::CaptureConfig {
        // Pass `None` straight through for OPEN-ENDED ("record until I stop").
        // The backend treats None as "run until the external stop flag is set".
        duration_ms,
        fps,
        capture_cursor: false, // hide the OS cursor; polish re-renders a synthetic one
        monitor,
        window, // capture one app window by title (Windows-only; None = whole screen)
        audio,
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
        clock: Some(record_capture::CaptureClock::new()),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let reservation = reserve_capture(capture_id.clone(), stop.clone())?;
    let stop_for_thread = stop.clone();
    std::thread::Builder::new()
        .name(format!("cut-capture-{capture_id}"))
        .spawn(move || {
            let _reservation = reservation;
            // Linux/Windows capture system audio beside the screen backend. The
            // orchestrator owns and joins this worker before project.json signals
            // completion; macOS owns its Core Audio tap inside the native capture
            // backend so it can stop exactly with the ScreenCaptureKit boundary.
            let system_audio_worker = if system_audio && !cfg!(target_os = "macos") {
                let sys_out = out_dir.join("system.wav");
                let sys_log = log_path.clone();
                let sys_stop = stop_for_thread.clone();
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
            // Resolve the backend INSIDE the thread (the trait object isn't moved across
            // threads — `cfg` is plain Send data).
            let captured = (|| {
                let cap = record_capture::live_capture().ok_or_else(|| {
                    record_core::RecordError::new(
                        "capture",
                        "no live capture backend",
                        "live_capture() returned None inside the capture thread",
                    )
                })?;
                cap.capture(&cfg, stop_for_thread.clone())
            })();
            stop_for_thread.store(true, Ordering::Relaxed);
            let result: Result<(), record_core::RecordError> =
                system_audio_capture::finalize_worker(system_audio_worker, &log_path)
                    .and(captured)
                    .and_then(|out| {
                        // Current macOS capture writes Core Audio system.wav beside a
                        // video-only source. Keep the compatibility normalizer for an
                        // older source that still carries embedded SCK audio.
                        #[cfg(target_os = "macos")]
                        if system_audio {
                            split_mac_system_audio(std::path::Path::new(&out.source_video));
                        }
                        let project = out.into_project();
                        let bytes = serde_json::to_vec_pretty(&project).map_err(|e| {
                            record_core::RecordError::new(
                                "io",
                                "serialize RecordingProject",
                                e.to_string(),
                            )
                        })?;
                        // The final project projection is published before the manifest's
                        // authoritative Complete receipt. If cutd dies in this tiny window,
                        // recovery::scan recognizes this sealed local projection and appends
                        // the receipt instead of falsely publishing recovered.mp4.
                        record_recovery::replace_synced(&project_path, &bytes).map_err(|e| {
                            record_core::RecordError::new("io", "write project.json", e.to_string())
                        })?;
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
                    });
            if let Err(e) = result {
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
        })
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                "could not start the screen-record worker",
                e.to_string(),
            )
        })?;
    Ok(())
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

    /// The capture dir's `\\?\` verbatim prefix must be stripped before it
    /// reaches the windows-capture backend / ffmpeg (which reject it with os error
    /// 123). Plain and Unix paths pass through unchanged.
    #[test]
    fn capture_out_dir_strips_verbatim_prefix() {
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

    /// A reservation owns the devices until worker cleanup. `stop_capture` signals
    /// the shared flag but deliberately keeps the reservation while finalization runs.
    #[test]
    fn capture_registry_enforces_single_owner_until_worker_release() {
        let id = format!("cap_test_{}", std::process::id());
        let second_id = format!("cap_test_second_{}", std::process::id());
        let flag = Arc::new(AtomicBool::new(false));
        let reservation = reserve_capture(id.clone(), flag.clone()).unwrap();
        let conflict = reserve_capture(second_id.clone(), Arc::new(AtomicBool::new(false)))
            .err()
            .expect("a second capture must be rejected");
        assert_eq!(conflict.code, error_codes::CONFLICT);

        // The "backend" hasn't been told to stop yet.
        assert!(!flag.load(Ordering::Relaxed), "flag starts unset");

        // Stop signals but keeps ownership until every worker has finished.
        assert!(stop_capture(&id), "stop_capture found the registered flag");
        assert!(
            flag.load(Ordering::Relaxed),
            "the flag the backend polls is now set — its loop will finalize"
        );

        assert!(
            stop_capture(&id),
            "repeat stop remains idempotent while finalization owns the devices"
        );
        assert!(
            capture_stops().lock().unwrap().contains_key(&id),
            "registry retains the capture until worker cleanup"
        );
        drop(reservation);
        assert!(!stop_capture(&id), "worker release removes the reservation");
        let second = reserve_capture(second_id, Arc::new(AtomicBool::new(false))).unwrap();
        drop(second);
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
