use super::*;

/// edit.multicam_sync{clips, reference?, max_offset_ms?, rationale?} — MULTICAM
/// AUDIO SYNC. Aligns ≥2 clips of the SAME event by cross-correlating their
/// audio energy envelopes: two cameras hear the same sound, so the lag that best
/// aligns their audio is the time offset between the clips. MEASUREMENT (non-mutating,
/// like edit.track): returns each clip's `offset_ms` vs the reference (+ a confidence
/// score); to sync, the agent places/trims each clip by its offset. Needs audio on
/// every clip.
pub(in crate::dispatch) async fn edit_multicam_sync(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clips: Vec<String>,
        reference: Option<String>,
        max_offset_ms: Option<u64>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    if a.clips.len() < 2 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "edit.multicam_sync needs at least 2 clips",
            "pass clips: [clip_id, clip_id, …] of the same event",
        ));
    }
    let reference = a.reference.clone().unwrap_or_else(|| a.clips[0].clone());
    if !a.clips.contains(&reference) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("reference '{reference}' is not in clips"),
            "reference must be one of the clips (default = the first)",
        ));
    }
    if a.max_offset_ms.is_some_and(|ms| ms < 100) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "max_offset_ms must be at least 100",
            "schema/verbs.json declares max_offset_ms minimum: 100",
        )
        .with_suggested_action(
            "omit max_offset_ms for the 15000ms default or pass a value >= 100",
        ));
    }

    // Resolve each clip → its source path (clips must exist + be media clips).
    let (project, _edl, _dir, _at) = snapshot(state).await?;
    let mut paths: Vec<(String, PathBuf)> = Vec::new();
    for cid in &a.clips {
        let (track_id, idx) = project
            .find_clip(cid)
            .map(|(t, i)| (t.to_string(), i))
            .ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no clip '{cid}'"),
                    "clip ids come from project.state",
                )
                .with_clip(cid)
            })?;
        let cut_core::Clip::Media(c) = &project.track(&track_id).unwrap().clips[idx] else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("'{cid}' is not a media clip"),
                "multicam sync needs media clips with audio",
            )
            .with_clip(cid));
        };
        let (p, _hash) = asset_info(state, &c.asset.clone()).await?;
        paths.push((cid.clone(), p));
    }
    let ref_pos = a.clips.iter().position(|c| c == &reference).unwrap();
    let max_lag = (a.max_offset_ms.unwrap_or(15_000) as usize) * cut_media::sync::ENV_HZ / 1000;

    let offsets = run_blocking("edit.multicam_sync", move || {
        // Envelope every clip (one ffmpeg decode each).
        let envs: Vec<Vec<f32>> = paths
            .iter()
            .map(|(_, p)| cut_media::sync::clip_envelope(p))
            .collect::<Result<_, _>>()?;
        let ref_env = &envs[ref_pos];
        let mut out: Vec<Value> = Vec::new();
        for (i, (cid, _)) in paths.iter().enumerate() {
            if i == ref_pos {
                out.push(json!({"clip": cid, "offset_ms": 0, "score": 1.0, "reference": true}));
                continue;
            }
            let (lag, score) = cut_media::sync::best_lag(ref_env, &envs[i], max_lag)
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        format!("could not find a reliable audio sync match for clip '{cid}'"),
                        "the clip may be silent, too short, outside max_offset_ms, or not from the same event",
                    )
                    .with_clip(cid)
                    .with_suggested_action(
                        "use clips with overlapping audio, increase max_offset_ms, or sync manually",
                    )
                })?;
            out.push(json!({
                "clip": cid,
                "offset_ms": cut_media::sync::lag_to_ms(lag),
                "score": (score * 1000.0).round() / 1000.0,
            }));
        }
        Ok::<_, CutError>(out)
    })
    .await?;

    Ok(VerbResult::ok(json!({
        "reference": reference,
        "env_hz": cut_media::sync::ENV_HZ,
        "offsets": offsets,
        "sync_hint": "offset_ms = how much LATER this clip's audio is than the reference; to align, place the clip offset_ms earlier (or trim offset_ms from its start). A low score (<~0.3) = weak/uncertain match.",
    })))
}

// ---------------------------------------------------------------------------
// edit.multicam_switch — automatic active-speaker camera switching
// ---------------------------------------------------------------------------

/// Default minimum shot length, ms — the anti-flicker / no-sub-second-cut floor.
const MULTICAM_DEFAULT_MIN_SHOT_MS: u64 = 1500;
/// Active-speaker analysis window, ms: the sub-shot granularity at which the loudest
/// camera is sampled. v1 fixed (min_shot_ms is the user knob); finer than min_shot so
/// a switch lands within ~one window of the real crossover.
const MULTICAM_ANALYSIS_WINDOW_MS: u64 = 250;
/// Energy hysteresis (loudness units): a camera must lead the on-screen one by MORE
/// than this to steal the shot, so two near-equal levels (the same room ambient on two
/// mics) don't flicker between angles.
const MULTICAM_HYSTERESIS_LU: f64 = 1.0;
/// Reserved output track for `edit.multicam_switch`. The old public name `program`
/// is deliberately not reused because a user may already have a real track with
/// that id.
const MULTICAM_PROGRAM_TRACK: &str = "__mc_program";
const LEGACY_MULTICAM_PROGRAM_TRACK: &str = "program";

/// One media segment of a camera track, with its timeline↔source mapping (linear, so
/// it is speed-agnostic: `timeline_out-timeline_in` already encodes the clip speed).
struct McSeg {
    asset: String,
    tl_in: u64,
    tl_out: u64,
    src_in: u64,
    src_out: u64,
}

/// One resolved camera angle: its track id, its media segments, its covered timeline
/// span, and its timeline-mapped audio-energy envelope (ascending by `t_ms`).
struct McCam {
    track_id: String,
    segs: Vec<McSeg>,
    cov_start: u64,
    cov_end: u64,
    env_t: Vec<u64>,
    env_e: Vec<f64>,
}

/// Nearest energy sample to `center` in a camera's (sorted) envelope. The grid window
/// is finer than the loudness cadence, so nearest-by-time is the honest reading; the
/// caller only asks within the camera's covered span, where samples exist.
fn mc_nearest_energy(cam: &McCam, center: u64) -> f64 {
    if cam.env_t.is_empty() {
        return f64::NEG_INFINITY;
    }
    let i = cam.env_t.partition_point(|&t| t < center);
    let cand = [i.saturating_sub(1), i.min(cam.env_t.len() - 1)];
    let mut best = f64::NEG_INFINITY;
    let mut best_d = u64::MAX;
    for &k in &cand {
        let d = cam.env_t[k].abs_diff(center);
        if d < best_d {
            best_d = d;
            best = cam.env_e[k];
        }
    }
    best
}

/// edit.multicam_switch{tracks?, min_shot_ms?, reference_track?, rationale?} — AUTO
/// Active-speaker multicam switching. Given at least two audio-synced camera angles of one
/// scene (each on its own VIDEO track, time-aligned on the timeline — run
/// edit.multicam_sync + place the clips first), cuts the program to whichever camera
/// is the ACTIVE SPEAKER (loudest) over time — the interview / podcast multicam edit.
///
/// ALGORITHM: per `MULTICAM_ANALYSIS_WINDOW_MS` window across the cameras' OVERLAPPING
/// timeline span, each camera's audio energy is read from its perception
/// `Loudness.windows` envelope (the SAME audio facts edit.duck / edit.auto_zoom use),
/// mapped to timeline coordinates; the loudest camera wins, with a small energy
/// HYSTERESIS so near-equal levels don't flicker. Adjacent same-camera windows merge
/// into shots; any shot shorter than `min_shot_ms` (default 1500 ms) is dissolved, so
/// there are no sub-second cuts (cut_core::multicam::switch_shots — pure + unit-tested).
///
/// REPRESENTATION (replay-safe): an ORCHESTRATOR like assemble.broll — records NO op
/// of its own. It wraps the build in an auto-checkpoint, (re)creates a single
/// `program` VIDEO track ON TOP (later video tracks composite over earlier ones, so a
/// full-frame opaque clip occludes the angles beneath it → exactly one angle shows),
/// and LOWERS each shot to a plain `edit.insert` of the active angle's segment at the
/// sync-correct source offset. The committed ops are ordinary inserts, so undo / diff
/// / replay already handle them. AUDIO is unchanged: video tracks contribute no audio
/// to the render (it mixes from audio tracks only), so the continuous program audio
/// stays whatever audio track is present — this switches the PICTURE, not the sound
/// (standard multicam practice; no per-cut audio jumps).
///
/// HONEST receipt `{shots:[{start_ms,end_ms,camera,energy}], switches, tracks,
/// min_shot_ms, program_track, span_ms}`. <2 camera angles, an angle with no audio
/// energy, or no overlapping span → an actionable error. One dominant speaker → one
/// shot / no switches (clean, not an error). v1 LIMITS: energy-only (no face /
/// diarization — the loudest mic wins, which is wrong if an off-camera voice is loud);
/// 250-ms window granularity; assumes the angles are already time-aligned.
pub(in crate::dispatch) async fn edit_multicam_switch(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // rationale is recorded on the lowered insert ops
    struct Args {
        tracks: Option<Vec<String>>,
        min_shot_ms: Option<u64>,
        reference_track: Option<String>,
        /// "speaker" | "energy". Default (omitted) = AUTO: speaker mode when a
        /// diarization is available for the reference/diarize asset, else energy.
        mode: Option<String>,
        /// The asset whose diarization drives speaker mode (default: the reference
        /// angle's asset). Must be one of the camera angles' assets (its track gives
        /// the timeline↔source mapping). Ignored in energy mode.
        diarize_asset: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    if let Some(m) = a.mode.as_deref() {
        if m != "speaker" && m != "energy" {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown multicam mode '{m}'"),
                "mode: \"speaker\" (cut to the active speaker's camera, needs media.diarize) | \"energy\" (loudest mic)",
            ));
        }
    }
    let min_shot_ms = a.min_shot_ms.unwrap_or(MULTICAM_DEFAULT_MIN_SHOT_MS).max(1);
    let window_ms = MULTICAM_ANALYSIS_WINDOW_MS;
    // The single output track the program is flattened onto. Excluded from
    // auto-detection so a re-run never treats its own prior output as an angle.
    let program_track = MULTICAM_PROGRAM_TRACK;

    // -- Read pass: resolve the angles, build per-window energies, decide
    // the shots, and pre-compute the insert plan. load_report is a small JSON read.
    struct Insert {
        asset: String,
        at_ms: u64,
        src_in: u64,
        src_out: u64,
    }
    let (cams_meta, shots, overlap_start, overlap_end, inserts, mode_info) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let project = &store.project;
        let edl = cut_core::edl_from_project(project);
        let receipts = store.receipts_dir();

        // 1. Resolve the camera track set: explicit `tracks`, else every VIDEO track
        // that holds a media clip (the multicam set).
        let track_ids: Vec<String> = match &a.tracks {
            Some(ts) => ts.clone(),
            None => project
                .tracks
                .iter()
                .filter(|t| {
                    t.kind == cut_core::TrackKind::Video
                        && t.id != program_track // never re-ingest our own output
                        && t.id != LEGACY_MULTICAM_PROGRAM_TRACK // do not treat legacy/user program tracks as angles
                        && t.clips
                            .iter()
                            .any(|c| matches!(c, cut_core::Clip::Media(_)))
                })
                .map(|t| t.id.clone())
                .collect(),
        };
        if track_ids.len() < 2 {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!(
                    "multicam switching needs ≥2 camera angles, found {}",
                    track_ids.len()
                ),
                "put each synced angle on its own video track (or pass tracks:[id,id,…])",
            )
            .with_suggested_action(
                "import each camera, place them time-aligned (edit.multicam_sync), then retry",
            ));
        }

        // 2. Build each camera's segment plan + timeline-mapped energy envelope.
        let mut cams: Vec<McCam> = Vec::new();
        for tid in &track_ids {
            let track = project.track(tid).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no track '{tid}'"),
                    "track ids come from project.state",
                )
            })?;
            if track.kind != cut_core::TrackKind::Video {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("track '{tid}' is not a video track"),
                    "multicam angles are video tracks (the switch composites the picture)",
                ));
            }
            let mut segs: Vec<McSeg> = Vec::new();
            let mut env: Vec<(u64, f64)> = Vec::new();
            let (mut cov_start, mut cov_end) = (u64::MAX, 0u64);
            for seg in edl.track_segments(tid) {
                let (Some(asset), Some(src_in), Some(src_out)) =
                    (seg.asset.as_deref(), seg.src_in_ms, seg.src_out_ms)
                else {
                    continue; // gaps carry no angle / audio
                };
                cov_start = cov_start.min(seg.timeline_in_ms);
                cov_end = cov_end.max(seg.timeline_out_ms);
                segs.push(McSeg {
                    asset: asset.to_string(),
                    tl_in: seg.timeline_in_ms,
                    tl_out: seg.timeline_out_ms,
                    src_in,
                    src_out,
                });
                // Map this segment's loudness windows (source time) → timeline time.
                let report = cut_perception::load_report(&receipts, asset)?.ok_or_else(|| {
                    CutError::new(
                        error_codes::NOT_FOUND,
                        format!("asset '{asset}' on '{tid}' has no perception report"),
                        "active-speaker switching reads each angle's loudness envelope",
                    )
                    .with_suggested_action(
                        "run media.perception{asset} (or wait for the import chain), then retry",
                    )
                })?;
                let span = (seg.timeline_out_ms - seg.timeline_in_ms) as f64;
                let src_span = src_out.saturating_sub(src_in).max(1) as f64;
                if let Some(loud) = report.loudness.as_ref() {
                    for w in &loud.windows {
                        if w.at_ms < src_in || w.at_ms > src_out {
                            continue;
                        }
                        let t = seg.timeline_in_ms
                            + (((w.at_ms - src_in) as f64) * span / src_span).round() as u64;
                        env.push((t, w.momentary_lufs));
                    }
                }
            }
            if segs.is_empty() {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("camera track '{tid}' has no media clips"),
                    "each angle needs at least one media clip on its track",
                ));
            }
            if env.is_empty() {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("camera track '{tid}' has no audio energy"),
                    "its clip(s) carry no loudness windows — multicam switching needs audio on every angle",
                )
                .with_suggested_action(
                    "use angles with an audio stream (the active speaker is the loudest angle)",
                ));
            }
            env.sort_by_key(|&(t, _)| t);
            let (env_t, env_e): (Vec<u64>, Vec<f64>) = env.into_iter().unzip();
            cams.push(McCam {
                track_id: tid.clone(),
                segs,
                cov_start,
                cov_end,
                env_t,
                env_e,
            });
        }

        // 3. Overlapping span = the intersection of every angle's coverage.
        let overlap_start = cams.iter().map(|c| c.cov_start).max().unwrap();
        let overlap_end = cams.iter().map(|c| c.cov_end).min().unwrap();
        if overlap_end <= overlap_start {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "the camera angles do not overlap in time",
                "multicam switching needs a span where ≥2 angles play at once",
            )
            .with_suggested_action(
                "align the angles on the timeline first (edit.multicam_sync → place each clip)",
            ));
        }

        // default / anchor camera = the reference track (else the first angle).
        let default_cam = match &a.reference_track {
            Some(r) => cams.iter().position(|c| &c.track_id == r).ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("reference_track '{r}' is not one of the camera angles"),
                    "reference_track must be one of the resolved tracks",
                )
            })?,
            None => 0,
        };

        // 4. Sample per-window energies across the overlap.
        let n_windows = ((overlap_end - overlap_start) / window_ms).max(1) as usize;
        let mut energies: Vec<Vec<f64>> = Vec::new();
        for i in 0..n_windows {
            let center =
                (overlap_start + i as u64 * window_ms + window_ms / 2).min(overlap_end - 1);
            energies.push(cams.iter().map(|c| mc_nearest_energy(c, center)).collect());
        }
        let window_center = |i: usize| -> u64 {
            (overlap_start + i as u64 * window_ms + window_ms / 2).min(overlap_end - 1)
        };

        // Speaker mode: when diarization is available for the
        // reference/diarize asset, cut to the camera of whoever is SPEAKING instead
        // of the loudest mic. Map each diarized speaker → the angle whose mic is
        // loudest during that speaker's turns (the on-mic speaker is loudest on their
        // own camera), then drive the program off the active speaker per window. Gated
        // on the presence of speaker_turns; energy mode stays the default/fallback.
        let energy_forced = a.mode.as_deref() == Some("energy");
        let speaker_requested = a.mode.as_deref() == Some("speaker");
        let mut speaker_shots: Option<Vec<cut_core::multicam::Shot>> = None;
        let mut mode_info = serde_json::json!({ "mode": "energy" });
        if !energy_forced {
            // The diarize source asset + the cam carrying it (for the tl↔src mapping).
            let diar_cam_idx = match a.diarize_asset.as_deref() {
                Some(da) => cams
                    .iter()
                    .position(|c| c.segs.iter().any(|s| s.asset == da)),
                None => Some(default_cam),
            };
            let diar_asset = a.diarize_asset.clone().or_else(|| {
                diar_cam_idx
                    .and_then(|i| cams[i].segs.first())
                    .map(|s| s.asset.clone())
            });
            let turns = match (&diar_cam_idx, &diar_asset) {
                (Some(_), Some(asset)) => cut_perception::load_report(&receipts, asset)?
                    .map(|r| r.speaker_turns)
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            if speaker_requested && turns.is_empty() {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    "speaker mode needs a diarized reference asset",
                    match &diar_asset {
                        Some(a) => format!("asset '{a}' has no speaker_turns"),
                        None => "no diarize asset could be resolved".to_string(),
                    },
                )
                .with_suggested_action(
                    "run media.diarize{asset} on the reference angle (or pass diarize_asset), then retry",
                ));
            }
            if let (Some(dci), Some(asset)) = (diar_cam_idx, diar_asset.clone()) {
                if !turns.is_empty() {
                    // Distinct speaker labels in arrival order → 0-based index space.
                    let mut labels: Vec<String> = Vec::new();
                    for t in &turns {
                        if !labels.iter().any(|l| l == &t.speaker) {
                            labels.push(t.speaker.clone());
                        }
                    }
                    let n_speakers = labels.len();
                    // Segments of the diarize cam that carry THIS asset (tl↔src for it).
                    let dsegs: Vec<&McSeg> =
                        cams[dci].segs.iter().filter(|s| s.asset == asset).collect();
                    let tl_to_src = |t: u64| -> Option<u64> {
                        for seg in &dsegs {
                            if t >= seg.tl_in && t < seg.tl_out {
                                let span = (seg.tl_out - seg.tl_in) as f64;
                                let src_span = seg.src_out.saturating_sub(seg.src_in) as f64;
                                return Some(
                                    seg.src_in
                                        + (((t - seg.tl_in) as f64) * src_span / span).round()
                                            as u64,
                                );
                            }
                        }
                        None
                    };
                    // Active speaker at a SOURCE instant: the covering turn (ties →
                    // latest onset = the most recent speaker), mapped to its index.
                    let active_at = |ts: u64| -> Option<usize> {
                        let mut chosen: Option<&cut_perception::SpeakerTurn> = None;
                        for t in &turns {
                            if ts >= t.start_ms
                                && ts < t.end_ms
                                && chosen.is_none_or(|c| t.start_ms > c.start_ms)
                            {
                                chosen = Some(t);
                            }
                        }
                        chosen.and_then(|t| labels.iter().position(|l| l == &t.speaker))
                    };
                    let speaker_active: Vec<Option<usize>> = (0..n_windows)
                        .map(|i| tl_to_src(window_center(i)).and_then(active_at))
                        .collect();

                    // Only adopt speaker mode if the diarize source actually covers the
                    // overlap (≥1 window has a speaker); else fall back to energy.
                    if speaker_active.iter().any(|s| s.is_some()) {
                        let map = cut_core::multicam::map_speakers_to_cameras(
                            &speaker_active,
                            &energies,
                            n_speakers,
                        );
                        let shots = cut_core::multicam::switch_shots_by_speaker(
                            &speaker_active,
                            &map,
                            &energies,
                            overlap_start,
                            window_ms,
                            min_shot_ms,
                            default_cam,
                        );
                        let s2c: Vec<Value> = labels
                            .iter()
                            .zip(map.iter())
                            .map(|(lab, &c)| {
                                serde_json::json!({"speaker": lab, "camera": cams[c].track_id})
                            })
                            .collect();
                        mode_info = serde_json::json!({
                            "mode": "speaker",
                            "diarize_asset": asset,
                            "num_speakers": n_speakers,
                            "speaker_to_cam": s2c,
                        });
                        speaker_shots = Some(shots);
                    } else if speaker_requested {
                        return Err(CutError::new(
                            error_codes::INVALID_ARGS,
                            "the diarized asset does not cover the angles' overlap",
                            "no diarized speech falls in the span where the angles play together",
                        )
                        .with_suggested_action(
                            "diarize the reference angle that spans the overlap, or use mode:\"energy\"",
                        ));
                    }
                }
            }
        }

        // Energy mode is the default + the fallback (no diarization available).
        let shots = match speaker_shots {
            Some(s) => s,
            None => cut_core::multicam::switch_shots(
                &energies,
                overlap_start,
                window_ms,
                min_shot_ms,
                MULTICAM_HYSTERESIS_LU,
                default_cam,
            ),
        };

        // 5. Lower each shot → edit.insert(s) of the active angle at the sync-correct
        // source offset (clamped to the overlap + the angle's own segments).
        let mut inserts: Vec<Insert> = Vec::new();
        for s in &shots {
            let a0 = s.start_ms.max(overlap_start);
            let b0 = s.end_ms.min(overlap_end);
            if b0 <= a0 {
                continue;
            }
            let cam = &cams[s.camera];
            for seg in &cam.segs {
                let u = a0.max(seg.tl_in);
                let v = b0.min(seg.tl_out);
                if v <= u {
                    continue;
                }
                let span = (seg.tl_out - seg.tl_in) as f64;
                let src_span = seg.src_out.saturating_sub(seg.src_in) as f64;
                let map = |t: u64| -> u64 {
                    seg.src_in + (((t - seg.tl_in) as f64) * src_span / span).round() as u64
                };
                inserts.push(Insert {
                    asset: seg.asset.clone(),
                    at_ms: u,
                    src_in: map(u),
                    src_out: map(v),
                });
            }
        }

        // Receipt-facing camera metadata (track id per index) — collected before the
        // lock drops; the shots index into this.
        let cams_meta: Vec<String> = cams.iter().map(|c| c.track_id.clone()).collect();
        (
            cams_meta,
            shots,
            overlap_start,
            overlap_end,
            inserts,
            mode_info,
        )
    };

    // -- Build pass: orchestrate the build (auto-checkpoint → fresh program track →
    // the lowered inserts), exactly like assemble.broll. No op of our own.
    let cp = Box::pin(dispatch(
        state,
        "project.checkpoint",
        json!({"name": "before-multicam-switch", "rationale": "auto: before edit.multicam_switch"}),
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
    let revert_hint =
        format!("project.revert{{to:\"{checkpoint_id}\"}} undoes the multicam program");

    // (Re)create the program track FRESH so a re-run overwrites cleanly: drop any
    // existing `program` track (with its clips), then add it back — appended last, so
    // it composites ON TOP of the angles.
    let exists = {
        let guard = state.project.read().await;
        guard
            .as_ref()
            .map(|s| s.project.tracks.iter().any(|t| t.id == program_track))
            .unwrap_or(false)
    };
    if exists {
        let rm = Box::pin(dispatch(
            state,
            "edit.remove_track",
            json!({"track": program_track, "force": true, "rationale": "auto: reset multicam program track"}),
            actor.clone(),
        ))
        .await;
        if !rm.ok {
            return Ok(rm);
        }
    }
    let at = Box::pin(dispatch(
        state,
        "edit.add_track",
        json!({"kind": "video", "id": program_track, "rationale": "auto: multicam program track"}),
        actor.clone(),
    ))
    .await;
    if !at.ok {
        return Ok(at);
    }

    // Place every shot segment onto the program track at its absolute timeline
    // position (no ripple — absolute placement, the angles below stay put).
    for (i, ins) in inserts.iter().enumerate() {
        let ires = Box::pin(dispatch(
            state,
            "edit.insert",
            json!({
                "asset": ins.asset,
                "track": program_track,
                "at_ms": ins.at_ms,
                "src_range_ms": [ins.src_in, ins.src_out],
                "ripple": false,
                "rationale": format!("auto: multicam_switch shot {i}"),
            }),
            actor.clone(),
        ))
        .await;
        if !ires.ok {
            return Ok(VerbResult::ok(json!({
                "status": "failed",
                "failed_step": "edit.insert",
                "shot": i,
                "error": ires.error,
                "checkpoint": checkpoint_id,
                "revert_hint": revert_hint,
            })));
        }
    }

    let shots_json: Vec<Value> = shots
        .iter()
        .map(|s| {
            json!({
                "start_ms": s.start_ms.max(overlap_start),
                "end_ms": s.end_ms.min(overlap_end),
                "camera": cams_meta[s.camera],
                "energy": if s.energy.is_finite() { (s.energy * 100.0).round() / 100.0 } else { 0.0 },
            })
        })
        .collect();

    let speaker_mode = mode_info.get("mode").and_then(|m| m.as_str()) == Some("speaker");
    let note = if speaker_mode {
        "SPEAKER mode: the picture cuts to the camera of whoever is SPEAKING (each \
         diarized speaker mapped to the angle whose mic is loudest during their turns), \
         so an off-camera loud noise no longer steals the shot. Audio is unchanged \
         (video tracks contribute no audio to the render). See `mode` for the \
         speaker→camera map."
    } else {
        "ENERGY mode: the picture switches to the loudest (active-speaker-by-mic) angle; \
         audio is unchanged (video tracks contribute no audio to the render). For \
         speaker-identity cutting run media.diarize on the reference angle (or pass \
         diarize_asset) and re-run — it auto-upgrades to speaker mode."
    };
    Ok(VerbResult::ok(json!({
        "status": "ok",
        "shots": shots_json,
        "switches": shots.len().saturating_sub(1),
        "tracks": cams_meta,
        "min_shot_ms": min_shot_ms,
        "program_track": program_track,
        "span_ms": [overlap_start, overlap_end],
        "mode": mode_info,
        "checkpoint": checkpoint_id,
        "revert_hint": revert_hint,
        "note": note,
    })))
}
