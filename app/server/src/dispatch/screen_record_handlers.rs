use super::*;
use std::io::Read;

// ---------------------------------------------------------------------------
// screen_record.* — integrated Cut recorder orchestration. Low-level recorder
// calls live in screen_record.rs; polish/export orchestration lives here because
// it needs the verb dispatcher.
// ---------------------------------------------------------------------------

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
///   2. read `.capture.json` for `project_path` + `duration_ms` (the recovery
///      marker `start` wrote; `duration_ms` is null for an open-ended capture).
///   3. POLL for `project.json` to exist AND be non-empty (the capture finalized),
///      up to `duration_ms + 15000` ms from the stop call (null → 15000 base),
///      sleeping 300ms between checks. Never appears → a clean CutError.
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
/// cursor_samples, raw_path?, raw_has_mic, raw_has_system}` — `clicks`/`cursor_samples`
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
    crate::screen_record::validate_screen_record_capture_id(&a.capture_id)?;

    // 1. Resolve the capture dir (must exist — it was created by `start`).
    let out_dir = crate::screen_record::screen_record_cache_dir(&dir)?.join(&a.capture_id);
    if !out_dir.is_dir() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("no such capture '{}' ({})", a.capture_id, out_dir.display()),
            "the capture_id does not name a capture dir under <project>/cache/screen_record/",
        )
        .with_suggested_action("pass a capture_id returned by screen_record.start"));
    }

    // SIGNAL the running capture to stop EARLY before we poll for the finalized
    // project.json. For an OPEN-ENDED capture this is what actually ends the recording
    // (the backend's poll loop sees the flag and finalizes); for a duration-bounded
    // one it just ends it a touch sooner. Returns false (and is a no-op) if cutd
    // restarted and lost the in-memory flag — the file poll below still recovers a
    // capture that finalized on its own deadline.
    let _signalled = crate::screen_record::stop_capture(&a.capture_id);

    // 2. Read the recovery marker for the project path + the capture duration.
    let marker = out_dir.join(".capture.json");
    let marker_val: Value = std::fs::read(&marker)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let project_path = marker_val
        .get("project_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| out_dir.join("project.json"));
    let marker_duration = marker_val
        .get("duration_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(15_000);

    // 3. POLL for project.json to exist + be non-empty (capture finalized). Budget
    //    = duration_ms + 15s of finalization slack; 300ms between checks.
    let deadline_ms = marker_duration.saturating_add(15_000);
    let max_iters = (deadline_ms / 300).max(1);
    let mut finalized = false;
    for _ in 0..max_iters {
        if project_path
            .metadata()
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false)
        {
            finalized = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
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
                "capture '{}' did not finalize within {}ms (no project.json at {})",
                a.capture_id,
                deadline_ms,
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
    let source_video_candidate = {
        let raw = Path::new(source_video_raw);
        if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            out_dir.join(raw)
        }
    };
    let source_video_path = fenced_existing_file_under_dir(
        &out_dir,
        &source_video_candidate,
        "capture source_video",
        "discard the incomplete capture or retry recording before requesting screen_record.stop",
    )?;
    let source_video = source_video_path.display().to_string();
    let webcam = if let Some(webcam_video_raw) = proj
        .get("webcam_video")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let raw = Path::new(webcam_video_raw);
        let candidate = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            out_dir.join(raw)
        };
        Some(
            fenced_existing_file_under_dir(
                &out_dir,
                &candidate,
                "capture webcam_video",
                "discard the incomplete camera stream or retry recording before requesting screen_record.stop",
            )?
            .display()
            .to_string(),
        )
    } else {
        None
    };
    let audio = proj.get("audio").and_then(|v| v.as_str()).map(String::from);
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

    // Write the events object to its own file so it feeds screen_record.autoedit.
    let events_path = out_dir.join("events.json");
    std::fs::write(
        &events_path,
        serde_json::to_vec_pretty(&events).unwrap_or_default(),
    )
    .map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not write {}: {e}", events_path.display()),
            "writing <capture_id>/events.json failed",
        )
    })?;
    let events_path_s = events_path.display().to_string();

    let studio_events_path = crate::screen_record_studio::studio_events_path(&out_dir);
    let studio_events = if studio_events_path.is_file() {
        let _ = crate::screen_record_studio::read_studio_events(&studio_events_path)?;
        Some(studio_events_path.display().to_string())
    } else {
        None
    };
    let mic_stream = audio
        .as_deref()
        .and_then(|raw| {
            let p = Path::new(raw);
            let candidate = if p.is_absolute() {
                p.to_path_buf()
            } else {
                out_dir.join(p)
            };
            fenced_existing_file_under_dir(
                &out_dir,
                &candidate,
                "capture audio",
                "discard the incomplete audio stream or retry recording before requesting screen_record.stop",
            )
            .ok()
        })
        .or_else(|| Some(out_dir.join("mic.wav")).filter(|p| p.is_file()))
        .map(|p| p.display().to_string());
    let system_stream = Some(out_dir.join("system.wav"))
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string());
    let raw_streams = json!({
        "screen": source_video.clone(),
        "camera": webcam.clone(),
        "mic": mic_stream.clone(),
        "system": system_stream.clone(),
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
        let source_path = PathBuf::from(&source_video);
        if !source_path.is_file() {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                "capture source_video is missing",
                format!("source_video '{}' is not a readable file", source_video),
            )
            .with_suggested_action(
                "discard the incomplete capture or retry recording before requesting mux_raw",
            ));
        }
        let parent = source_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| out_dir.clone());
        // mic: prefer the project.json `audio` path; else the sibling mic.wav.
        let mic = audio
            .clone()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .or_else(|| Some(parent.join("mic.wav")).filter(|p| p.exists()));
        let system = Some(parent.join("system.wav")).filter(|p| p.exists());
        let raw_out = fence_output_path(&dir, a.raw_path.as_deref(), "exports/raw_recording.mp4")?;
        crate::screen_record::mux_raw_sources(
            &source_path,
            mic.as_deref(),
            system.as_deref(),
            &raw_out,
        )?;
        raw_has_mic = mic.is_some();
        raw_has_system = system.is_some();
        raw_path = Some(raw_out.display().to_string());
    }

    // 5. Optional chain into autoedit → produce the EditPlan from the events track.
    let mut plan: Option<Value> = None;
    if a.autoedit {
        let mut ae_args = json!({"track": events_path_s});
        if let Value::Object(map) = &mut ae_args {
            if let Some(webcam) = &webcam {
                map.insert("webcam".into(), Value::String(webcam.clone()));
            }
            if let Some(studio_events) = &studio_events {
                map.insert("studio_events".into(), Value::String(studio_events.clone()));
            }
        }
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

    let source = resolve_existing_project_file(
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
    let mic_wav = source
        .parent()
        .map(|d| d.join("mic.wav"))
        .filter(|p| p.exists());
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
        if raw {
            // FAST path: stream-copy video + mux mic, no re-encode.
            crate::screen_record::mux_raw(&source, audio, &baked)?;
        } else {
            // IN-PROCESS: bake source + EditPlan (+ mic audio) → polished mp4.
            crate::screen_record::render(&source, &plan_path, &baked, audio)?;
        }
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
    if let Some(sys_wav) = source.parent().map(|p| p.join("system.wav")) {
        if sys_wav.is_file() {
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
                // A dedicated "System" audio track (create if absent), insert at `at_ms`.
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
                    if let Some(err) =
                        screen_record_polish_subverb_error("system audio track creation", &added)
                    {
                        return Err(err);
                    }
                }
                // Trim the system audio to the VIDEO's length. A WASAPI loopback
                // flush / stop-latency can leave system.wav slightly LONGER than the video;
                // inserted full-length the a_system track overran the picture, which pushed
                // the timeline's content end (contentExtentMs) past the last video frame —
                // so preview playback ran on past the picture and the <video> looped to fill
                // the longer timeline and play twice. Clamp the inserted range to
                // min(video_dur, system_dur) so the system track never exceeds the video.
                let sys_range: Option<[u64; 2]> = {
                    let guard = state.project.read().await;
                    guard.as_ref().and_then(|s| {
                        let video_dur = clip_id.as_ref().and_then(|cid| {
                            s.project
                                .tracks
                                .iter()
                                .flat_map(|t| t.clips.iter())
                                .find_map(|c| match c {
                                    cut_core::Clip::Media(m) if &m.id == cid => {
                                        Some(m.src_out_ms.saturating_sub(m.src_in_ms))
                                    }
                                    _ => None,
                                })
                        });
                        let sys_dur = s
                            .project
                            .assets
                            .get(&sys_asset)
                            .and_then(|x| x.probe.as_ref())
                            .and_then(|p| p.get("duration_ms"))
                            .and_then(|d| d.as_u64());
                        match (video_dur, sys_dur) {
                            (Some(vd), Some(sd)) if vd > 0 => Some([0, vd.min(sd)]),
                            _ => None,
                        }
                    })
                };
                let sins_args = match sys_range {
                    Some(r) => {
                        json!({"asset": sys_asset, "track": strack, "at_ms": at_ms, "ripple": false, "src_range_ms": r, "rationale": "auto: screen_record.polish system audio"})
                    }
                    None => {
                        json!({"asset": sys_asset, "track": strack, "at_ms": at_ms, "ripple": false, "rationale": "auto: screen_record.polish system audio"})
                    }
                };
                let sins = Box::pin(dispatch(state, "edit.insert", sins_args, actor.clone())).await;
                if let Some(err) = screen_record_polish_subverb_error("system audio insert", &sins)
                {
                    return Err(err);
                }
                system_clip_id = sins
                    .result
                    .as_ref()
                    .and_then(|r| r["clip_id"].as_str())
                    .map(String::from);
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
        "system_warnings": [],
        "baked": baked,
        "cached": cached,
    })))
}
