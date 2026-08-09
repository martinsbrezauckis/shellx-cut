use super::*;

/// Resolve an `edit.eq` PRESET name into explicit (high_pass, low_pass, bands) so
/// the op log stores resolved values (replay never depends on the preset table).
/// Bands are JSON `{freq_hz, gain_db, q}` matching `EqBand`. All on the talking-
/// head / podcast wedge: clean the voice, not surgical mastering.
fn resolve_eq_preset(preset: &str) -> Result<(Option<f64>, Option<f64>, Vec<Value>), CutError> {
    let band = |f: f64, g: f64, q: f64| json!({ "freq_hz": f, "gain_db": g, "q": q });
    let (hp, lp, bands): (Option<f64>, Option<f64>, Vec<Value>) = match preset {
        // Broadcast voice: low-cut rumble, scoop boxy "mud", lift presence.
        "voice" => (
            Some(80.0),
            None,
            vec![band(300.0, -2.0, 1.0), band(3000.0, 3.0, 1.0)],
        ),
        // Add low-mid body/warmth.
        "warmth" => (None, None, vec![band(200.0, 3.0, 0.8)]),
        // Just kill subsonic rumble / HVAC.
        "de_rumble" => (Some(100.0), None, vec![]),
        // Telephone band-limit (a deliberate lo-fi effect).
        "phone" => (Some(300.0), Some(3400.0), vec![]),
        // Tame harsh sibilance ("ess"-es).
        "de_ess" => (None, None, vec![band(6500.0, -4.0, 2.5)]),
        // Add high-end "air"/brightness.
        "brighten" => (None, None, vec![band(8000.0, 4.0, 0.7)]),
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown eq preset '{other}'"),
                "preset must be one of: voice, warmth, de_rumble, phone, de_ess, brighten",
            )
            .with_suggested_action(
                "use a preset, or pass explicit high_pass_hz / low_pass_hz / bands instead",
            ));
        }
    };
    Ok((hp, lp, bands))
}

/// edit.eq{clip, preset?|high_pass_hz?/low_pass_hz?/bands?, enabled?=true} —
/// parametric audio EQ (high-pass + peaking bands + low-pass), the audio analog of
/// edit.grade. A `preset` (voice/warmth/de_rumble/phone/de_ess/brighten) is
/// RESOLVED here to explicit high_pass/low_pass/bands so the OP LOG stores resolved
/// values and replay never depends on the preset table (live-only resolution, like
/// edit.animate). Without a preset, raw high_pass_hz/low_pass_hz/bands pass through.
/// enabled:false clears. Core validates the clip is on an audio track + clamps +
/// drops 0 dB bands + identity→None. A core op (replay-safe).
pub(in crate::dispatch) async fn edit_eq(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation; we re-read fields explicitly below
    struct Args {
        clip: String,
        preset: Option<String>,
        high_pass_hz: Option<f64>,
        low_pass_hz: Option<f64>,
        bands: Option<Vec<Value>>,
        enabled: Option<bool>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let enabled = a.enabled.unwrap_or(true);
    let resolved = if !enabled {
        json!({ "clip": a.clip, "enabled": false, "rationale": a.rationale })
    } else if let Some(preset) = a.preset.as_deref() {
        let (hp, lp, bands) = resolve_eq_preset(preset)?;
        json!({ "clip": a.clip, "high_pass_hz": hp, "low_pass_hz": lp, "bands": bands, "enabled": true, "rationale": a.rationale })
    } else {
        // Raw fields (or all-empty = identity, which the core treats as a clear).
        json!({ "clip": a.clip, "high_pass_hz": a.high_pass_hz, "low_pass_hz": a.low_pass_hz, "bands": a.bands.unwrap_or_default(), "enabled": true, "rationale": a.rationale })
    };
    commit_core(state, "edit.eq", resolved, actor).await
}

/// audio.cleanup_voice{clip?, track?, strength?, rationale?} — the one-shot
/// talking-head / podcast VOICE CHAIN as a single auditable, one-step-revertible
/// pass (the agent-first "make my voice publish-ready" button). Applies the
/// cleanup chain to the target audio clip(s):
///   edit.eq{preset:"voice"}  (low-cut rumble + de-mud + presence), THEN
///   edit.effect{[denoise, gate, compressor]}  (afftdn hiss/hum → agate room-tone-
///   between-phrases → acompressor dynamics + auto makeup gain).
/// The render emits these in the FIXED audio-chain order (denoise→gate→compressor
/// →eq→gain→fade, see render.rs), so this is "clean the voice", not surgical
/// mastering. SCOPE: `clip` = one audio clip; `track` = every media clip on that
/// audio track; neither = EVERY media clip on EVERY audio track (clean the whole
/// project's voice in one call). `strength` (light|medium|strong, default medium)
/// scales the denoise/gate/compressor amounts.
///
/// assemble.broll{slots:[{query,at_ms,duration_ms}], source? = "search", provider?,
/// dir?, kind? = "video", track?, rationale?} — the ASSEMBLY-AUTOMATION verb.
/// For each slot it RETRIEVES a clip (search a provider → fetch) and PLACES it on a
/// b-roll track at `at_ms` for `duration_ms`. An ORCHESTRATOR (like audio.cleanup_
/// voice): records NO op of its own — it wraps the work in an auto-checkpoint and
/// dispatches assets.search + assets.fetch + edit.insert as real, replay-safe ops.
/// Stops on the first failing slot and hands back the checkpoint (never a silently
/// half-assembled timeline). `source:"generate"` is reserved for assets.generate.
pub(in crate::dispatch) async fn assemble_broll(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Slot {
        query: String,
        at_ms: u64,
        duration_ms: u64,
    }
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        slots: Vec<Slot>,
        source: Option<String>,
        provider: Option<String>,
        dir: Option<String>,
        kind: Option<String>,
        track: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    if a.slots.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "assemble.broll needs at least one slot",
            "slots: [{query, at_ms, duration_ms}]",
        ));
    }
    let source = a.source.as_deref().unwrap_or("search");
    if source == "generate" {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "source:\"generate\" needs assets.generate, which is not built yet",
            "use source:\"search\" (a provider) for now",
        ));
    }
    if source != "search" {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown source '{source}'"),
            "source is search | generate",
        ));
    }
    let provider = a
        .provider
        .clone()
        .unwrap_or_else(|| "local_folder".to_string());
    let kind = a.kind.clone().unwrap_or_else(|| "video".to_string());

    // Auto-checkpoint: the whole assembly reverts to here in one step.
    let cp = Box::pin(dispatch(
        state,
        "project.checkpoint",
        json!({"name": "before-assemble-broll", "rationale": "auto: before assemble.broll"}),
        actor.clone(),
    ))
    .await;
    if !cp.ok {
        return Ok(cp);
    }
    let checkpoint_id = cp
        .result
        .as_ref()
        .and_then(|r| r.pointer("/checkpoint/id"))
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();

    // Resolve the b-roll track (the given id, or "broll") and ENSURE it exists —
    // create it as a video track if absent. This auto-creates a named target the
    // caller asked for (e.g. track:"broll") rather than failing at insert time.
    let track_id = a.track.clone().unwrap_or_else(|| "broll".to_string());
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
            json!({"kind": "video", "id": track_id, "rationale": "auto: assemble.broll b-roll track"}),
            actor.clone(),
        ))
        .await;
        if !at.ok {
            return Ok(at);
        }
    }

    let revert_hint = format!(
        "a slot failed — project.revert{{to:\"{checkpoint_id}\"}} undoes the partial assembly"
    );
    let mut placed: Vec<Value> = Vec::new();
    for (i, slot) in a.slots.iter().enumerate() {
        let fail = |step: &str, res: &VerbResult| {
            VerbResult::ok(json!({
                "status": "failed",
                "slot": i,
                "query": slot.query,
                "failed_step": step,
                "error": res.error,
                "placed": placed,
                "checkpoint": checkpoint_id,
                "revert_hint": revert_hint,
            }))
        };

        // 1. SEARCH the provider for this slot's query (top hit).
        let sres = Box::pin(dispatch(
            state,
            "assets.search",
            json!({"provider": provider, "q": slot.query, "kind": kind, "limit": 1, "dir": a.dir}),
            actor.clone(),
        ))
        .await;
        if !sres.ok {
            return Ok(fail("assets.search", &sres));
        }
        let hit_id = sres
            .result
            .as_ref()
            .and_then(|r| r.pointer("/hits/0/id"))
            .and_then(|x| x.as_str())
            .map(String::from);
        let Some(hit_id) = hit_id else {
            return Ok(VerbResult::ok(json!({
                "status": "failed",
                "slot": i,
                "query": slot.query,
                "failed_step": "assets.search",
                "error": format!("no '{kind}' result for query '{}'", slot.query),
                "placed": placed,
                "checkpoint": checkpoint_id,
                "revert_hint": revert_hint,
            })));
        };

        // 2. FETCH it as a project asset (downloads + starts the import chain).
        let mut fetch_args = json!({"provider": provider, "id": hit_id, "kind": kind});
        if provider == "local_folder" {
            fetch_args["dir"] = json!(a.dir.clone());
        }
        let fres = Box::pin(dispatch(state, "assets.fetch", fetch_args, actor.clone())).await;
        if !fres.ok {
            return Ok(fail("assets.fetch", &fres));
        }
        let asset_id = fres
            .result
            .as_ref()
            .and_then(|r| r["asset_id"].as_str())
            .map(String::from);
        let Some(asset_id) = asset_id else {
            return Ok(VerbResult::ok(json!({
                "status": "failed",
                "slot": i,
                "query": slot.query,
                "failed_step": "assets.fetch",
                "error": "assets.fetch returned ok without asset_id",
                "placed": placed,
                "checkpoint": checkpoint_id,
                "revert_hint": revert_hint,
            })));
        };
        let import_job = fres
            .result
            .as_ref()
            .and_then(|r| r["job_id"].as_str())
            .map(String::from);

        // 3. WAIT for the import chain (probe) so the asset is insert-ready.
        if let Some(job) = import_job {
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

        // 4. PLACE the first `duration_ms` of the source at `at_ms` (no ripple — a
        // b-roll overlay drops onto its own track without shifting siblings).
        let ires = Box::pin(dispatch(
            state,
            "edit.insert",
            json!({
                "asset": asset_id,
                "track": track_id,
                "at_ms": slot.at_ms,
                "src_range_ms": [0, slot.duration_ms],
                "ripple": false,
                "rationale": format!("auto: assemble.broll slot {i} ('{}')", slot.query),
            }),
            actor.clone(),
        ))
        .await;
        if !ires.ok {
            return Ok(fail("edit.insert", &ires));
        }
        let clip_id = ires
            .result
            .as_ref()
            .and_then(|r| r["clip_id"].as_str())
            .map(String::from);
        placed.push(json!({
            "slot": i,
            "query": slot.query,
            "asset_id": asset_id,
            "clip_id": clip_id,
            "at_ms": slot.at_ms,
            "duration_ms": slot.duration_ms,
        }));
    }

    Ok(VerbResult::ok(json!({
        "status": "ok",
        "track": track_id,
        "slots_filled": placed.len(),
        "placed": placed,
        "checkpoint": checkpoint_id,
        "revert_hint": format!("project.revert{{to:\"{checkpoint_id}\"}} undoes the whole assembly"),
    })))
}

/// ORCHESTRATOR (mirrors comment.apply / autopilot): records NO op of its own — it
/// wraps the sub-verbs in an auto-checkpoint and dispatches edit.eq + edit.effect
/// as real, replay-safe ops (each carries its own rationale + receipt). The
/// checkpoint makes the whole pass revert in ONE step; replay needs no new arm
/// (the sub-ops replay). Stops on the first failing sub-verb and hands back the
/// checkpoint so a half-applied pass is never left silently. LOUDNESS is a
/// render-time concern (render.final{normalize_loudness}), not a per-clip effect,
/// so it is RECOMMENDED in the receipt (`loudness_hint`), never baked here.
pub(in crate::dispatch) async fn audio_cleanup_voice(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: Option<String>,
        track: Option<String>,
        strength: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;

    // strength → (denoise, gate, compressor) amounts. EQ is always the "voice"
    // preset (the chain's tone shaping is fixed; strength scales the dynamics/
    // noise stages only). The sub-verbs clamp to 0..1, so these are intent levels.
    let strength = a.strength.as_deref().unwrap_or("medium");
    let (denoise, gate, comp): (f64, f64, f64) = match strength {
        "light" => (0.30, 0.25, 0.30),
        "medium" => (0.50, 0.40, 0.50),
        "strong" => (0.75, 0.60, 0.70),
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown strength '{other}'"),
                "strength is light|medium|strong (default medium)",
            ))
        }
    };

    // Resolve the target audio MEDIA clips (scoped read — the guard is dropped
    // before any sub-verb dispatch, which takes the write lock). A `track` filter
    // must name an existing AUDIO track; a `clip` filter must resolve to a media
    // clip on an audio track (else the chain would fail mid-apply on edit.eq).
    let targets: Vec<String> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        if let Some(tid) = &a.track {
            let is_audio_track = store
                .project
                .tracks
                .iter()
                .any(|t| &t.id == tid && t.kind == cut_core::TrackKind::Audio);
            if !is_audio_track {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no audio track '{tid}'"),
                    "cleanup_voice targets AUDIO tracks; ids come from project.state",
                ));
            }
        }
        let mut v = Vec::new();
        for t in &store.project.tracks {
            if t.kind != cut_core::TrackKind::Audio {
                continue;
            }
            if let Some(tid) = &a.track {
                if &t.id != tid {
                    continue;
                }
            }
            for c in &t.clips {
                if let cut_core::Clip::Media(m) = c {
                    if let Some(cid) = &a.clip {
                        if &m.id != cid {
                            continue;
                        }
                    }
                    v.push(m.id.clone());
                }
            }
        }
        v
    };
    if targets.is_empty() {
        // Distinguish "named clip not found" from "nothing to clean" for a clear fix.
        if let Some(cid) = &a.clip {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                format!("no audio clip '{cid}'"),
                "clip must be a media clip on an audio track (a video file's audio is a \
                 separate clip on its own audio track); ids come from project.state",
            ));
        }
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "no audio clips to clean",
            "import/place audio (or footage with audio) first; cleanup_voice cleans \
             media clips on audio tracks",
        ));
    }

    // Auto-checkpoint: the whole pass reverts to here in one edit.restore/revert.
    let cp_res = Box::pin(dispatch(
        state,
        "project.checkpoint",
        json!({"name": "before-cleanup-voice", "rationale": format!("auto: before audio.cleanup_voice ({strength})")}),
        actor.clone(),
    ))
    .await;
    if !cp_res.ok {
        return Ok(cp_res);
    }
    let checkpoint_id = cp_res
        .result
        .as_ref()
        .and_then(|r| r["checkpoint"]["id"].as_str())
        .unwrap_or_default()
        .to_string();

    let why = a.rationale.clone().unwrap_or_else(|| {
        format!(
            "audio.cleanup_voice: {strength} voice chain (eq:voice + denoise + gate + compressor)"
        )
    });

    // Apply eq THEN effects to each target clip. Stop on the first failure and hand
    // back the checkpoint to revert the partial pass (never half-applied silently).
    let mut applied: Vec<Value> = Vec::new();
    for clip in &targets {
        let eq_res = Box::pin(dispatch(
            state,
            "edit.eq",
            json!({"clip": clip, "preset": "voice", "rationale": why}),
            actor.clone(),
        ))
        .await;
        applied.push(
            json!({"clip": clip, "step": "eq:voice", "ok": eq_res.ok, "error": eq_res.error}),
        );
        if !eq_res.ok {
            return Ok(VerbResult::ok(json!({
                "status": "failed",
                "failed_step": format!("eq:voice on {clip}"),
                "applied": applied,
                "checkpoint": checkpoint_id,
                "revert_hint": format!("a step failed mid-pass — project.revert{{to:\"{checkpoint_id}\"}} undoes the partial cleanup"),
            })));
        }

        let fx_res = Box::pin(dispatch(
            state,
            "edit.effect",
            json!({
                "clip": clip,
                "effects": [
                    {"type": "denoise", "amount": denoise},
                    {"type": "gate", "amount": gate},
                    {"type": "compressor", "amount": comp},
                ],
                "rationale": why,
            }),
            actor.clone(),
        ))
        .await;
        applied.push(json!({"clip": clip, "step": "denoise+gate+compressor", "ok": fx_res.ok, "error": fx_res.error}));
        if !fx_res.ok {
            return Ok(VerbResult::ok(json!({
                "status": "failed",
                "failed_step": format!("denoise+gate+compressor on {clip}"),
                "applied": applied,
                "checkpoint": checkpoint_id,
                "revert_hint": format!("a step failed mid-pass — project.revert{{to:\"{checkpoint_id}\"}} undoes the partial cleanup"),
            })));
        }
    }

    // before/after diff (checkpoint → tip) — the review artifact for the pass.
    let tip = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store.log.read_all()?.last().map(|o| o.op_id.clone())
    };
    let diff = match tip {
        Some(tip) => {
            Box::pin(dispatch(
                state,
                "project.diff",
                json!({"from": checkpoint_id, "to": tip}),
                actor.clone(),
            ))
            .await
            .result
        }
        None => None,
    };

    Ok(VerbResult::ok(json!({
        "status": "cleaned",
        "strength": strength,
        "clips": targets,
        "chain": ["eq:voice", format!("denoise:{denoise}"), format!("gate:{gate}"), format!("compressor:{comp}")],
        "applied": applied,
        "loudness_hint": {
            "note": "loudness is a render-time setting (not baked into the clip)",
            "recommended_lufs": -16,
            "how": "render.final{normalize_loudness: -16}  — standards: -16 long-form, -14 social, -23 EBU R128",
        },
        "checkpoint": checkpoint_id,
        "diff": diff,
        "revert_hint": format!("project.revert{{to:\"{checkpoint_id}\"}} undoes the whole cleanup pass"),
    })))
}

/// Default linear ramp length for edit.duck windows (each side), ms.
const DUCK_DEFAULT_ATTACK_MS: u64 = 250;

/// Compute auto-duck gain windows for a music track from an against-track's
/// speech (SHARED by edit.duck and audio.add_music). Speech = the complement
/// of the against-track assets' perception silences, mapped through the EDL to
/// timeline time, then merged where the attack ramps would bridge the gap. Each
/// resulting window carries `db` (negative) + `attack_ms`. Errors if any
/// against-track asset has no perception report yet. Returns an empty vec when
/// the against-track is fully silent (no speech to duck under).
///
/// Pure read over `project` + `receipts` so callers can hold a read guard.
fn compute_duck_windows(
    project: &cut_core::Project,
    receipts: &std::path::Path,
    against_track: &str,
    db: f64,
    attack_ms: u64,
) -> Result<Vec<cut_core::GainWindow>, CutError> {
    let track = project.track(against_track).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no duck against_track '{against_track}'"),
            "against_track must be an existing audio track with the speech to duck under",
        )
        .with_suggested_action("check project.state for audio track ids, or omit against_track")
    })?;
    if track.kind != cut_core::TrackKind::Audio {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("duck against_track '{against_track}' is not an audio track"),
            "ducking is computed from speech/silence on an audio track",
        )
        .with_suggested_action("choose an audio track such as a1t, or omit against_track"));
    }
    let edl = cut_core::edl_from_project(project);
    let mut speech: Vec<[u64; 2]> = Vec::new();
    for seg in edl.track_segments(against_track) {
        let (Some(asset_id), Some(src_in), Some(src_out)) =
            (seg.asset.as_deref(), seg.src_in_ms, seg.src_out_ms)
        else {
            continue; // gaps carry no speech
        };
        let report = cut_perception::load_report(receipts, asset_id)?.ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("asset '{asset_id}' on '{against_track}' has no perception report"),
                "duck windows are computed from perception silence facts",
            )
            .with_suggested_action(
                "run media.perception{asset} (or wait for the import chain), then retry",
            )
        })?;
        // Speech (source time) = complement of silences within the segment's
        // source window, mapped to timeline coordinates.
        let mut silences: Vec<[u64; 2]> = report
            .silences
            .iter()
            .map(|s| [s.start_ms, s.end_ms])
            .collect();
        silences.sort();
        let mut cursor = src_in;
        let push = |from: u64, to: u64, speech: &mut Vec<[u64; 2]>| {
            if to > from {
                // Source offsets → timeline through the clip speed (identity at 1.0).
                let t0 = seg.timeline_in_ms + cut_core::src_off_to_tl(from - src_in, seg.speed);
                let t1 = seg.timeline_in_ms + cut_core::src_off_to_tl(to - src_in, seg.speed);
                speech.push([t0, t1]);
            }
        };
        for [s0, s1] in silences {
            let s = s0.clamp(src_in, src_out);
            let e = s1.clamp(src_in, src_out);
            push(cursor, s, &mut speech);
            cursor = cursor.max(e);
        }
        push(cursor, src_out, &mut speech);
    }
    // Merge windows the attack ramps would bridge anyway (fewer windows =
    // shorter expression, no ramp flutter).
    speech.sort();
    let mut merged: Vec<[u64; 2]> = Vec::new();
    for s in speech {
        match merged.last_mut() {
            Some(last) if s[0] <= last[1] + 2 * attack_ms => last[1] = last[1].max(s[1]),
            _ => merged.push(s),
        }
    }
    Ok(merged
        .into_iter()
        .map(|range_ms| cut_core::GainWindow {
            range_ms,
            db,
            attack_ms,
        })
        .collect())
}

/// edit.duck{music_track, against_track, db, attack_ms?} — duck the music
/// track under the against-track's speech. HONEST SEMANTICS (see
/// cut_core::GainWindow): this is WINDOWED GAIN computed NOW — speech spans
/// = the complement of the against-track assets' perception silences,
/// mapped through the current EDL — not a render-time sidechain compressor.
/// The exact db amount is honored and renders stay deterministic; re-run
/// after timeline changes that add/move speech (ripples DO remap windows).
/// The resolved windows are recorded in the op args (self-contained replay).
pub(in crate::dispatch) async fn edit_duck(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        music_track: String,
        against_track: String,
        db: f64,
        attack_ms: Option<u64>,
    }
    let a: Args = parse_args(args.clone())?;
    if a.music_track == a.against_track {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "music_track and against_track must differ",
            "ducking a track against itself would mute all its own content",
        ));
    }
    if a.db >= 0.0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("db must be negative (got {})", a.db),
            "ducking is a reduction, e.g. -18",
        )
        .with_suggested_action("use edit.gain for static level changes"));
    }
    let attack_ms = a.attack_ms.unwrap_or(DUCK_DEFAULT_ATTACK_MS);
    // Compute speech windows on the against-track from perception silences
    // (shared with audio.add_music via compute_duck_windows).
    let windows: Vec<cut_core::GainWindow> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        speech_text::check_scope(&store.project, None, Some(&a.music_track))?;
        speech_text::check_scope(&store.project, None, Some(&a.against_track))?;
        compute_duck_windows(
            &store.project,
            &store.receipts_dir(),
            &a.against_track,
            a.db,
            attack_ms,
        )?
    };
    if windows.is_empty() {
        // No speech on the against-track → honest no-op, no op appended.
        return Ok(VerbResult::ok(json!({
            "track_id": a.music_track,
            "windows_applied": 0,
            "total_ducked_ms": 0,
            "note": format!("no speech found on '{}' (perception reports it fully silent) — nothing to duck against", a.against_track),
        })));
    }
    let mut args = args;
    args["windows"] = serde_json::to_value(&windows)?; // self-contained op record
    commit_core(state, "edit.duck", args, actor).await
}

// ---------------------------------------------------------------------------
// audio.* — music bed with auto-duck + beat markers
// ---------------------------------------------------------------------------

/// Default music-bed level under voice (sits clearly below the VO), dB.
const MUSIC_BED_DEFAULT_GAIN_DB: f64 = -18.0;
/// Default auto-duck depth under detected speech, dB (negative).
const MUSIC_BED_DEFAULT_DUCK_DB: f64 = -15.0;
/// Default dedicated music-bed track id (created on demand).
const MUSIC_BED_TRACK_ID: &str = "music1";

/// `duck` sub-arg of audio.add_music: a config object, or the literal `false`
/// to skip ducking entirely. Default (absent) = auto-duck under the base
/// audio track's speech at MUSIC_BED_DEFAULT_DUCK_DB.
#[derive(Debug, Clone)]
enum DuckArg {
    /// Auto-duck with the given (against_track override, db, attack_ms).
    On {
        against_track: Option<String>,
        db: f64,
        attack_ms: u64,
    },
    /// Explicitly disabled (`duck: false`).
    Off,
}

/// audio.add_music{asset, track?, at_ms?, src_range_ms?, bed_gain_db?, duck?,
/// beat_markers?} — place an imported music asset as a BED under the timeline
/// and (by default) AUTO-DUCK it under the base track's speech. The asset
/// must already be imported (run media.import first; the import chain runs
/// perception → beats + silences are then available). Lowers to real edit ops
/// (add_track? + insert + gain + duck + per-beat add_marker), recorded so
/// replay reproduces them; the op keeps the honest name `audio.add_music`.
///
/// AUTO-DUCK reuses the edit.duck windowed-gain machinery: speech windows are
/// computed NOW from the against-track's perception silences and RECORDED on
/// the lowered edit.duck step (deterministic, auditable — same philosophy as
/// edit.duck). BEAT MARKERS (beat:N, mirroring capture:N) are surfaced from the
/// bed asset's perception BeatGrid mapped to the bed's timeline position, so
/// cut-on-beat receipts become possible later. All defaults overridable.
pub(in crate::dispatch) async fn audio_add_music(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        track: Option<String>,
        at_ms: Option<u64>,
        src_range_ms: Option<[u64; 2]>,
        bed_gain_db: Option<f64>,
        // When true (and no explicit src_range_ms), the bed is trimmed to the
        // current timeline length so it doesn't extend the composition with
        // trailing black/silence. Default false keeps the full-asset behavior
        // (replay-safe: old logs have no field → full asset).
        #[serde(default)]
        fit_to_timeline: bool,
        #[serde(default)]
        beat_markers: Option<bool>,
        // duck: object | false | absent. Parsed manually below (serde can't
        // express "object-or-false" cleanly as a typed field).
        #[serde(default)]
        duck: Value,
    }
    let a: Args = parse_args(args.clone())?;
    let at_ms = a.at_ms.unwrap_or(0);
    let bed_gain_db = a.bed_gain_db.unwrap_or(MUSIC_BED_DEFAULT_GAIN_DB);
    let beat_markers = a.beat_markers.unwrap_or(true);
    // Parse the duck arg: false → Off; absent/object → On with defaults.
    let duck = match &a.duck {
        Value::Bool(false) => DuckArg::Off,
        Value::Bool(true) | Value::Null => DuckArg::On {
            against_track: None,
            db: MUSIC_BED_DEFAULT_DUCK_DB,
            attack_ms: DUCK_DEFAULT_ATTACK_MS,
        },
        Value::Object(m) => DuckArg::On {
            against_track: m
                .get("against_track")
                .and_then(|v| v.as_str())
                .map(String::from),
            db: m
                .get("db")
                .and_then(|v| v.as_f64())
                .unwrap_or(MUSIC_BED_DEFAULT_DUCK_DB),
            attack_ms: m
                .get("attack_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(DUCK_DEFAULT_ATTACK_MS),
        },
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "duck must be an object, false, or omitted",
                format!("got {other}"),
            ))
        }
    };
    if let DuckArg::On { db, .. } = &duck {
        if *db >= 0.0 {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("duck.db must be negative (got {db})"),
                "ducking is a reduction, e.g. -15",
            ));
        }
    }

    // ---- resolve everything under a READ guard (pure planning) ------------
    // The lowered steps are built here; apply_lowered re-runs them on a clone.
    struct Plan {
        steps: Vec<InverseOp>,
        bed_track: String,
        bed_clip_id: String,
        created_track: bool,
        ducked_windows: usize,
        beats_marked: usize,
        bed_dur_ms: u64,
    }
    let plan: Plan = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let project = &store.project;
        // Asset must exist + have a duration (audio assets always do).
        let asset = project.assets.get(&a.asset).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no asset '{}'", a.asset),
                "import the music first: media.import{path}, then audio.add_music{asset}",
            )
        })?;
        let asset_dur = asset
            .probe
            .as_ref()
            .and_then(|p| p.get("duration_ms"))
            .and_then(|v| v.as_u64());
        // Resolve the bed src range (defaults to full asset).
        let (s_in, s_out) = match a.src_range_ms {
            Some([i, o]) if i < o => (i, o),
            Some([i, o]) => {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("src_range_ms [{i}, {o}) is empty or inverted"),
                    "range start must be < end",
                ))
            }
            None => {
                let full = asset_dur.ok_or_else(|| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        format!("asset '{}' has no probed duration yet", a.asset),
                        "wait for the import chain (jobs.status) or pass src_range_ms",
                    )
                })?;
                // fit_to_timeline: clamp the bed to the current timeline length so it
                // sits UNDER the video instead of extending it. Only shortens (a bed
                // shorter than the timeline is left as-is — looping is a v2 feature).
                let tl = cut_core::edl_from_project(project).duration_ms;
                let end = if a.fit_to_timeline && tl > 0 {
                    full.min(tl.saturating_sub(at_ms))
                } else {
                    full
                };
                (0, end.max(1))
            }
        };
        let bed_dur_ms = s_out - s_in;

        // Resolve the bed track + whether we must create it. A bed should be an
        // OVERLAY/extra audio track (so insert defaults to ripple:false — it
        // floats under the VO, nothing shifts). When `track` is omitted, reuse
        // an existing `music1` or create one.
        let (bed_track, created_track) = match &a.track {
            Some(tid) => {
                let t = project.track(tid).ok_or_else(|| {
                    CutError::new(
                        error_codes::NOT_FOUND,
                        format!("no track '{tid}'"),
                        "pass an existing audio track id, or omit `track` to auto-create music1",
                    )
                })?;
                if t.kind != cut_core::TrackKind::Audio {
                    return Err(CutError::new(
                        error_codes::INVALID_ARGS,
                        format!("'{tid}' is not an audio track"),
                        "a music bed lives on an audio track",
                    ));
                }
                (tid.clone(), false)
            }
            None => match project.track(MUSIC_BED_TRACK_ID) {
                Some(_) => (MUSIC_BED_TRACK_ID.to_string(), false),
                None => (MUSIC_BED_TRACK_ID.to_string(), true),
            },
        };

        // The base AUDIO track (the speech track) the bed ducks under: the
        // first audio track with clips, falling back to a1t — NOT the bed track.
        let against_track = match &duck {
            DuckArg::Off => None,
            DuckArg::On {
                against_track: Some(t),
                ..
            } => Some(t.clone()),
            DuckArg::On {
                against_track: None,
                ..
            } => project
                .tracks
                .iter()
                .find(|t| {
                    t.kind == cut_core::TrackKind::Audio && t.id != bed_track && !t.clips.is_empty()
                })
                .map(|t| t.id.clone()),
        };

        // Build the lowered steps in order.
        let mut steps: Vec<InverseOp> = Vec::new();
        if created_track {
            steps.push(InverseOp {
                verb: "edit.add_track".into(),
                args: json!({"kind": "audio", "id": bed_track}),
            });
        }
        // The bed clip id is allocated by core during insert — predict it the
        // same way new_clip_id does (max cN + 1) so the gain step can target it.
        // (create_track adds no clips, so the prediction is stable.)
        let bed_clip_id = cut_core::edit::new_clip_id(project);
        steps.push(InverseOp {
            verb: "edit.insert".into(),
            args: json!({
                "asset": a.asset,
                "track": bed_track,
                "at_ms": at_ms,
                "src_range_ms": [s_in, s_out],
                // Beds float: never ripple the program tracks under them.
                "ripple": false,
            }),
        });
        // Bed level (skip the op when already unity to keep the log minimal).
        if bed_gain_db != 0.0 {
            steps.push(InverseOp {
                verb: "edit.gain".into(),
                args: json!({"clip": bed_clip_id, "db": bed_gain_db}),
            });
        }
        // Auto-duck: compute windows NOW and record them on the lowered
        // edit.duck step (self-contained replay; reuses edit.duck machinery).
        let mut ducked_windows = 0usize;
        if let (DuckArg::On { db, attack_ms, .. }, Some(against)) = (&duck, &against_track) {
            // The bed isn't placed yet, but ducking reads the AGAINST track's
            // speech (the VO), which is already on the timeline — its windows
            // are independent of the bed. Compute against the current project.
            let windows =
                compute_duck_windows(project, &store.receipts_dir(), against, *db, *attack_ms)?;
            ducked_windows = windows.len();
            if !windows.is_empty() {
                steps.push(InverseOp {
                    verb: "edit.duck".into(),
                    args: json!({
                        "music_track": bed_track,
                        "against_track": against,
                        "db": db,
                        "attack_ms": attack_ms,
                        "windows": windows, // resolved — self-contained replay
                    }),
                });
            }
        }
        // Beat markers (beat:N) from the bed asset's perception BeatGrid,
        // mapped from asset (source) time into the bed's timeline position. A
        // beat at source time `b` lands at `at_ms + (b - s_in)` when b is within
        // the placed src range. Only when perception ran with beats.
        let mut beats_marked = 0usize;
        if beat_markers {
            let receipts = store.receipts_dir();
            if let Some(report) = cut_perception::load_report(&receipts, &a.asset)? {
                if let Some(grid) = report.beats {
                    let bpm = grid.bpm;
                    for (i, &b) in grid.beats_ms.iter().enumerate() {
                        if b < s_in || b >= s_out {
                            continue; // outside the placed range
                        }
                        // The bed clip is placed at normal speed (1.0); routed
                        // through the central helper for uniformity. The
                        // authoritative beat alignment is the EDL-based
                        // cut_on_beat receipt, which reads each clip's real speed.
                        let tl = at_ms + cut_core::src_off_to_tl(b - s_in, 1.0);
                        steps.push(InverseOp {
                            verb: "edit.add_marker".into(),
                            args: json!({
                                "at_ms": tl,
                                "label": "beat",
                                "note": format!("beat:{i} bpm:{bpm:.1} asset:{} (audio.add_music)", a.asset),
                            }),
                        });
                        beats_marked += 1;
                    }
                }
            }
        }
        Plan {
            steps,
            bed_track,
            bed_clip_id,
            created_track,
            ducked_windows,
            beats_marked,
            bed_dur_ms,
        }
    };

    // ---- commit the lowered steps as ONE audio.add_music op ----------------
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let include_legacy_inverse = wants_legacy_inverse(&args);
    let extra = vec![effect(
        Some(&plan.bed_track),
        json!({
            "bed_clip": plan.bed_clip_id,
            "bed_gain_db": bed_gain_db,
            "ducked_windows": plan.ducked_windows,
            "beats_marked": plan.beats_marked,
            "created_track": plan.created_track,
        }),
    )];
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("audio.add_music", || {
        store.apply_lowered("audio.add_music", args, actor, rationale, plan.steps, extra)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        json!({
            "track_id": plan.bed_track,
            "bed_clip": plan.bed_clip_id,
            "bed_gain_db": bed_gain_db,
            "bed_duration_ms": plan.bed_dur_ms,
            "ducked_windows": plan.ducked_windows,
            "beats_marked": plan.beats_marked,
            "created_track": plan.created_track,
            "op": op_for_result(&op, include_legacy_inverse),
        }),
        vec![op_id],
    ))
}
