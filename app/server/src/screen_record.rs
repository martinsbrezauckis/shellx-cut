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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::dispatch::{parse_args, snapshot};
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde::Serialize;
use serde_json::{json, Value};

mod polish;
#[cfg(test)]
use polish::{autoedit, parse_autoedit_config, read_bounded_json};
pub(crate) use polish::{
    mux_raw, mux_raw_sources, plan_cache_tag, render, screen_record_autoedit, screen_record_export,
};

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
    // Unique scratch dir for the throwaway capture (SystemTime is available in Rust — only the
    // workflow JS sandbox blocks the clock).
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
    };
    let capture_res = cap.capture(&cfg, stop).map_err(record_err);
    let result = capture_res.and_then(|out| {
        let src = &out.source_video;
        // Extract frame 0 → PNG (resolved ffmpeg; Win/macOS keep ffmpeg in app-data, not PATH).
        let status = std::process::Command::new(cut_media::toolpath::ffmpeg())
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(src)
            .args(["-frames:v", "1", "-update", "1"])
            .arg(out_png)
            .status()
            .map_err(|e| {
                CutError::new(
                    error_codes::IO,
                    "ffmpeg screenshot extract spawn",
                    e.to_string(),
                )
            })?;
        if !status.success() || !out_png.is_file() {
            return Err(CutError::new(
                error_codes::IO,
                "screenshot frame extract failed",
                "ffmpeg could not write the PNG from the captured frame",
            ));
        }
        // Probe the PNG dimensions (best-effort; 0×0 if ffprobe is unavailable).
        let dims = std::process::Command::new(cut_media::toolpath::ffprobe())
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
            .arg(out_png)
            .output()
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
/// (`ok`|`missing`|`degraded`), `detail` the human hint.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecordCard {
    pub name: String,
    pub status: String,
    pub detail: String,
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

/// The doctor result: every capability card plus a `ready` rollup — true iff the
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

fn ensure_capture_ready() -> Result<(), CutError> {
    let status = doctor();
    if status.ready {
        return Ok(());
    }
    let blocked = [
        "ffmpeg",
        "screen_capture",
        "input_hook",
        "gstreamer",
        "wayland_input",
    ]
    .into_iter()
    .filter(|name| required_capture_card(&status.cards, name))
    .filter_map(|name| {
        let card = status.cards.iter().find(|card| card.name == name);
        match card {
            Some(card) if card.status != "ok" => {
                Some(format!("{}={} ({})", card.name, card.status, card.detail))
            }
            None => Some(format!("{name}=missing")),
            _ => None,
        }
    })
    .collect::<Vec<_>>()
    .join("; ");
    Err(CutError::new(
        error_codes::NOT_FOUND,
        "screen recording is not ready on this system",
        blocked,
    )
    .with_suggested_action(
        "resolve the required screen_record.doctor cards, then retry the recording",
    ))
}

/// In-process capability cards (`record_capture::doctor()`). Honestly reports what
/// this build compiled + what the runtime environment offers; never spawns.
pub fn doctor() -> RecordDoctor {
    align_ffmpeg_env();
    let mut cards: Vec<RecordCard> = record_capture::doctor()
        .into_iter()
        .map(|c| RecordCard {
            name: c.id,
            status: c.status,
            detail: c.detail,
        })
        .collect();
    // Preserve ScreenCaptureKit/TCC enumeration failures. The old Vec-only API
    // collapsed permission denial to an empty picker while leaving ready=true.
    let monitor_probe = record_capture::list_monitors_checked();
    if monitor_probe.is_err() {
        apply_capture_access_failure(&mut cards);
    }
    let ready = ready_rollup(&cards);
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
        monitors,
        windows,
    }
}

/// screen_record.doctor{} — report the recorder's capability cards (the environment
/// doctor, the recorder analog of system.doctor). Calls `record_capture::doctor()`
/// IN-PROCESS (no child process), maps each card, and rolls up a `ready` flag.
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

    let capture_id = new_capture_id();
    let cache = screen_record_cache_dir(&dir)?;
    let out_dir = cache.join(&capture_id);
    std::fs::create_dir_all(&out_dir).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!(
                "could not create the capture dir {}: {e}",
                out_dir.display()
            ),
            "writing <project>/cache/screen_record/<capture_id>/ failed",
        )
    })?;
    let project_path = out_dir.join("project.json");
    let project_path_s = project_path.display().to_string();
    let pid = std::process::id();
    let marker = out_dir.join(".capture.json");
    let marker_body = json!({
        "pid": pid,
        "duration_ms": duration_ms,
        "open_ended": duration_ms.is_none(),
        "fps": fps,
        "audio": a.audio,
        "system_audio": a.system_audio,
        "studio": a.studio,
        "keys": a.keys,
        "project_path": project_path_s,
    });
    std::fs::write(
        &marker,
        serde_json::to_vec_pretty(&marker_body).unwrap_or_default(),
    )
    .map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!(
                "could not write the capture marker {}: {e}",
                marker.display()
            ),
            "writing <capture_id>/.capture.json failed",
        )
    })?;

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
        "note": if duration_ms.is_none() {
            "OPEN-ENDED capture: runs until screen_record.stop. The first capture pops a one-time XDG ScreenCast consent dialog on the desktop"
        } else {
            "capture runs up to duration_ms (or until screen_record.stop). The first capture pops a one-time XDG ScreenCast consent dialog on the desktop"
        },
    })))
}

/// Resolve a `<cutproj>/cache/screen_record/` path, creating the dir.
pub(crate) fn screen_record_cache_dir(project_dir: &Path) -> Result<PathBuf, CutError> {
    let d = project_dir.join("cache").join("screen_record");
    std::fs::create_dir_all(&d).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not create the screen_record cache dir: {e}"),
            "writing <project>/cache/screen_record/ failed",
        )
    })?;
    Ok(d)
}

pub(crate) fn validate_screen_record_capture_id(capture_id: &str) -> Result<(), CutError> {
    let valid = !capture_id.is_empty()
        && capture_id.len() <= 128
        && capture_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::INVALID_ARGS,
        "capture_id is not valid",
        "capture_id must be the filesystem-safe id returned by screen_record.start",
    )
    .with_suggested_action("pass the exact capture_id from screen_record.start"))
}

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
/// Strip the Windows extended-length (`\\?\`) "verbatim" prefix from a path.
/// Rust's `std::fs` tolerates verbatim paths, but the external capture backend
/// (windows-capture crate + ffmpeg muxer) rejects them with `ERROR_INVALID_NAME`
/// (os error 123). `std::fs::canonicalize` on Windows returns verbatim paths, so
/// the capture `out_dir` (a canonicalized project cache dir) arrives prefixed and
/// MUST be normalized before any path is handed to the recorder/ffmpeg. No-op on
/// non-verbatim paths and on Unix. Mirrors `cut_media::render::strip_verbatim_prefix`
/// (kept local to avoid a cross-crate dependency for one tiny helper).
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
    // Validate before reserving devices or starting the system-audio worker.
    ensure_capture_ready()?;
    // Normalize the `\\?\` verbatim prefix off the capture dir BEFORE deriving
    // any path. On Windows the caller canonicalizes the project dir → verbatim path;
    // the windows-capture backend + ffmpeg reject it (os error 123 — "filename,
    // directory name, or volume label syntax is incorrect"), so Windows recording
    // produced only a record.log and no video until this strip. `source.mp4` and
    // `system.wav` are both derived from `out_dir`, so one strip fixes both.
    let out_dir = strip_verbatim_prefix(&out_dir);
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
            // completion; macOS captures it inside ScreenCaptureKit instead.
            let system_audio_worker = if system_audio && !cfg!(target_os = "macos") {
                let sys_out = out_dir.join("system.wav");
                let sys_log = log_path.clone();
                let sys_stop = stop_for_thread.clone();
                std::thread::Builder::new()
                    .name("cut-system-audio".into())
                    .spawn(move || {
                        if let Err(e) = capture_system_audio_until(&sys_out, duration_ms, sys_stop)
                        {
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
            if let Some(worker) = system_audio_worker {
                let _ = worker.join();
            }
            let result: Result<(), record_core::RecordError> = captured.and_then(|out| {
                // On macOS the SCK backend muxed desktop/system audio into source.mp4,
                // split it back out to a sibling system.wav + strip source.mp4 to video-only so the
                // recording matches the cross-platform contract (video track + separate a_system).
                #[cfg(target_os = "macos")]
                if system_audio {
                    split_mac_system_audio(std::path::Path::new(&out.source_video));
                }
                let project = out.into_project();
                let bytes = serde_json::to_vec_pretty(&project).map_err(|e| {
                    record_core::RecordError::new("io", "serialize RecordingProject", e.to_string())
                })?;
                std::fs::write(&project_path, bytes).map_err(|e| {
                    record_core::RecordError::new("io", "write project.json", e.to_string())
                })?;
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

/// On macOS, the SCK backend muxes desktop/system audio into source.mp4 (the stream's
/// `capturesAudio`). Split it back to the cross-platform contract: extract the audio to a sibling
/// `system.wav` (48 kHz stereo → its own `a_system` track in polish) and strip source.mp4 to
/// VIDEO-ONLY (so the mic muxes cleanly at bake and the desktop audio isn't double-counted).
/// Best-effort: if source.mp4 has no audio stream (system audio wasn't requested or SCK produced
/// none), it's a no-op and the recording stays video + mic, exactly as before.
#[cfg(target_os = "macos")]
fn split_mac_system_audio(source_mp4: &Path) {
    use std::process::Command;
    let ffmpeg = cut_media::toolpath::ffmpeg();
    let ffprobe = cut_media::toolpath::ffprobe();
    let has_audio = Command::new(&ffprobe)
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
        .arg(source_mp4)
        .output()
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
        let _ = Command::new(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(source_mp4)
            .args(["-map", "0:a:0", "-ac", "2", "-ar", "48000"])
            .arg(&system_wav)
            .status();
    }
    // 2) strip source.mp4 → VIDEO-ONLY (fast remux, no re-encode) via a temp file then rename.
    let tmp = dir.join("source.video.mp4");
    let stripped = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(source_mp4)
        .args(["-map", "0:v:0", "-c", "copy", "-an"])
        .arg(&tmp)
        .status();
    if matches!(stripped, Ok(s) if s.success()) && tmp.is_file() {
        let _ = std::fs::rename(&tmp, source_mp4);
    }
}

/// The per-OS ffmpeg loopback source for DESKTOP/SYSTEM audio (the game-recording
/// 2nd track). Returns `(format, input)` for `ffmpeg -f <format> -i <input>`.
///
/// Verified on Linux/WSLg: a 440 Hz tone played to the default sink captured at
/// `@DEFAULT_MONITOR@` reads −29.8 dB (signal), vs −85 dB on the plain mic source.
#[cfg(not(windows))] // Windows uses the native WASAPI loopback (capture_system_loopback)
fn system_audio_source() -> (&'static str, &'static str) {
    #[cfg(target_os = "linux")]
    {
        ("pulse", "@DEFAULT_MONITOR@") // default sink's monitor (PulseAudio / PipeWire-pulse)
    }
    #[cfg(target_os = "macos")]
    {
        // Superseded on the recording path: macOS captures desktop/system audio
        // INSIDE the ScreenCaptureKit stream (see `split_mac_system_audio`), so `start_capture`
        // skips this parallel ffmpeg path on Mac. avfoundation `:default` is the MIC, not system
        // audio — kept only as the last-resort source for the (ignored) loopback unit test.
        ("avfoundation", ":default")
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        ("pulse", "@DEFAULT_MONITOR@")
    }
}

#[cfg(all(not(windows), unix))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SystemAudioStopStrategy {
    Sigint(i32),
    Kill,
}

#[cfg(all(not(windows), unix))]
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
#[cfg(not(windows))] // Windows uses the native WASAPI loopback (capture_system_loopback)
pub fn capture_system_audio(out: &Path, duration_ms: u64) -> Result<(), CutError> {
    align_ffmpeg_env();
    let (fmt, input) = system_audio_source();
    let dur = format!("{:.3}", duration_ms as f64 / 1000.0);
    // Use the RESOLVED ffmpeg, not a bare "ffmpeg": on Windows/macOS ffmpeg isn't on
    // PATH (it's in the app-data tools dir), so a bare spawn fails "program not found"
    // — exactly why system-audio produced no system.wav on the Windows rig.
    let status = std::process::Command::new(cut_media::toolpath::ffmpeg())
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
        .arg(out)
        .status()
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!("ffmpeg system-audio spawn failed: {e}"),
                "ffmpeg not found / not on PATH",
            )
        })?;
    if !status.success() {
        return Err(CutError::new(
            error_codes::IO,
            "system-audio capture failed",
            "no desktop-audio loopback on this OS (Linux: PulseAudio/PipeWire monitor; Windows: WASAPI loopback; macOS: ScreenCaptureKit)",
        ));
    }
    Ok(())
}

/// System-audio capture that supports OPEN-ENDED recording. When `duration_ms`
/// is `Some`, this is exactly the bounded [`capture_system_audio`] (ffmpeg `-t`).
/// When `None`, ffmpeg records the loopback with NO `-t`; we poll `stop` and SIGINT
/// the process when it's set so the WAV is finalized cleanly (SIGKILL would truncate
/// the moov/header). Shares the screen capture's stop flag so both end together.
pub fn capture_system_audio_until(
    out: &Path,
    duration_ms: Option<u64>,
    stop: Arc<AtomicBool>,
) -> Result<(), CutError> {
    #[cfg(windows)]
    {
        // Endpoint-independent WASAPI process loopback. Excluding Cut's daemon tree
        // captures the rest of the system mix without opening a physical render driver.
        // No ffmpeg or virtual-audio device; bounded and open-ended captures share this path.
        return record_capture::capture_system_loopback(&out.to_string_lossy(), duration_ms, stop)
            .map(|_| ())
            .map_err(record_err);
    }
    #[cfg(not(windows))]
    {
        // Bounded path → reuse the proven, simple `-t` capture.
        if let Some(ms) = duration_ms {
            return capture_system_audio(out, ms);
        }
        align_ffmpeg_env();
        let (fmt, input) = system_audio_source();
        // Resolved ffmpeg (not bare) — see capture_system_audio Windows path handling.
        let mut child = std::process::Command::new(cut_media::toolpath::ffmpeg())
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
            .arg(out)
            .spawn()
            .map_err(|e| {
                CutError::new(
                    error_codes::IO,
                    format!("ffmpeg system-audio spawn failed: {e}"),
                    "ffmpeg not found / not on PATH",
                )
            })?;
        // Poll the stop flag ~10×/s; also exit early if ffmpeg died on its own (e.g. no
        // loopback device) so we don't spin until stop with a dead child.
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match child.try_wait() {
                Ok(Some(_)) => break, // ffmpeg exited on its own (likely a source error)
                Ok(None) => {}
                Err(_) => break,
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        // SIGINT so ffmpeg flushes + writes the WAV trailer (kill -INT, not SIGKILL).
        #[cfg(unix)]
        {
            match system_audio_stop_strategy(child.id()) {
                SystemAudioStopStrategy::Sigint(pid) => {
                    let _ = std::process::Command::new("kill")
                        .args(["-INT", &pid.to_string()])
                        .status();
                }
                SystemAudioStopStrategy::Kill => {
                    let _ = child.kill();
                }
            }
        }
        #[cfg(not(unix))]
        {
            // No SIGINT on Windows from std; kill() truncates but at least stops. The
            // native WASAPI-loopback path will replace this on Windows.
            let _ = child.kill();
        }
        let status = child.wait().map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!("ffmpeg system-audio wait failed: {e}"),
                "the system-audio ffmpeg process could not be reaped",
            )
        })?;
        // SIGINT makes ffmpeg exit non-zero on some builds even though the WAV is valid;
        // treat a produced, non-empty file as success regardless of the exit code.
        let wrote = out.metadata().map(|m| m.len() > 0).unwrap_or(false);
        if !status.success() && !wrote {
            return Err(CutError::new(
            error_codes::IO,
            "system-audio capture failed",
            "no desktop-audio loopback on this OS (Linux: PulseAudio/PipeWire monitor; Windows: WASAPI loopback; macOS: ScreenCaptureKit)",
        ));
        }
        Ok(())
    }
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

    /// Live capture proof — needs a desktop audio loopback (PulseAudio monitor on
    /// Linux/WSLg), so #[ignore]'d in the normal suite. Run explicitly:
    /// `cargo test -p server --release -- --ignored system_audio_captures_signal`.
    #[test]
    #[cfg(not(windows))] // exercises the ffmpeg pulse-monitor path; Windows uses cpal loopback
    #[ignore = "needs a desktop audio loopback (PulseAudio monitor); run on a desktop/WSLg"]
    fn system_audio_captures_signal() {
        let out = std::env::temp_dir().join(format!("f16_syscap_{}.wav", std::process::id()));
        // Play a 3s tone to the default sink while we capture 2s of the monitor.
        let mut player = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=3",
                "-f",
                "pulse",
                "f16test",
            ])
            .spawn()
            .expect("spawn tone player");
        let r = capture_system_audio(&out, 2000);
        let _ = player.wait();
        assert!(r.is_ok(), "capture failed: {r:?}");
        // Assert the captured WAV carries signal (not silence) via ffmpeg volumedetect.
        let probe = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-i"])
            .arg(&out)
            .args(["-af", "volumedetect", "-f", "null", "-"])
            .output()
            .expect("probe");
        let log = String::from_utf8_lossy(&probe.stderr);
        let mean: Option<f64> = log
            .lines()
            .find_map(|l| l.split("mean_volume:").nth(1))
            .and_then(|s| s.trim().split(' ').next())
            .and_then(|s| s.parse::<f64>().ok());
        let _ = std::fs::remove_file(&out);
        assert!(
            mean.is_some_and(|m| m > -60.0),
            "expected desktop-audio signal > -60 dB, got {mean:?} (is something playing?)"
        );
    }

    #[test]
    fn ready_rollup_requires_core_and_platform_cards() {
        let mk = |name: &str, status: &str| RecordCard {
            name: name.into(),
            status: status.into(),
            detail: String::new(),
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
    fn capture_access_failure_degrades_ready_card_with_recovery_guidance() {
        let mut cards = vec![
            RecordCard {
                name: "ffmpeg".into(),
                status: "ok".into(),
                detail: String::new(),
            },
            RecordCard {
                name: "screen_capture".into(),
                status: "ok".into(),
                detail: "compiled backend".into(),
            },
            RecordCard {
                name: "input_hook".into(),
                status: "ok".into(),
                detail: String::new(),
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
