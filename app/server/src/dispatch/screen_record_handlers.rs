use super::*;
use std::io::Read;
use std::path::Path;

// ---------------------------------------------------------------------------
// screen_record.* — integrated Cut recorder orchestration. Low-level recorder
// calls live in screen_record.rs; polish/export orchestration lives here because
// it needs the verb dispatcher.
// ---------------------------------------------------------------------------

fn capture_marker_duration(marker: &Path) -> Result<Option<u64>, CutError> {
    match std::fs::symlink_metadata(marker) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(capture_file_error(marker, "inspect capture marker", error)),
        Ok(_) => {
            if !record_recovery::is_plain_regular_file(marker).map_err(|error| {
                capture_file_error(
                    marker,
                    "validate capture marker",
                    std::io::Error::other(error),
                )
            })? {
                return Err(CutError::new(
                    error_codes::IO,
                    format!("could not read capture marker {}", marker.display()),
                    "the capture marker must be a local regular file",
                ));
            }
            let bytes = std::fs::read(marker)
                .map_err(|error| capture_file_error(marker, "read capture marker", error))?;
            let marker: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            Ok(marker.get("duration_ms").and_then(|value| value.as_u64()))
        }
    }
}

/// Read only the durable capture-clock start used to size an open-ended Stop
/// wait. A live writer can be between journal appends, so malformed/torn journal
/// state falls back to the explicit minimum budget rather than blocking Stop.
fn capture_started_unix_ms(capture_dir: &Path) -> Option<u64> {
    record_recovery::read_manifest(capture_dir)
        .ok()
        .map(|manifest| manifest.start.started_unix_ms)
        .filter(|started| *started > 0)
}

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn local_regular_file_nonempty(path: &Path, stage: &str) -> Result<bool, CutError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(capture_file_error(path, stage, error)),
        Ok(metadata) => metadata,
    };
    if !record_recovery::is_plain_regular_file(path)
        .map_err(|error| capture_file_error(path, stage, std::io::Error::other(error)))?
    {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "could not {stage}: {} is not a local regular file",
                path.display()
            ),
            "capture files must remain inside the local capture directory",
        ));
    }
    Ok(metadata.len() > 0)
}

fn capture_file_error(path: &Path, stage: &str, error: std::io::Error) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("could not {stage} at {}: {error}", path.display()),
        "capture files must remain inside the local capture directory",
    )
}

/// screen_record.stop{capture_id, autoedit?, rationale?} — finalize a capture
/// started by `screen_record.start` and surface its artifacts.
///
/// "stop" ACTUALLY STOPS the capture. It first calls
/// `screen_record::stop_capture(capture_id)` to set the running capture's external
/// stop flag — for an OPEN-ENDED capture this is what ends the recording (the
/// backend's poll loop sees the flag and finalizes the mp4 + EventTrack). It then
/// polls for the finalized `project.json` and reads it. (If cutd restarted mid-
/// capture the in-memory flag is gone — the file poll still recovers a capture that
/// finalized on its own deadline.) Pipeline:
///   1. resolve `<cutproj>/cache/screen_record/<capture_id>/`; missing → NOT_FOUND.
///   1b. SIGNAL stop (`stop_capture`) so an open-ended capture ends now.
///   2. read the local `.capture.json` only for `duration_ms`; the project is
///      always the exact local `<capture>/project.json`.
///   3. POLL for that local `project.json` to exist AND be non-empty (the capture finalized),
///      using a finite wait derived from declared or journal-observed capture work
///      (two spans plus 15s, 45s minimum, 15min maximum), sleeping 300ms between
///      checks. Never appears → a clean CutError.
///   4. parse the RecordingProject → extract `source_video`, `audio?`, and the
///      embedded `events` EventTrack; write the events object to
///      `<dir>/events.json` (pretty) so it feeds the existing
///      `screen_record.autoedit {track}` verb.
///   5. if `autoedit:true`, chain `screen_record.autoedit {track: <events.json>}`
///      and include its produced `plan` path.
///   5b. RAW MODE: if `mux_raw:true`, fold the finished capture's
///      streams (video + mic + system) into ONE standalone export file via
///      `screen_record::mux_raw_sources` and return its path — with NO autoedit and
///      NO polish (the Record panel's "Raw capture" mode: just the recording, as
///      captured). Independent of (and composable with, though the UI doesn't) the
///      `autoedit` chain.
///
/// Returns: `{capture_id, project, source, audio, events, plan?, clicks,
/// cursor_samples, cursor_correlation, raw_path?, raw_has_mic, raw_has_system}` — `clicks`/`cursor_samples`
/// are the lengths of the events arrays; the `raw_*` fields are populated only when
/// `mux_raw:true`. Requires an open project. Errors: NO_PROJECT; NOT_FOUND if the
/// capture dir or marker is absent; a CutError if the capture never finalized, the
/// project JSON is malformed, or (raw mode) the ffmpeg combine fails.
pub(super) async fn screen_record_stop(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    const MAX_RECORDING_PROJECT_JSON_BYTES: u64 = 4 * 1024 * 1024;

    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // `rationale` is operator metadata only.
    struct Args {
        capture_id: String,
        #[serde(default)]
        autoedit: bool,
        // RAW MODE: produce a single standalone raw.mp4 (video + captured sound
        // sources, no editing) and return its path. Default false → the unchanged
        // pipeline behaviour. Mutually useful with `autoedit:false` (raw mode skips
        // BOTH autoedit and polish).
        #[serde(default)]
        mux_raw: bool,
        // Optional explicit output path for the standalone raw capture. Omitted =
        // the default export folder (project.set_output_dir) or <project>/exports.
        raw_path: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (_project, _edl, dir, _at) = snapshot(state).await?;
    crate::screen_record::recovery::validate_capture_id(&a.capture_id)?;

    // 1. Resolve the capture dir through the local component-safe anchor.
    let out_dir = crate::screen_record::existing_capture_dir(&dir, &a.capture_id)?;
    let Some(out_dir) = out_dir else {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("no such capture '{}'", a.capture_id),
            "the capture_id does not name a capture dir under <project>/cache/screen_record/",
        )
        .with_suggested_action("pass a capture_id returned by screen_record.start"));
    };

    // SIGNAL the running capture to stop EARLY before we poll for the finalized
    // project.json. For an OPEN-ENDED capture this is what actually ends the recording
    // (the backend's poll loop sees the flag and finalizes); for a duration-bounded
    // one it just ends it a touch sooner. Returns false (and is a no-op) if cutd
    // restarted and lost the in-memory flag — the file poll below still recovers a
    // capture that finalized on its own deadline.
    let _signalled = crate::screen_record::stop_capture(&a.capture_id);

    // 2. Legacy markers may name a project_path, but marker metadata never
    // selects a file to read. The capture-owned local project name is fixed.
    let marker = crate::screen_record::capture_file(&dir, &a.capture_id, ".capture.json")?;
    let marker_duration = capture_marker_duration(&marker)?;
    let project_path = crate::screen_record::capture_file(&dir, &a.capture_id, "project.json")?;

    // 3. POLL for project.json to exist + be non-empty. Finalization has real
    //    proportional work (sparse checkpoint transcodes) plus audio teardown, so
    //    a fixed 30s open-ended wait produced a false failure on valid 4K captures.
    let finalization_budget = crate::screen_record::finalization_budget::finalization_wait_budget(
        marker_duration,
        capture_started_unix_ms(&out_dir),
        unix_ms_now(),
    );
    const POLL_INTERVAL_MS: u64 = 300;
    let max_iters = finalization_budget
        .wait_ms
        .saturating_add(POLL_INTERVAL_MS - 1)
        / POLL_INTERVAL_MS;
    let mut finalized = false;
    for _ in 0..max_iters {
        if local_regular_file_nonempty(&project_path, "inspect capture project")? {
            finalized = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
    if !finalized {
        // Surface the recorder's own diagnostics: start_capture appends a failed
        // in-process capture's error to <dir>/record.log, so a stuck capture (e.g. blocked
        // on a ScreenCast consent dialog) or a crash leaves a trail there. Append the
        // tail so the agent sees WHY without a second round-trip.
        let log_tail = {
            let log = out_dir.join("record.log");
            std::fs::read_to_string(&log)
                .ok()
                .map(|s| {
                    let t = s.trim();
                    if t.is_empty() {
                        "record.log is empty — the capture likely blocked before any output (most often an unanswered ScreenCast consent dialog)".to_string()
                    } else {
                        let tail: String = t.chars().rev().take(600).collect::<String>().chars().rev().collect();
                        format!("record.log tail: …{tail}")
                    }
                })
                .unwrap_or_else(|| format!("no record.log at {}", log.display()))
        };
        return Err(CutError::new(
            error_codes::SIDECAR,
            format!(
                "capture '{}' did not finalize within {}ms derived from {}ms capture work (no project.json at {})",
                a.capture_id,
                finalization_budget.wait_ms,
                finalization_budget.capture_work_ms,
                project_path.display()
            ),
            format!(
                "the recorder did not write project.json under {} — {log_tail}",
                out_dir.display()
            ),
        )
        .with_suggested_action(
            "verify the capture actually started (screen_record.doctor) and the desktop granted ScreenCast consent — for unattended capture the recorder must persist a ScreenCast restore token (PersistMode::ExplicitlyRevoked)",
        ));
    }

    // 4. Parse the RecordingProject; pull source_video, audio, and the events track.
    let project_file = std::fs::File::open(&project_path).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!(
                "could not read the capture project {}: {e}",
                project_path.display()
            ),
            "reading <capture_id>/project.json failed",
        )
    })?;
    let mut proj_bytes = Vec::new();
    project_file
        .take(MAX_RECORDING_PROJECT_JSON_BYTES + 1)
        .read_to_end(&mut proj_bytes)
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!(
                    "could not read the capture project {}: {e}",
                    project_path.display()
                ),
                "reading <capture_id>/project.json failed",
            )
        })?;
    if proj_bytes.len() as u64 > MAX_RECORDING_PROJECT_JSON_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "capture project.json is too large: {} bytes (limit: {} bytes)",
                proj_bytes.len(),
                MAX_RECORDING_PROJECT_JSON_BYTES
            ),
            format!("oversized RecordingProject at {}", project_path.display()),
        )
        .with_suggested_action(
            "discard the capture or regenerate it; screen_record.stop only accepts bounded RecordingProject metadata",
        ));
    }
    let proj: Value = serde_json::from_slice(&proj_bytes).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("capture project.json is not valid JSON: {e}"),
            format!("malformed RecordingProject at {}", project_path.display()),
        )
    })?;
    if !proj.is_object() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "capture project.json must be an object",
            format!("unexpected RecordingProject shape at {}", project_path.display()),
        )
        .with_suggested_action(
            "regenerate the capture; screen_record.stop expects a RecordingProject object with source_video and events",
        ));
    }
    let source_video_raw = proj
        .get("source_video")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "capture project.json is missing source_video",
                "screen_record.stop requires a non-empty RecordingProject.source_video",
            )
            .with_suggested_action("discard the incomplete capture or retry recording")
        })?;
    let webcam_video_raw = proj
        .get("webcam_video")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let audio = proj.get("audio").and_then(|v| v.as_str()).map(String::from);
    let artifacts = crate::screen_record::resolve_stop_artifacts(
        &out_dir,
        source_video_raw,
        webcam_video_raw,
        audio.as_deref(),
    )?;
    let source_video = artifacts.source_video.display().to_string();
    let webcam = artifacts.webcam.clone();
    let events = proj.get("events").cloned().unwrap_or(Value::Null);

    // Count clicks/cursor samples straight off the parsed arrays (lengths, not data).
    let clicks = events
        .get("clicks")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let cursor_samples = events
        .get("cursor")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    // New captures preserve how their click positions were obtained. Older project
    // files cannot prove that provenance, so report it as unavailable rather than
    // inferring exactness from coordinates alone.
    let cursor_correlation = events
        .get("cursor_correlation")
        .cloned()
        .unwrap_or_else(|| {
            json!({
                "source": "legacy_unknown",
                "state": "unavailable",
                "exact_clicks": 0,
                "approximate_clicks": 0,
                "unavailable_clicks": clicks,
                "detail": "capture predates cursor-coordinate provenance"
            })
        });

    // Write the events object to its own file so it feeds screen_record.autoedit.
    let events_path = crate::screen_record::capture_file(&dir, &a.capture_id, "events.json")?;
    record_recovery::replace_synced(
        &events_path,
        &serde_json::to_vec_pretty(&events).unwrap_or_default(),
    )
    .map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not write {}: {e}", events_path.display()),
            "writing <capture_id>/events.json failed",
        )
    })?;
    let events_path_s = events_path.display().to_string();

    let mic_stream = artifacts
        .mic
        .as_ref()
        .map(|path| path.display().to_string());
    let system_stream = artifacts
        .system
        .as_ref()
        .map(|path| path.display().to_string());
    let system_timing = artifacts.system_timing.clone();
    let studio_events = artifacts.studio_events.clone();
    let raw_streams = json!({
        "screen": source_video.clone(),
        "camera": webcam.clone(),
        "mic": mic_stream.clone(),
        "system": system_stream.clone(),
        "system_timing": system_timing,
        "studio_events": studio_events.clone(),
    });

    // 5b. RAW MODE: fold the captured streams into ONE standalone raw.mp4 (video +
    //     mic + system, no autoedit/polish). Independent of the autoedit chain below
    //     — the Record panel's "Raw capture" mode calls stop{autoedit:false,
    //     mux_raw:true} so NOTHING is auto-edited or auto-placed; the UI surfaces the
    //     file and offers to import it as-is. The mic/system WAVs sit beside the
    //     source.mp4 in the capture dir (same names polish resolves: mic.wav /
    //     system.wav).
    let mut raw_path: Option<String> = None;
    let mut raw_has_mic = false;
    let mut raw_has_system = false;
    if a.mux_raw {
        let inputs = artifacts.raw_mux_inputs(&out_dir)?;
        let raw_out = fence_output_path(
            &dir,
            a.raw_path.as_deref(),
            "exports/raw_recording.mp4",
            OutputPathPolicy::MP4,
        )?;
        crate::screen_record::mux_raw_sources(
            &inputs.source_video,
            inputs.mic.as_deref(),
            inputs.system.as_deref(),
            inputs.system_offset_ms,
            &raw_out,
        )?;
        raw_has_mic = inputs.mic.is_some();
        raw_has_system = inputs.system.is_some();
        raw_path = Some(raw_out.display().to_string());
    }

    // 5. Optional chain into autoedit → produce the EditPlan from the events track.
    let mut plan: Option<Value> = None;
    if a.autoedit {
        let ae_args = crate::screen_record::autoedit_args_for_capture(
            events_path_s,
            &proj,
            webcam.as_deref(),
            studio_events.as_deref(),
        )?;
        let ae = Box::pin(dispatch(
            state,
            "screen_record.autoedit",
            ae_args,
            actor.clone(),
        ))
        .await;
        if !ae.ok {
            return Ok(ae);
        }
        plan = ae.result.as_ref().and_then(|r| r.get("plan").cloned());
    }

    Ok(VerbResult::ok(json!({
        "capture_id": a.capture_id,
        "project": project_path,
        "source": source_video,
        "webcam": webcam,
        "audio": audio,
        "events": events_path,
        "studio_events": studio_events,
        "raw_streams": raw_streams,
        "plan": plan,
        "clicks": clicks,
        "cursor_samples": cursor_samples,
        "cursor_correlation": cursor_correlation,
        // RAW MODE: the standalone raw recording (null unless mux_raw:true) + which
        // sound sources it folded in, so the UI can tell the user exactly what was saved.
        "raw_path": raw_path,
        "raw_has_mic": raw_has_mic,
        "raw_has_system": raw_has_system,
    })))
}

pub(super) fn screen_record_polish_subverb_error(step: &str, res: &VerbResult) -> Option<CutError> {
    if res.ok {
        return None;
    }
    let inner = res.error.clone().unwrap_or_else(|| {
        CutError::new(
            error_codes::JOB_FAILED,
            format!("{step} failed"),
            "sub-verb returned ok:false without an error payload",
        )
    });
    let mut err = CutError::new(
        &inner.code,
        format!("{step} failed"),
        format!("{}: {}", inner.message, inner.cause),
    );
    err.clip_id = inner.clip_id;
    err.at_ms = inner.at_ms;
    err.suggested_action = inner.suggested_action.or_else(|| {
        Some("revert the auto-checkpoint or retry screen_record.polish after fixing the failed sub-step".to_string())
    });
    Some(err)
}

/// screen_record.polish{source, plan, track?, at_ms?, rationale?} bakes the
/// recorder's polished MP4 (source + EditPlan → zoom/
/// speed/cursor polish) and drop it onto the timeline. An ORCHESTRATOR (mirrors
/// assemble.broll / import.otio): records NO op of its own — it wraps the work in
/// an auto-checkpoint and dispatches media.import + edit.add_track + edit.insert
/// as real, replay-safe ops.
///
/// Pipeline:
///   1. CONTENT-ADDRESSED BAKE (the matte pattern): key = `hash_file(source) +
///      "_" + <16-hex of the plan bytes>`; baked mp4 =
///      `<cutproj>/cache/screen_record/<key>.mp4`. If it exists → `cached:true`,
///      skip; else render through the integrated recorder crates (bake-on-demand).
///   2. media.import{path: baked} → asset_id + job_id; poll jobs.status until
///      done/failed (≤60×300ms, like assemble.broll) so the asset is insert-ready.
///   3. ensure the target track exists (default `"v1"`; an explicit `track` that's
///      absent is created as a video track), then edit.insert{asset, track, at_ms
///      (default 0), ripple:false}.
///
/// Returns: `{clip_id, asset_id, baked: <path>, cached: bool}`. Requires an open
/// project. Errors: NOT_FOUND / FFMPEG from the bake; any sub-verb failure is
/// returned as that sub-verb's envelope (and the auto-checkpoint lets it revert).
pub(super) async fn screen_record_polish(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // rationale rides into the auto-checkpoint note
    struct Args {
        source: String,
        plan: String,
        track: Option<String>,
        at_ms: Option<u64>,
        rationale: Option<String>,
        // Skip the slow zoom/cursor re-render — stream-copy the raw capture (+
        // mux the mic) for a fast "stop → editable clip". The plan is still required
        // (kept for cache-keying + a later real polish) but is NOT applied on this path.
        raw: Option<bool>,
    }
    let a: Args = parse_args(args)?;
    let (_project, _edl, dir, _at) = snapshot(state).await?;

    let source = crate::screen_record::plain_existing_file_under_project(
        &dir,
        &a.source,
        "recording source",
        "pass the source path returned by screen_record.stop",
    )?;

    // 1. Content-addressed bake (source content + plan bytes → one cache slot).
    let cache = crate::screen_record::screen_record_cache_dir(&dir)?;
    let src_hash = cut_core::hash_file(&source)?;
    let plan_path = resolve_existing_project_file(
        &dir,
        &a.plan,
        "EditPlan",
        "run screen_record.autoedit first and pass the returned plan path",
    )?;
    let plan_tag = crate::screen_record::plan_cache_tag(&plan_path)?;
    // The capture records the MIC to a SEPARATE `mic.wav` beside the
    // video `source.mp4` (WGC/ScreenCaptureKit capture video only). The bake MUST mux
    // it back in, or the polished clip is silent — which is exactly what shipped
    // ("replay has no audio track"). Find the mic WAV next to the source and pass it
    // to the render's audio arg. (System/desktop audio = `system.wav` is a separate
    // second-track feature; the mic is the must-have voice track.)
    let source_dir = source.parent().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "recording source has no parent directory",
            "the recording source must remain inside a project-local directory",
        )
    })?;
    let mic_wav = crate::screen_record::optional_plain_file_in_dir(
        source_dir,
        "mic.wav",
        "recording microphone audio",
        "remove the unsafe capture artifact and retry screen_record.polish",
    )?;
    let system_wav = crate::screen_record::optional_plain_file_in_dir(
        source_dir,
        "system.wav",
        "recording system audio",
        "remove the unsafe capture artifact and retry screen_record.polish",
    )?;
    let _system_timing = crate::screen_record::optional_plain_file_in_dir(
        source_dir,
        crate::screen_record::system_audio::SYSTEM_AUDIO_TIMING_FILE,
        "recording system-audio timing",
        "remove the unsafe capture artifact and retry screen_record.polish",
    )?;
    let audio = mic_wav.as_deref();
    // hash_file returns "sha256:<hex>"; ':' is fine in a filename on every target
    // we ship, but keep the key tidy by replacing it. The `av`/`v` tag keeps a
    // prior video-only bake from being reused now that audio is muxed.
    // raw:true → fast stream-copy (no re-render). Separate cache tag so a raw
    // bake is never served where a polished one was asked for, and vice-versa.
    let raw = a.raw.unwrap_or(false);
    let av_tag = match (raw, audio.is_some()) {
        (true, true) => "raw_av",
        (true, false) => "raw_v",
        (false, true) => "av",
        (false, false) => "v",
    };
    let key = format!("{}_{plan_tag}_{av_tag}", src_hash.replace(':', "_"));
    let baked = cache.join(format!("{key}.mp4"));
    let baked_s = baked.display().to_string();
    let cached = baked.exists();
    if !cached {
        // Rendering is CPU/ffmpeg work. Keep the verb's established synchronous
        // result shape, but never occupy an async worker while it runs. It shares
        // the export limiter so an auto-polish cannot saturate the recorder path.
        let render_source = source.clone();
        let render_plan = plan_path.clone();
        let render_baked = baked.clone();
        let render_audio = audio.map(std::path::Path::to_path_buf);
        state
            .jobs
            .with_limit(
                "screen_record.export",
                1,
                run_blocking_cancellable("screen_record.polish", move |cancellation| {
                    let child_cancellation = cancellation.clone();
                    let control = record_render::ffmpeg::ProcessControl::bounded(
                        std::time::Duration::from_secs(30 * 60),
                        move || child_cancellation.is_cancelled(),
                    );
                    if raw {
                        crate::screen_record::mux_raw_with_control(
                            &render_source,
                            render_audio.as_deref(),
                            &render_baked,
                            &control,
                        )
                    } else {
                        crate::screen_record::render_with_control(
                            &render_source,
                            &render_plan,
                            &render_baked,
                            render_audio.as_deref(),
                            &control,
                        )
                        .map(|_| ())
                    }
                }),
            )
            .await?;
    }
    let _ = &baked_s; // (baked path string kept for the result payload below)

    // Auto-checkpoint: the whole polish (import + insert) reverts in one step.
    let cp = Box::pin(dispatch(
        state,
        "project.checkpoint",
        json!({"name": "before-screen-record-polish", "rationale": "auto: before screen_record.polish"}),
        actor.clone(),
    ))
    .await;
    if !cp.ok {
        return Ok(cp);
    }

    // 2. media.import the baked mp4 → asset_id + job_id; wait for the chain.
    let imp = Box::pin(dispatch(
        state,
        "media.import",
        json!({"path": baked_s, "rationale": "auto: screen_record.polish baked recording"}),
        actor.clone(),
    ))
    .await;
    if !imp.ok {
        return Ok(imp);
    }
    let asset_id = imp
        .result
        .as_ref()
        .and_then(|r| r["asset_id"].as_str())
        .map(String::from)
        .unwrap_or_default();
    if let Some(job) = imp.result.as_ref().and_then(|r| r["job_id"].as_str()) {
        let job = job.to_string();
        for _ in 0..60 {
            let js = Box::pin(dispatch(
                state,
                "jobs.status",
                json!({"job_id": job}),
                actor.clone(),
            ))
            .await;
            let st = js
                .result
                .as_ref()
                .and_then(|r| r["state"].as_str())
                .unwrap_or("");
            if st == "done" || st == "failed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    // 3. Place the baked clip EXACTLY ONCE. `media.import` AUTO-PLACES the
    //    first clip into an empty timeline ("first import becomes the timeline") — so
    //    a fresh recording's import already put the clip on v1@0 full-length. Doing our
    //    own `edit.insert` on top of that produced a DUPLICATE: a second clip appended
    //    side-by-side (the recording played twice) and a timeline TWICE the true length
    //    (playback ran past the content into the duplicated tail = the "black screen").
    //    Fix: if the import already placed this asset, ADOPT that clip; otherwise (the
    //    timeline already had content, so no auto-place) insert it explicitly.
    let track_id = a.track.clone().unwrap_or_else(|| "v1".to_string());
    let at_ms = a.at_ms.unwrap_or(0);
    let auto_placed = {
        let guard = state.project.read().await;
        guard.as_ref().and_then(|s| {
            s.project
                .tracks
                .iter()
                .flat_map(|t| t.clips.iter())
                .find_map(|c| match c {
                    cut_core::Clip::Media(m) if m.asset == asset_id => Some(m.id.clone()),
                    _ => None,
                })
        })
    };
    let clip_id = if let Some(cid) = auto_placed {
        // Already on the timeline via media.import auto-place — do NOT insert again.
        Some(cid)
    } else {
        // Timeline already had content → no auto-place → place it ourselves.
        let exists = {
            let guard = state.project.read().await;
            guard
                .as_ref()
                .map(|s| s.project.tracks.iter().any(|t| t.id == track_id))
                .unwrap_or(false)
        };
        if !exists {
            let at = Box::pin(dispatch(
                state,
                "edit.add_track",
                json!({"kind": "video", "id": track_id, "rationale": "auto: screen_record.polish target track"}),
                actor.clone(),
            ))
            .await;
            if !at.ok {
                return Ok(at);
            }
        }
        let ires = Box::pin(dispatch(
            state,
            "edit.insert",
            json!({
                "asset": asset_id,
                "track": track_id,
                "at_ms": at_ms,
                "ripple": false,
                "rationale": a.rationale.clone().unwrap_or_else(|| "auto: screen_record.polish insert".to_string()),
            }),
            actor.clone(),
        ))
        .await;
        if !ires.ok {
            return Ok(ires);
        }
        ires.result
            .as_ref()
            .and_then(|r| r["clip_id"].as_str())
            .map(String::from)
    };

    // Place the recording's DESKTOP/SYSTEM audio (captured to <capture>/system.wav by
    // screen_record.start{system_audio:true}) on its OWN audio track, so the game/app sound
    // and the mic are independently mixable. (The video's muxed mic already lands on its own
    // track via the linked-A/V import.) Best-effort: no system.wav → mic-only, as before.
    let mut system_clip_id: Option<String> = None;
    let mut system_warnings = Vec::<String>::new();
    if let Some(sys_wav) = system_wav.as_deref() {
        let timing = crate::screen_record::system_audio::read_timing(source_dir)?;
        if timing
            .as_ref()
            .is_some_and(|timing| timing.first_packet_offset_ms.is_none())
        {
            system_warnings.push(
                "system audio delivered no proven packets; no system-audio clip was inserted"
                    .into(),
            );
        } else {
            let simp = Box::pin(dispatch(
                state,
                "media.import",
                json!({"path": sys_wav.display().to_string(), "rationale": "auto: screen_record.polish system audio"}),
                actor.clone(),
            ))
            .await;
            if let Some(err) = screen_record_polish_subverb_error("system audio import", &simp) {
                return Err(err);
            } else if let Some(sys_asset) = simp
                .result
                .as_ref()
                .and_then(|r| r["asset_id"].as_str())
                .map(String::from)
            {
                // Wait for the import to finish probing (duration) — edit.insert needs it,
                // same as the video import above.
                if let Some(job) = simp.result.as_ref().and_then(|r| r["job_id"].as_str()) {
                    let job = job.to_string();
                    let mut failed: Option<String> = None;
                    for _ in 0..60 {
                        let js = Box::pin(dispatch(
                            state,
                            "jobs.status",
                            json!({"job_id": job}),
                            actor.clone(),
                        ))
                        .await;
                        let st = js
                            .result
                            .as_ref()
                            .and_then(|r| r["state"].as_str())
                            .unwrap_or("");
                        if st == "done" || st == "failed" {
                            if st == "failed" {
                                failed = Some(
                                    js.result
                                        .as_ref()
                                        .and_then(|r| r.get("error"))
                                        .map(|e| {
                                            let msg = e
                                                .get("message")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("job failed");
                                            let cause = e
                                                .get("cause")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            if cause.is_empty() {
                                                msg.to_string()
                                            } else {
                                                format!("{msg}: {cause}")
                                            }
                                        })
                                        .unwrap_or_else(|| {
                                            "system audio import job failed".to_string()
                                        }),
                                );
                            }
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                    if let Some(cause) = failed {
                        return Err(CutError::new(
                            error_codes::JOB_FAILED,
                            "system audio import job failed",
                            cause,
                        )
                        .with_suggested_action(
                            "check system.wav is readable audio, then retry screen_record.polish",
                        ));
                    }
                }
                let placement = {
                    let guard = state.project.read().await;
                    guard.as_ref().map(|s| {
                        let video_duration_ms = clip_id.as_ref().and_then(|cid| {
                            s.project
                                .tracks
                                .iter()
                                .flat_map(|track| track.clips.iter())
                                .find_map(|clip| match clip {
                                    cut_core::Clip::Media(media) if &media.id == cid => {
                                        Some(media.src_out_ms.saturating_sub(media.src_in_ms))
                                    }
                                    _ => None,
                                })
                        });
                        let system_duration_ms = s
                            .project
                            .assets
                            .get(&sys_asset)
                            .and_then(|asset| asset.probe.as_ref())
                            .and_then(|probe| probe.get("duration_ms"))
                            .and_then(|duration| duration.as_u64());
                        crate::screen_record::system_audio::plan_placement(
                            at_ms,
                            video_duration_ms,
                            system_duration_ms,
                            timing.as_ref(),
                        )
                    })
                }
                .unwrap_or_else(|| {
                    crate::screen_record::system_audio::SystemAudioPlacement::Skip {
                        warning: "no open project remained while placing system audio".into(),
                    }
                });
                let placement = match placement {
                    crate::screen_record::system_audio::SystemAudioPlacement::Insert {
                        at_ms,
                        source_duration_ms,
                    } => Some((at_ms, source_duration_ms)),
                    crate::screen_record::system_audio::SystemAudioPlacement::Skip { warning } => {
                        system_warnings.push(warning);
                        None
                    }
                };
                if let Some((system_at_ms, source_duration_ms)) = placement {
                    // A dedicated "System" audio track (create if absent), placed at the
                    // first real system packet and clipped to the remaining video span.
                    let strack = "a_system".to_string();
                    let sexists = {
                        let guard = state.project.read().await;
                        guard
                            .as_ref()
                            .map(|s| s.project.tracks.iter().any(|t| t.id == strack))
                            .unwrap_or(false)
                    };
                    if !sexists {
                        let added = Box::pin(dispatch(
                        state,
                        "edit.add_track",
                        json!({"kind": "audio", "id": strack, "rationale": "auto: screen_record.polish system-audio track"}),
                        actor.clone(),
                    ))
                    .await;
                        if let Some(err) = screen_record_polish_subverb_error(
                            "system audio track creation",
                            &added,
                        ) {
                            return Err(err);
                        }
                    }
                    let sins_args = json!({
                        "asset": sys_asset,
                        "track": strack,
                        "at_ms": system_at_ms,
                        "ripple": false,
                        "src_range_ms": [0, source_duration_ms],
                        "rationale": "auto: screen_record.polish system audio",
                    });
                    let sins =
                        Box::pin(dispatch(state, "edit.insert", sins_args, actor.clone())).await;
                    if let Some(err) =
                        screen_record_polish_subverb_error("system audio insert", &sins)
                    {
                        return Err(err);
                    }
                    system_clip_id = sins
                        .result
                        .as_ref()
                        .and_then(|r| r["clip_id"].as_str())
                        .map(String::from);
                }
            } else {
                return Err(CutError::new(
                    error_codes::JOB_FAILED,
                    "system audio import returned no asset id",
                    "media.import succeeded but did not return result.asset_id",
                )
                .with_suggested_action(
                    "retry screen_record.polish; if it repeats, inspect media.import for the system.wav",
                ));
            }
        }
    }

    Ok(VerbResult::ok(json!({
        "clip_id": clip_id,
        "asset_id": asset_id,
        "system_clip_id": system_clip_id,
        "system_warnings": system_warnings,
        "baked": baked,
        "cached": cached,
    })))
}
