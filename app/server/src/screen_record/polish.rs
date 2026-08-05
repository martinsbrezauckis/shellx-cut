//! Auto-edit, polish, and export boundary for integrated screen recordings.
//!
//! Capture ownership, readiness, and device lifecycle stay in the parent module.
//! This module owns bounded recorder metadata reads and all transformations from
//! EventTrack/EditPlan data to rendered or raw-export media.

use super::{align_ffmpeg_env, record_err, screen_record_cache_dir};
use crate::dispatch::{parse_args, snapshot};
use crate::output_paths::{fence_output_path, resolve_existing_project_file};
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;

const MAX_EDIT_PLAN_JSON_BYTES: u64 = 32 * 1024 * 1024;
const MAX_EVENT_TRACK_JSON_BYTES: u64 = 64 * 1024 * 1024;

pub(super) fn read_bounded_json(
    path: &Path,
    label: &str,
    max_bytes: u64,
    suggested_action: &str,
) -> Result<Vec<u8>, CutError> {
    let file = std::fs::File::open(path).map_err(|e| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("cannot read the {label} at {}: {e}", path.display()),
            format!("the {label} path must point at an existing JSON file"),
        )
        .with_suggested_action(suggested_action)
    })?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!("cannot read the {label} at {}: {e}", path.display()),
                format!("reading the {label} JSON failed"),
            )
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "the {label} at {} exceeds the {} MiB limit",
                path.display(),
                max_bytes / (1024 * 1024)
            ),
            format!("the {label} JSON is too large to process safely"),
        ));
    }
    Ok(bytes)
}

/// screen_record.autoedit{track, config?, webcam?, studio_events?} — run the
/// recorder's auto-edit engine and optionally patch Recording Studio metadata
/// into the polished plan.
pub(crate) async fn screen_record_autoedit(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        track: String,
        config: Option<Value>,
        webcam: Option<String>,
        studio_events: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (_project, _edl, dir, _at) = snapshot(state).await?;
    let cache = screen_record_cache_dir(&dir)?;
    let track_path = resolve_existing_project_file(
        &dir,
        &a.track,
        "EventTrack",
        "run screen_record.stop first and pass the returned events path",
    )?;
    let stem = track_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("events");
    let out = cache.join(format!("{stem}.plan.json"));
    let config = parse_autoedit_config(a.config)?;
    let mut summary = autoedit(&track_path, &out, &config)?;
    let webcam_path = if let Some(webcam) = a.webcam.as_deref() {
        Some(resolve_existing_project_file(
            &dir,
            webcam,
            "webcam stream",
            "pass the webcam path returned by screen_record.stop",
        )?)
    } else {
        None
    };
    let studio_events_path = if let Some(studio_events) = a.studio_events.as_deref() {
        Some(resolve_existing_project_file(
            &dir,
            studio_events,
            "Studio events",
            "pass the studio_events path returned by screen_record.stop",
        )?)
    } else {
        None
    };
    let mut studio_event_count = 0usize;
    if webcam_path.is_some() || studio_events_path.is_some() {
        let log = if let Some(path) = studio_events_path.as_deref() {
            crate::screen_record_studio::read_studio_events(path)?
        } else {
            crate::screen_record_studio::StudioEventLog::default()
        };
        let mut plan = load_plan(&out)?;
        studio_event_count = crate::screen_record_studio::apply_studio_events_to_plan(
            &mut plan,
            webcam_path.as_ref().map(|path| path.display().to_string()),
            &log,
        )?;
        std::fs::write(
            &out,
            serde_json::to_vec_pretty(&plan).map_err(|e| {
                CutError::new(
                    error_codes::IO,
                    format!("could not serialize the Studio-patched EditPlan: {e}"),
                    "EditPlan serialization failed after applying Studio events",
                )
            })?,
        )
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!(
                    "could not write the Studio-patched EditPlan to {}: {e}",
                    out.display()
                ),
                "writing the plan file failed after applying Studio events",
            )
        })?;
        if studio_event_count > 0 {
            summary = format!("{summary}; {studio_event_count} Studio camera event(s)");
        }
    }
    Ok(VerbResult::ok(json!({
        "plan": out,
        "summary": summary,
        "config": config,
        "webcam": webcam_path,
        "studio_events": studio_events_path,
        "studio_event_count": studio_event_count,
    })))
}

pub(super) fn parse_autoedit_config(
    config: Option<Value>,
) -> Result<record_engine::EngineConfig, CutError> {
    let Some(config) = config else {
        return Ok(record_engine::EngineConfig::default());
    };
    let obj = config.as_object().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "screen_record.autoedit config must be an object",
            "config overrides the recorder EngineConfig by field name",
        )
    })?;
    let allowed = [
        "max_zoom",
        "zoom_in_ms",
        "zoom_hold_min_ms",
        "zoom_out_ms",
        "dwell_merge_ms",
        "stay_zoomed_gap_ms",
        "cursor_window",
        "keycast_gap_ms",
        "keycast_hold_ms",
        "out_fps",
        "idle_threshold_ms",
        "enable_idle",
    ];
    if let Some(key) = obj.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown screen_record.autoedit config key '{key}'"),
            format!("allowed keys: {}", allowed.join(", ")),
        ));
    }
    let mut merged = serde_json::to_value(record_engine::EngineConfig::default()).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "could not serialize default recorder EngineConfig",
            e.to_string(),
        )
    })?;
    if let Value::Object(base) = &mut merged {
        for (key, value) in obj {
            base.insert(key.clone(), value.clone());
        }
    }
    let cfg: record_engine::EngineConfig = serde_json::from_value(merged).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "screen_record.autoedit config has an invalid value",
            e.to_string(),
        )
    })?;
    validate_autoedit_config(&cfg)?;
    Ok(cfg)
}

fn validate_autoedit_config(cfg: &record_engine::EngineConfig) -> Result<(), CutError> {
    let invalid = |field: &str, cause: String| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("screen_record.autoedit config.{field} is invalid"),
            cause,
        )
    };
    if !cfg.max_zoom.is_finite() || !(1.0..=8.0).contains(&cfg.max_zoom) {
        return Err(invalid(
            "max_zoom",
            "max_zoom must be finite and between 1.0 and 8.0".into(),
        ));
    }
    if cfg.cursor_window == 0 || cfg.cursor_window > 121 {
        return Err(invalid(
            "cursor_window",
            "cursor_window must be between 1 and 121 samples".into(),
        ));
    }
    if !cfg.out_fps.is_finite() || !(1.0..=240.0).contains(&cfg.out_fps) {
        return Err(invalid(
            "out_fps",
            "out_fps must be finite and between 1 and 240".into(),
        ));
    }
    for (field, value) in [
        ("zoom_in_ms", cfg.zoom_in_ms),
        ("zoom_hold_min_ms", cfg.zoom_hold_min_ms),
        ("zoom_out_ms", cfg.zoom_out_ms),
        ("dwell_merge_ms", cfg.dwell_merge_ms),
        ("stay_zoomed_gap_ms", cfg.stay_zoomed_gap_ms),
        ("keycast_gap_ms", cfg.keycast_gap_ms),
        ("keycast_hold_ms", cfg.keycast_hold_ms),
        ("idle_threshold_ms", cfg.idle_threshold_ms),
    ] {
        if value > 60 * 60 * 1000 {
            return Err(invalid(
                field,
                "duration tunables must be at most one hour".into(),
            ));
        }
    }
    Ok(())
}

/// 16-hex digest of a plan file's bytes for the polish cache key.
pub(crate) fn plan_cache_tag(plan_path: &Path) -> Result<String, CutError> {
    use sha2::{Digest, Sha256};
    let bytes = read_bounded_json(
        plan_path,
        "EditPlan",
        MAX_EDIT_PLAN_JSON_BYTES,
        "run screen_record.autoedit first to produce the plan",
    )?;
    let hex = format!("{:x}", Sha256::digest(&bytes));
    Ok(hex[..16].to_string())
}

/// screen_record.export{source, plan, path?, format?, gif_fps?, gif_width?} —
/// render the recorder's polished output straight to a fenced file.
pub(crate) async fn screen_record_export(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        source: String,
        plan: String,
        path: Option<String>,
        format: Option<String>,
        gif_fps: Option<u32>,
        gif_width: Option<u32>,
    }
    let a: Args = parse_args(args)?;
    let format = a.format.as_deref().unwrap_or("mp4");
    let (_project, _edl, dir, _at) = snapshot(state).await?;
    let source = resolve_existing_project_file(
        &dir,
        &a.source,
        "recording source",
        "pass the source path returned by screen_record.stop",
    )?;
    let plan = resolve_existing_project_file(
        &dir,
        &a.plan,
        "EditPlan",
        "run screen_record.autoedit first and pass the returned plan path",
    )?;

    match format {
        "mp4" => {
            let out = fence_output_path(&dir, a.path.as_deref(), "exports/recording.mp4")?;
            render(&source, &plan, &out, None)?;
            Ok(VerbResult::ok(json!({"path": out, "format": "mp4"})))
        }
        "gif" => {
            let out = fence_output_path(&dir, a.path.as_deref(), "exports/recording.gif")?;
            let tmp = tempfile::Builder::new()
                .prefix(".cut-recorder-export-")
                .suffix(".mp4")
                .tempfile_in(out.parent().unwrap_or(&dir))
                .map_err(|e| {
                    CutError::new(
                        error_codes::IO,
                        format!("could not create a secure GIF intermediate: {e}"),
                        "creating the recorder export intermediate failed",
                    )
                })?
                .into_temp_path();
            render(&source, &plan, tmp.as_ref(), None)?;
            let fps = a.gif_fps.unwrap_or(15);
            let width = a.gif_width.unwrap_or(720);
            gif(tmp.as_ref(), &out, fps, width)?;
            Ok(VerbResult::ok(json!({"path": out, "format": "gif"})))
        }
        other => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown screen_record.export format '{other}'"),
            "format must be mp4 | gif",
        )),
    }
}

/// Read + deserialize an `EditPlan` JSON (shared by render/export).
fn load_plan(plan_path: &Path) -> Result<record_core::EditPlan, CutError> {
    let bytes = read_bounded_json(
        plan_path,
        "EditPlan",
        MAX_EDIT_PLAN_JSON_BYTES,
        "run screen_record.autoedit first to produce the plan",
    )?;
    serde_json::from_slice(&bytes).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "the EditPlan at {} is not valid JSON: {e}",
                plan_path.display()
            ),
            "the plan file is not a valid Cut recorder EditPlan",
        )
    })
}

/// `screen_record.autoedit`: read the EventTrack JSON at `track_path`, run the
/// auto-edit engine (EventTrack → EditPlan), write the plan JSON to `out_plan`.
/// Returns a one-line human summary. Pure compute — no desktop, no ffmpeg.
pub fn autoedit(
    track_path: &Path,
    out_plan: &Path,
    cfg: &record_engine::EngineConfig,
) -> Result<String, CutError> {
    let bytes = read_bounded_json(
        track_path,
        "EventTrack",
        MAX_EVENT_TRACK_JSON_BYTES,
        "run screen_record.stop to write events.json first",
    )?;
    let events: record_core::EventTrack = serde_json::from_slice(&bytes).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "the EventTrack at {} is not valid JSON: {e}",
                track_path.display()
            ),
            "the events file is not a valid Cut recorder EventTrack",
        )
    })?;
    let plan = record_engine::autoedit(&events, cfg);
    let json = serde_json::to_vec_pretty(&plan).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not serialize the EditPlan: {e}"),
            "EditPlan serialization failed",
        )
    })?;
    std::fs::write(out_plan, json).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!(
                "could not write the EditPlan to {}: {e}",
                out_plan.display()
            ),
            "writing the plan file failed",
        )
    })?;
    Ok(format!(
        "autoedit: {} zoom keyframe(s), {} click effect(s), {} key-cast chip(s)",
        plan.zoom.keys.len(),
        plan.clicks.len(),
        plan.keycast.len(),
    ))
}

/// `screen_record.polish`/`export mp4`: read the EditPlan, render `source` + plan →
/// `out` (zoom/cursor/frame/background polish). `audio` optionally muxes a separate
/// track. Returns the frame count written. Shells to ffmpeg (in-process via the
/// record-render pipeline), no external recorder process.
pub fn render(
    source: &Path,
    plan_path: &Path,
    out: &Path,
    audio: Option<&Path>,
) -> Result<u64, CutError> {
    align_ffmpeg_env();
    let plan = load_plan(plan_path)?;
    let audio_s = audio.map(|p| p.to_string_lossy().into_owned());
    record_render::render_video_audio(
        &source.to_string_lossy(),
        &plan,
        &out.to_string_lossy(),
        audio_s.as_deref(),
    )
    .map_err(record_err)
}

/// `screen_record.export gif`: MP4 → GIF (palettegen/paletteuse via ffmpeg).
pub fn gif(source: &Path, out: &Path, fps: u32, width: u32) -> Result<(), CutError> {
    align_ffmpeg_env();
    record_render::ffmpeg::mp4_to_gif(
        &source.to_string_lossy(),
        &out.to_string_lossy(),
        fps,
        width,
    )
    .map_err(record_err)
}

/// The FAST path for `screen_record.polish{raw:true}` produces an editable clip
/// WITHOUT the (slow) zoom/cursor/frame re-encode. Stream-COPIES the captured video
/// and, when a mic WAV is present, muxes it as the audio track (AAC). ffmpeg `-c:v
/// copy` is near-instant even for long recordings, vs a full re-render — so "stop →
/// editable clip" is fast. The EditPlan is ignored on this path (raw = no polish); the
/// user can run a normal polish afterwards if they want the auto-zoom/cursor pass.
pub fn mux_raw(source: &Path, audio: Option<&Path>, out: &Path) -> Result<(), CutError> {
    align_ffmpeg_env();
    let ffmpeg = cut_media::toolpath::ffmpeg();
    let mut cmd = std::process::Command::new(&ffmpeg);
    cmd.arg("-y").arg("-i").arg(source);
    if let Some(a) = audio {
        cmd.arg("-i").arg(a).args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-shortest",
        ]);
    } else {
        cmd.args(["-map", "0:v:0", "-c", "copy"]);
    }
    cmd.arg(out);
    let output = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| {
            CutError::new(
                error_codes::FFMPEG,
                format!("raw mux could not start ffmpeg: {e}"),
                "ffmpeg must be runnable to mux the raw recording",
            )
        })?;
    if !output.status.success() {
        let tail = String::from_utf8_lossy(&output.stderr);
        let last = tail.lines().last().unwrap_or("ffmpeg failed").to_string();
        return Err(CutError::new(
            error_codes::FFMPEG,
            format!("raw mux failed: {last}"),
            "ffmpeg stream-copy of the raw recording failed",
        ));
    }
    Ok(())
}

/// RAW-CAPTURE finalize helper (the Record panel's "Raw capture" mode:
/// fold a finished capture's streams into ONE standalone, shareable `raw.mp4` with
/// NO autoedit / zoom / cursor / framing pass — the user gets the recording exactly
/// as captured, fast. Stream-COPIES the video (`-c:v copy`, near-instant even for
/// long takes) and combines whatever sound sources were captured:
///   - neither mic nor system → video-only (a silent screen recording),
///   - exactly one of them    → that stream as the audio track (AAC),
///   - BOTH mic AND system    → `amix` into a SINGLE AAC track so the file plays back
///     with voice + desktop/game sound everywhere (a 2-audio-track MP4 would have most
///     players surface only the first track). `normalize=0` keeps each source at its
///     full captured level (a raw SUM — no level scaling/ducking), matching "raw, no
///     post-processing": this is CAPTURE-combining, not the editing pass raw mode skips.
///
/// For SEPARATELY mixable mic / system tracks on the timeline, use the auto-edit mode
/// (`screen_record.polish`, which lays system audio on its own `a_system` track).
/// Mirrors [`mux_raw`]'s ffmpeg invocation/diagnostics; left as its own fn so the
/// polish fast-path's `mux_raw` (video+mic only) stays untouched.
pub fn mux_raw_sources(
    source: &Path,
    mic: Option<&Path>,
    system: Option<&Path>,
    out: &Path,
) -> Result<(), CutError> {
    align_ffmpeg_env();
    let ffmpeg = cut_media::toolpath::ffmpeg();
    let mut cmd = std::process::Command::new(&ffmpeg);
    cmd.arg("-y").arg("-i").arg(source);
    match (mic, system) {
        // No sound sources captured — copy the video stream as-is (silent recording).
        (None, None) => {
            cmd.args(["-map", "0:v:0", "-c", "copy"]);
        }
        // A single captured source — copy video, mux that one stream as AAC audio.
        (Some(a), None) | (None, Some(a)) => {
            cmd.arg("-i").arg(a).args([
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-shortest",
            ]);
        }
        // Both sources — sum them into ONE track. `amix` does NOT resample, and the mic
        // (cpal device rate, often 44.1k) and system.wav (48k) can differ, so resample
        // BOTH to 48k first or amix mis-speeds/fails. normalize=0 keeps each source at its
        // full captured level (a raw SUM — no level scaling).
        (Some(m), Some(s)) => {
            cmd.arg("-i").arg(m).arg("-i").arg(s).args([
                "-filter_complex",
                "[1:a]aresample=48000[m];[2:a]aresample=48000[s];[m][s]amix=inputs=2:duration=longest:normalize=0[a]",
                "-map", "0:v:0", "-map", "[a]", "-c:v", "copy", "-c:a", "aac", "-ar", "48000", "-shortest",
            ]);
        }
    }
    cmd.arg(out);
    let output = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| {
            CutError::new(
                error_codes::FFMPEG,
                format!("raw mux could not start ffmpeg: {e}"),
                "ffmpeg must be runnable to combine the raw recording's sources",
            )
        })?;
    if !output.status.success() {
        let tail = String::from_utf8_lossy(&output.stderr);
        let last = tail.lines().last().unwrap_or("ffmpeg failed").to_string();
        return Err(CutError::new(
            error_codes::FFMPEG,
            format!("raw mux failed: {last}"),
            "ffmpeg combine of the raw recording's sound sources failed",
        ));
    }
    Ok(())
}
