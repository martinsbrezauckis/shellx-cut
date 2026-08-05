//! Render, export-preview, bundle, queue, and autopilot dispatch handlers.

use super::*;

mod bundle_package;
use bundle_package::{assess_publish_package, optional_artifact_hash, publish_package_manifest};

/// Snapshot project + EDL + log head for render calls (no lock held while
/// ffmpeg runs).
pub(crate) async fn snapshot(
    state: &AppState,
) -> Result<(cut_core::Project, cut_core::Edl, PathBuf, String), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let edl = cut_core::edl_from_project(&store.project);
    let at_op = store
        .log
        .read_all()?
        .last()
        .map(|o| o.op_id.clone())
        .unwrap_or_else(|| "op_000000".into());
    Ok((store.project.clone(), edl, store.dir.clone(), at_op))
}

/// Snapshot and resolve compound clips for a media-reading operation without
/// mutating the stored project. Nest baking can invoke ffmpeg, so it always runs
/// outside the async runtime and after the project read lock has been released.
pub(crate) async fn snapshot_for_media_io(
    state: &AppState,
    context: &'static str,
) -> Result<(cut_core::Project, cut_core::Edl, PathBuf, String), CutError> {
    let (project, edl, dir, at_op) = snapshot(state).await?;
    if !project.has_nests() {
        return Ok((project, edl, dir, at_op));
    }
    let bake_dir = dir.clone();
    let (project, edl) = run_blocking(context, move || {
        crate::nest::flatten_for_media_io(&project, &edl, &bake_dir)
    })
    .await?;
    Ok((project, edl, dir, at_op))
}

/// Default preview window length — MUST match the `default` advertised for
/// `duration_ms` in schema/verbs.json (render.preview). A dispatch test
/// asserts the two stay in sync (the value drifted to 3000 once).
pub(super) const PREVIEW_DEFAULT_DURATION_MS: u64 = 5000;

/// render.preview{at_ms, duration_ms?, draft?} → fast low-res window render,
/// OR (`draft:true`) the incremental whole-timeline draft preview.
///
/// Default (draft:false): the original window render — a fast low-res encode of
/// `[at_ms, at_ms+duration_ms)`. `at_ms` is required in this mode.
///
/// draft:true: re-renders ONLY the base-track segments whose inputs changed
/// since the last preview, reuses cached segment files for the rest, and
/// concat-stitches them into one whole-timeline preview mp4. `at_ms`/
/// `duration_ms` are IGNORED (the preview is the whole timeline). Returns the
/// stitched path plus which segments rendered vs reused (the cache-correctness
/// evidence). Cache dir: `<proj>/proxies/preview-cache/` — DERIVED state, never
/// an op, never a receipt input (public verb contract; receipts stay render.final).
pub(super) async fn render_preview(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        at_ms: Option<u64>,
        duration_ms: Option<u64>,
        #[serde(default)]
        draft: bool,
    }
    let a: Args = parse_args(args)?;
    let (project, edl, dir, _at_op) = snapshot_for_media_io(state, "render.preview.nests").await?;

    if a.draft {
        // Incremental whole-timeline draft preview.
        let cache_dir = dir.join("proxies").join("preview-cache");
        let preset =
            cut_media::render::RenderPreset::named("draft").expect("draft is a registered preset");
        let r = run_blocking("render.preview.draft", move || {
            cut_media::preview::render_preview_incremental(
                &project, &edl, &dir, &cache_dir, &preset,
            )
        })
        .await?;
        return Ok(VerbResult::ok(json!({
            "path": r.path,
            "mime": "video/mp4",
            "draft": true,
            "segments_rendered": r.segments_rendered,
            "segments_reused": r.segments_reused,
            "rendered": r.rendered,
            "reused": r.reused,
            "duration_ms": r.duration_ms,
        })));
    }

    // Window render (the original synchronous path). at_ms is required here.
    let at_ms = a.at_ms.ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "at_ms is required for a window preview (omit it only with draft:true)",
            "render.preview without draft renders a window starting at at_ms",
        )
        .with_suggested_action(
            "pass at_ms, or pass draft:true for the whole-timeline incremental preview",
        )
    })?;
    let out_dir = dir.join("previews");
    std::fs::create_dir_all(&out_dir)?;
    let duration_ms = a.duration_ms.unwrap_or(PREVIEW_DEFAULT_DURATION_MS);
    let path = run_blocking("render.preview", move || {
        cut_media::render::render_preview(&project, &edl, &dir, at_ms, duration_ms, &out_dir)
    })
    .await?;
    Ok(VerbResult::ok(json!({"path": path, "mime": "video/mp4"})))
}

/// The frame-cache revision key: a hash of the PROJECT (its `.cutproj` dir) AND the
/// current op id. Every mutation appends an op (the id changes → cache invalidates
/// the old state), and the project dir makes the key PROJECT-UNIQUE.
///
/// BUG FIX: the old key was just the bare op-SEQUENCE number
/// (`op_000042` → 42). Sequence numbers RESTART per project, so two different
/// projects at the same op count + at_ms + height + mode collided in the global
/// frame cache — a fresh project would be served a DIFFERENT project's cached frame.
/// This silently failed the release `surface-blend` verification (the "after blend"
/// render at 2000 ms collided with a prior test's frame → SSIM 1.0) and could show a
/// user the wrong project's pixels. Hashing the dir in scopes the cache per project.
fn edl_rev(dir: &std::path::Path, at_op: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    dir.hash(&mut h);
    at_op.hash(&mut h);
    h.finish()
}

/// Scrub frame bytes with the fast proxy-seek path + LRU cache.
///
/// `height` scales the output (preview size; `?h=`). `compose` forces the EXACT
/// composed frame (captions + overlays, project geometry — the agent's verify
/// eyes); the default (false) takes the FAST path when the position is eligible
/// (`plan_scrub_frame`) and silently falls back to the composed path otherwise
/// (never a wrong frame). Returns `(bytes, used_fast)` so the caller can report
/// which path served the frame (latency-table evidence / UI hinting).
pub(crate) async fn scrub_frame_bytes(
    state: &AppState,
    at_ms: u64,
    height: u32,
    compose: bool,
) -> Result<(Vec<u8>, bool), CutError> {
    let (project, edl, dir, at_op) = snapshot_for_media_io(state, "render.frame.nests").await?;
    scrub_frame_bytes_from_snapshot(state, project, edl, dir, at_op, at_ms, height, compose).await
}

async fn scrub_frame_bytes_from_snapshot(
    state: &AppState,
    project: cut_core::Project,
    edl: cut_core::Edl,
    dir: PathBuf,
    at_op: String,
    at_ms: u64,
    height: u32,
    compose: bool,
) -> Result<(Vec<u8>, bool), CutError> {
    // Past the composition end → a BLACK frame, not a 422. The UI ruler extends
    // past short content (min 60s), so scrubbing into the empty region requested
    // frames the engine refused; the poster <img> then broke and the loading
    // spinner spun forever ("endless rendering"). Black is the NLE-correct view.
    if at_ms >= edl.duration_ms {
        let s = &project.settings;
        let aspect = if s.height > 0 {
            s.width as f64 / s.height as f64
        } else {
            16.0 / 9.0
        };
        let w = ((height as f64) * aspect).round().max(2.0) as u32;
        let h = height.max(2);
        let bytes = run_blocking("render.black", move || {
            cut_media::render::black_frame_jpeg(w, h)
        })
        .await?;
        return Ok((bytes, false));
    }
    let rev = edl_rev(&dir, &at_op);
    let mode = if compose {
        crate::framecache::FrameMode::Compose
    } else {
        crate::framecache::FrameMode::Scrub
    };
    // Cache hit → memcpy, no ffmpeg. used_fast reflects the cached path's mode.
    if let Some(bytes) = state.frame_cache.get(rev, at_ms, height, mode) {
        return Ok((bytes, mode == crate::framecache::FrameMode::Scrub));
    }
    // Decide the path: fast scrub when not compose AND the position is eligible.
    let plan = if compose {
        None
    } else {
        cut_media::render::plan_scrub_frame(&project, &edl, &dir, at_ms)
    };
    let (bytes, used_fast) = if let Some(plan) = plan {
        let bytes = run_blocking("render.scrub", move || {
            cut_media::render::extract_scrub_frame(&plan, height)
        })
        .await?;
        (bytes, true)
    } else {
        // Composed fallback (captions/overlays/gap/no-proxy or compose=1).
        // A preview request (height below the project's full height) composites at
        // that reduced height over the light proxies → ~8.5× faster on heavy 4K HEVC, no
        // more multi-second "freeze" when an effect flips the preview into composed mode.
        // export.frame asks for the full project height → None → full-res raw source.
        let preview_height = if height < project.settings.height.max(1) {
            Some(height)
        } else {
            None
        };
        let bytes = run_blocking("render.frame", move || {
            cut_media::render::extract_frame(&project, &edl, &dir, at_ms, preview_height)
        })
        .await?;
        (bytes, false)
    };
    let store_mode = if used_fast {
        crate::framecache::FrameMode::Scrub
    } else {
        crate::framecache::FrameMode::Compose
    };
    state
        .frame_cache
        .put(rev, at_ms, height, store_mode, bytes.clone());
    Ok((bytes, used_fast))
}

/// render.frame{at_ms, inline?, h?, compose?} → {path, mime, at_ms, width,
/// height, fast} (+base64 when inline, the binary-output contract).
///
/// `h` scales the frame to that height (preview size; default the proxy
/// height 540). `compose:true` forces the EXACT composed frame (captions +
/// overlays at project geometry) — the agent's verify eyes; the default takes
/// the fast proxy-seek scrub path when the position is eligible. `width`/
/// `height` in the result are the SERVED frame's geometry.
/// export.frame{at_ms, to_asset?, path?} — render the COMPOSED frame at `at_ms` at FULL
/// project resolution and save it to the default export folder as `frame_<ms>.jpg`. By default
/// (`to_asset`) it ALSO imports the saved image as a NEW asset, so the frame
/// appears in the Assets tray (draggable/insertable) AND is a real file on disk
/// importable into another project ("extract a frame as an asset"). The
/// extract is NON-DESTRUCTIVE: nothing on the timeline changes. Returns
/// {path, asset_id?, job_id?}.
pub(super) async fn export_frame(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        at_ms: u64,
        to_asset: Option<bool>,
        path: Option<String>,
    }
    let a: Args = parse_args(args)?;
    // Full project resolution + composed (all effects), so the still matches what
    // the viewer sees — not the proxy-grade scrub frame.
    let (project, _edl, dir, _at) = snapshot(state).await?;
    let height = project.settings.height.max(1);
    let (bytes, _fast) = scrub_frame_bytes(state, a.at_ms, height, true).await?;
    let path = fence_output_path(
        &dir,
        a.path.as_deref(),
        &format!("exports/frame_{}.jpg", a.at_ms),
    )?;
    write_output_atomic(&path, &bytes)?;

    if a.to_asset != Some(false) {
        // Register the saved frame as a first-class image asset (its own import op
        // + probe), so it shows in the Assets tray and can be inserted / reused.
        let imp = media_import(
            state,
            json!({ "path": path.display().to_string(), "rationale": format!("extracted frame @ {}ms", a.at_ms) }),
            actor,
        )
        .await?;
        let res = imp.result.unwrap_or(Value::Null);
        return Ok(VerbResult::ok(json!({
            "path": path,
            "asset_id": res.get("asset_id").cloned().unwrap_or(Value::Null),
            "job_id": res.get("job_id").cloned().unwrap_or(Value::Null),
        })));
    }
    Ok(VerbResult::ok(json!({ "path": path })))
}

/// export.range{range_ms, to_asset?, preset?} — render a TIME WINDOW of the
/// composed timeline (all effects baked) to exports/range_<in>_<out>.mp4 and, by
/// default, import it as a NEW asset — "save a part of the timeline as a reusable
/// clip" (#save-range-as-asset). Non-destructive; the saved clip lands in the
/// Assets tray AND on disk (importable into another project). Returns
/// {path, asset_id?, job_id?}.
/// Audio-only export formats → ffmpeg codec args + extension. mp3 (lossy,
/// universal), m4a/aac (lossy, Apple), wav (uncompressed PCM), flac (lossless
/// compressed), opus (modern lossy, best at low bitrate). None for an unknown id.
fn audio_format_args(format: &str) -> Option<(Vec<String>, &'static str)> {
    let s = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    Some(match format {
        "mp3" => (s(&["-c:a", "libmp3lame", "-q:a", "2"]), "mp3"),
        "m4a" | "aac" => (s(&["-c:a", "aac", "-b:a", "192k"]), "m4a"),
        "wav" => (s(&["-c:a", "pcm_s16le"]), "wav"),
        "flac" => (s(&["-c:a", "flac"]), "flac"),
        "opus" => (s(&["-c:a", "libopus", "-b:a", "160k"]), "opus"),
        _ => return None,
    })
}

/// export.publish{platform, hardware?, preset?, path?, dry_run?} — ONE-CLICK
/// platform export. Resolves a platform id (youtube|youtube_4k|tiktok|reels|
/// instagram_feed|x|square, + aliases shorts/twitter/ig) to its 2026-researched
/// encoding spec (output geometry, video+audio bitrate, format), then DELEGATES
/// to render.final with those args — reusing the entire render path (job,
/// auto-run checks, receipt). The result carries the chosen platform + spec so a
/// caller knows exactly what shipped. This is sugar: an agent can equally call
/// render.final{width,height,bitrate,format,…} directly. Vertical platforms
/// (tiktok/reels) reframe to 9:16 via render.final's explicit geometry + cover
/// fit (subject-aware reframe is the separate render.reframe verb).
pub(super) async fn export_publish(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        platform: Option<String>,
        /// Quality tier passthrough (draft|standard|high) — shifts the encoder
        /// effort; the bitrate target comes from the platform spec.
        preset: Option<String>,
        /// Encoder tier passthrough (auto|off) — auto uses the GPU when present.
        hardware: Option<String>,
        path: Option<String>,
        rationale: Option<String>,
        #[serde(default)]
        dry_run: bool,
    }
    let a: Args = parse_args(args)?;
    let platform = a.platform.as_deref().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "platform is required",
            format!(
                "platforms: {} (+ aliases shorts/twitter/ig)",
                cut_media::render::PLATFORM_NAMES.join("|")
            ),
        )
        .with_suggested_action("e.g. export.publish{platform:\"tiktok\"}")
    })?;
    let spec = cut_media::render::platform_spec(platform).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown platform '{platform}'"),
            format!(
                "platforms: {} (+ aliases shorts/twitter/ig)",
                cut_media::render::PLATFORM_NAMES.join("|")
            ),
        )
        .with_suggested_action("e.g. export.publish{platform:\"youtube\"} or {platform:\"reels\"}")
    })?;
    // Compose the render.final args from the platform spec and delegate. Explicit
    // width/height sets Resolution::Explicit (render.final defaults its fit to
    // cover there — fills the new frame, the publish intent). bitrate/audio_bitrate
    // hit the exact platform target via the codec-aware rate-control rewrite.
    let mut rf = serde_json::Map::new();
    rf.insert("width".into(), json!(spec.width));
    rf.insert("height".into(), json!(spec.height));
    rf.insert("format".into(), json!(spec.format));
    rf.insert("bitrate".into(), json!(format!("{}k", spec.video_kbps)));
    rf.insert(
        "rate_control".into(),
        json!(if spec.cbr { "cbr" } else { "vbr" }),
    );
    rf.insert(
        "audio_bitrate".into(),
        json!(format!("{}k", spec.audio_kbps)),
    );
    if let Some(hw) = a.hardware.as_deref() {
        rf.insert("hardware".into(), json!(hw));
    }
    if let Some(q) = a.preset.as_deref() {
        rf.insert("preset".into(), json!(q));
    }
    if let Some(p) = a.path.as_deref() {
        rf.insert("path".into(), json!(p));
    }
    if a.dry_run {
        rf.insert("dry_run".into(), json!(true));
    }
    rf.insert(
        "rationale".into(),
        json!(a
            .rationale
            .clone()
            .unwrap_or_else(|| format!("publish for {}", spec.label))),
    );
    // Delegate to the full render path (job + checks + receipt). `?` propagates an
    // actionable error (empty timeline, bad path) straight through.
    let res = render_final(state, Value::Object(rf), actor).await?;
    // Annotate the render result with the platform facts (the job_id/render_id
    // pass through unchanged), so the caller sees exactly what was targeted.
    let publish = json!({
        "platform": platform,
        "label": spec.label,
        "width": spec.width,
        "height": spec.height,
        "video_bitrate": format!("{}k", spec.video_kbps),
        "audio_bitrate": format!("{}k", spec.audio_kbps),
        "rate_control": if spec.cbr { "cbr" } else { "vbr" },
        "format": spec.format,
    });
    let mut merged = res.result.unwrap_or_else(|| json!({}));
    if let Some(obj) = merged.as_object_mut() {
        obj.insert("publish".into(), publish);
    }
    Ok(VerbResult {
        result: Some(merged),
        ..res
    })
}

/// export.audio{format?, path?} — export the timeline's MIXED audio as an audio
/// file (mp3|m4a|wav|flac|opus, default mp3). Renders the same audio graph as
/// render.final (per-track mix, gains/fades/speed) with no video, into the
/// project's exports/ (downloadable via /api/export). Synchronous like the other
/// export.* verbs; returns the file path.
pub(super) async fn export_audio(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        format: Option<String>,
        path: Option<String>,
        /// Optional: export ONLY this audio track's contribution (its processed
        /// audio — clip effects/eq/fades + the track's gain + ducking — as it sits
        /// in the full mix) = a per-track STEM. Drives the mixer's per-track meters
        /// (v2b). The sum of every track's stem == the full mix (WYSIWYG). NO engine
        /// change: we render a project VIEW that holds only this track, so build_graph
        /// produces just its audio. ids from project.state (audio tracks only).
        track: Option<String>,
        /// Also import the exported audio as a project asset (Assets tray —
        /// reusable as a music bed etc.). Default false: export.audio writes a
        /// file; opt in to add it to the tray. Returns {asset_id, job_id} too.
        #[serde(default)]
        to_asset: bool,
        #[serde(default)]
        #[allow(dead_code)] // accepted for parity; export is not an op
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let format = a.format.as_deref().unwrap_or("mp3").to_string();
    let (audio_args, ext) = audio_format_args(&format).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown audio format '{format}'"),
            "formats: mp3 | m4a | wav | flac | opus (default mp3)",
        )
        .with_suggested_action(
            "mp3 = universal, wav = uncompressed, flac = lossless, opus = best small, m4a = Apple",
        )
    })?;
    let (mut project, edl, dir, _at) = snapshot_for_media_io(state, "export.audio.nests").await?;
    if edl.duration_ms == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "timeline is empty — nothing to export",
            "insert at least one clip before export.audio",
        ));
    }
    // Per-track STEM (v2b): keep only the requested audio track in the project
    // VIEW we render, so build_graph emits just that track's audio (the EDL still
    // holds its segments, keyed by id). The full timeline length is preserved
    // (silence in this track's gaps) → the stem is the track's exact contribution.
    if let Some(tid) = &a.track {
        let is_audio_track = project
            .tracks
            .iter()
            .any(|t| &t.id == tid && t.kind == cut_core::TrackKind::Audio);
        if !is_audio_track {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                format!("no audio track '{tid}'"),
                "export.audio{track} isolates ONE audio track's stem; ids come from project.state",
            ));
        }
        project.tracks.retain(|t| &t.id == tid);
        // The stem is this track's RAW contribution — it drives the mixer's
        // per-track meter and the "Listen" audition, where the user expects to
        // HEAR the track regardless of its mute/solo state (you audition a track
        // to decide whether to unmute it). So clear the audibility flags on this
        // render VIEW (a snapshot copy — the stored project is untouched); without
        // this, a muted track's stem would be silent and lose its meter.
        for t in &mut project.tracks {
            t.muted = false;
            t.solo = false;
        }
    }
    let default_rel = match &a.track {
        Some(tid) => format!("exports/audio_{tid}.{ext}"),
        None => format!("exports/audio.{ext}"),
    };
    let out = fence_output_path(&dir, a.path.as_deref(), &default_rel)?;
    let (p, e, d, o, aa) = (project, edl, dir.clone(), out.clone(), audio_args);
    let res = run_blocking("export.audio", move || {
        let fence = make_fence(&d)?;
        cut_media::render::render_audio(&p, &e, &fence, &o, &aa, None)
    })
    .await?;
    // to_asset: import the rendered audio as a project asset (Assets tray) — like
    // export.range. Returns {asset_id, job_id} alongside the file facts.
    if a.to_asset {
        let imp = media_import(
            state,
            json!({
                "path": out.display().to_string(),
                "rationale": format!("exported timeline audio ({format}) as asset"),
            }),
            actor,
        )
        .await?;
        let r = imp.result.unwrap_or(Value::Null);
        return Ok(VerbResult::ok(json!({
            "path": out.display().to_string(),
            "format": format,
            "duration_ms": res.duration_ms,
            "hash": res.hash,
            "asset_id": r.get("asset_id").cloned().unwrap_or(Value::Null),
            "job_id": r.get("job_id").cloned().unwrap_or(Value::Null),
        })));
    }
    Ok(VerbResult::ok(json!({
        "path": out.display().to_string(),
        "format": format,
        "duration_ms": res.duration_ms,
        "hash": res.hash,
        "track": a.track,
    })))
}

pub(super) async fn export_range(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        range_ms: [u64; 2],
        to_asset: Option<bool>,
        preset: Option<String>,
        /// Explicit output file path (fenced). Default: the session output dir
        /// or <project>/exports/range_<in>_<out>.mp4.
        path: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (project, edl, dir, _at) = snapshot_for_media_io(state, "export.range.nests").await?;
    if a.range_ms[1] <= a.range_ms[0] {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "range_ms must be [start, end) with end > start",
            format!("got {:?}", a.range_ms),
        ));
    }
    let preset = match a.preset.as_deref() {
        None => cut_media::render::RenderPreset::default(),
        Some(name) => cut_media::render::RenderPreset::named(name).ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown preset '{name}'"),
                format!(
                    "presets: {} (default standard)",
                    cut_media::render::PRESET_NAMES.join("|")
                ),
            )
        })?,
    };
    let out = fence_output_path(
        &dir,
        a.path.as_deref(),
        &format!("exports/range_{}_{}.mp4", a.range_ms[0], a.range_ms[1]),
    )?;
    let tmp_out = temp_output_path_for_render(&out);
    let (p, e, d, o, t, range) = (
        project,
        edl,
        dir.clone(),
        out.clone(),
        tmp_out.clone(),
        a.range_ms,
    );
    run_blocking("export.range", move || {
        let fence = make_fence(&d)?;
        match cut_media::render::render_range(
            &p,
            &e,
            &fence,
            &t,
            &preset,
            range,
            cut_media::render::RenderOptions::default(),
            None,
        ) {
            Ok(_) => publish_output_atomic(&t, &o),
            Err(err) => {
                let _ = std::fs::remove_file(&t);
                Err(err)
            }
        }
    })
    .await?;

    if a.to_asset != Some(false) {
        let imp = media_import(
            state,
            json!({ "path": out.display().to_string(), "rationale": format!("saved timeline range {}-{}ms as asset", a.range_ms[0], a.range_ms[1]) }),
            actor,
        )
        .await?;
        let res = imp.result.unwrap_or(Value::Null);
        return Ok(VerbResult::ok(json!({
            "path": out,
            "asset_id": res.get("asset_id").cloned().unwrap_or(Value::Null),
            "job_id": res.get("job_id").cloned().unwrap_or(Value::Null),
        })));
    }
    Ok(VerbResult::ok(json!({ "path": out })))
}

/// export.gif{range_ms?, fps?, width?, dither?, to_asset?} — export a SHORT
/// timeline window as a looping animated GIF (the social/reaction-clip export).
/// Renders the window to a temp clip (reusing render_range — all effects baked),
/// then converts via the ffmpeg palettegen/paletteuse high-quality path
/// (cut_media::gif). Default range = the first 15 s; hard-capped at 30 s because
/// GIFs balloon. fps default 12, width default 480 px (the file-size levers).
/// Output exports/gif_<in>_<out>.gif; imports as an asset by default.
pub(super) async fn export_gif(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        range_ms: Option<[u64; 2]>,
        fps: Option<u32>,
        width: Option<u32>,
        dither: Option<String>,
        to_asset: Option<bool>,
        /// Explicit output file path (fenced). Default: the session output dir
        /// or <project>/exports/gif_<in>_<out>.gif.
        path: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (project, edl, dir, _at) = snapshot_for_media_io(state, "export.gif.nests").await?;
    if edl.duration_ms == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "timeline is empty — nothing to export",
            "insert at least one clip before export.gif",
        ));
    }
    // Default = the first 15 s of the timeline (a GIF wants to be short); an
    // explicit range overrides. Clamp the end to the timeline duration.
    let range = a.range_ms.unwrap_or([0, edl.duration_ms.min(15_000)]);
    if range[1] <= range[0] {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "range_ms must be [start, end) with end > start",
            format!("got {range:?}"),
        ));
    }
    let span = range[1].saturating_sub(range[0]);
    if span > 30_000 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("GIF range {span} ms exceeds the 30 s cap"),
            "GIFs balloon past a few seconds — pick a shorter range_ms",
        )
        .with_suggested_action(
            "export a ≤30 s window, or use render.final/export.range for a long clip",
        ));
    }
    let fps = a.fps.unwrap_or(12);
    let width = a.width.unwrap_or(480);
    let dither = a.dither.unwrap_or_else(|| "floyd".into());
    // A draft-tier temp render is plenty — the GIF re-quantizes to 256 colours
    // anyway, so a high-CRF source clip would only waste encode time.
    let preset = cut_media::render::RenderPreset::named("draft").unwrap_or_default();
    let out = fence_output_path(
        &dir,
        a.path.as_deref(),
        &format!("exports/gif_{}_{}.gif", range[0], range[1]),
    )?;
    let tmp = out.with_extension("src.mp4"); // composed window, deleted after
    let (p, e, d, o, t) = (project, edl, dir.clone(), out.clone(), tmp.clone());
    run_blocking("export.gif", move || {
        let fence = make_fence(&d)?;
        cut_media::render::render_range(
            &p,
            &e,
            &fence,
            &t,
            &preset,
            range,
            cut_media::render::RenderOptions::default(),
            None,
        )?;
        cut_media::gif::make_gif(&t, &o, fps, width, &dither)?;
        let _ = std::fs::remove_file(&t); // best-effort temp cleanup
        Ok(())
    })
    .await?;

    if a.to_asset != Some(false) {
        let imp = media_import(
            state,
            json!({ "path": out.display().to_string(), "rationale": format!("GIF of timeline range {}-{}ms", range[0], range[1]) }),
            actor,
        )
        .await?;
        let res = imp.result.unwrap_or(Value::Null);
        return Ok(VerbResult::ok(json!({
            "path": out, "fps": fps, "width": width, "range_ms": range,
            "asset_id": res.get("asset_id").cloned().unwrap_or(Value::Null),
            "job_id": res.get("job_id").cloned().unwrap_or(Value::Null),
        })));
    }
    Ok(VerbResult::ok(
        json!({ "path": out, "fps": fps, "width": width, "range_ms": range }),
    ))
}

pub(super) async fn render_frame(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        at_ms: u64,
        #[serde(default)]
        inline: bool,
        h: Option<u32>,
        #[serde(default)]
        compose: bool,
    }
    let a: Args = parse_args(args)?;
    let height = a.h.unwrap_or(cut_media::render::SCRUB_DEFAULT_HEIGHT);
    let (bytes, used_fast) = scrub_frame_bytes(state, a.at_ms, height, a.compose).await?;
    let (_project, _e, dir, _a2) = snapshot(state).await?;
    let frames = dir.join("frames");
    std::fs::create_dir_all(&frames)?;
    let path = frames.join(format!("frame_{}_{}.jpg", a.at_ms, height));
    std::fs::write(&path, &bytes)?;
    // Report the served frame's true pixel geometry (from the JPEG header) so
    // the caller never has to guess what scale=-2:h produced.
    let (w, h) = jpeg_dimensions(&bytes).unwrap_or((0, height));
    let mut result = json!({
        "path": path,
        "mime": "image/jpeg",
        "at_ms": a.at_ms,
        "width": w,
        "height": h,
        "fast": used_fast,
    });
    if a.inline {
        use base64::Engine;
        result["base64"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes));
    }
    Ok(VerbResult::ok(result))
}

/// Read width/height from a JPEG's SOF marker (no image crate dep — we only
/// need two u16s). Returns None on a non-JPEG / truncated buffer.
pub(super) fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    // JPEG: starts FF D8; scan markers for a Start-Of-Frame (FFC0..FFCF except
    // C4/C8/CC), whose payload is [precision(1)][height(2)][width(2)].
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        // SOF markers carry the dimensions.
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return Some((w, h));
        }
        // Other markers: skip by their segment length (next 2 bytes).
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        i += 2 + len;
    }
    None
}

/// 16:9 tile width for a requested storyboard height. Ceiling first so a
/// scaled 16:9 frame can never be wider than its pad target, then round up to
/// an even pixel count for yuv420/mjpeg compatibility.
pub(super) fn storyboard_tile_width(height: u32) -> u32 {
    let width = height.saturating_mul(16).div_ceil(9).max(2);
    width + (width & 1)
}

struct StoryboardScratch {
    path: PathBuf,
}

impl StoryboardScratch {
    fn prepare(path: PathBuf) -> Result<Self, CutError> {
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StoryboardScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn storyboard_assembly_error(_error: CutError) -> CutError {
    CutError::new(
        error_codes::FFMPEG,
        "could not assemble the storyboard",
        "the extracted frames could not be scaled and tiled into one image",
    )
    .with_suggested_action(
        "retry once; if it continues, open Settings > Environment and re-check FFmpeg",
    )
}

pub(super) async fn write_storyboard_tiles<F, Fut>(
    count: usize,
    dur: u64,
    height: u32,
    compose: bool,
    tmp: &Path,
    mut extract: F,
) -> Result<Vec<Value>, CutError>
where
    F: FnMut(usize, u64, u32, bool) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>, CutError>>,
{
    let mut warnings = Vec::new();
    for i in 0..count {
        let at = (dur * (2 * i as u64 + 1) / (2 * count as u64)).min(dur.saturating_sub(1));
        let bytes = match extract(i, at, height, compose).await {
            Ok(bytes) => bytes,
            Err(err) => {
                warnings.push(json!({
                    "index": i,
                    "at_ms": at,
                    "code": err.code,
                    "message": err.message,
                    "cause": err.cause,
                }));
                let width = storyboard_tile_width(height);
                let h = height.max(2);
                run_blocking("render.storyboard.black", move || {
                    cut_media::render::black_frame_jpeg(width, h)
                })
                .await?
            }
        };
        std::fs::write(tmp.join(format!("f{i:03}.jpg")), &bytes)?;
    }
    Ok(warnings)
}

/// render.storyboard{count?, h?, compose?, inline?} → a contact-sheet image:
/// `count` evenly-spaced frames of the composed timeline tiled into a grid
/// (binary verb, the binary-output contract: {path, mime} by default, `inline` adds base64). The
/// agent/judge's "see the whole edit at a glance" view — one image instead of N
/// frame fetches. Fast proxy-scrub frames by
/// default; `compose:true` forces exact composed frames (captions/overlays).
/// Written to the engine-owned `frames/` dir (no caller path → no fence needed,
/// same as render.frame).
pub(super) async fn render_storyboard(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        count: Option<usize>,
        h: Option<u32>,
        #[serde(default)]
        compose: bool,
        #[serde(default)]
        inline: bool,
    }
    let a: Args = parse_args(args)?;
    let count = a.count.unwrap_or(12).clamp(1, 100);
    let height = a.h.unwrap_or(180).clamp(60, 720);
    let (project, edl, dir, at_op) =
        snapshot_for_media_io(state, "render.storyboard.nests").await?;
    if edl.duration_ms == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "timeline is empty — nothing to storyboard",
            "EDL duration is 0 ms",
        )
        .with_suggested_action("insert at least one clip first"));
    }
    let dur = edl.duration_ms;
    let frames_dir = dir.join("frames");
    std::fs::create_dir_all(&frames_dir)?;
    // Engine-owned scratch subdir for the per-frame JPEGs (cleaned up after).
    let scratch = StoryboardScratch::prepare(frames_dir.join(".sb_tmp"))?;
    // Extract `count` frames at the MIDPOINT of each equal slice. A contact
    // sheet is a review aid, so a single bad source frame becomes a visible
    // black tile plus structured warning instead of aborting the whole sheet.
    let frame_warnings = write_storyboard_tiles(
        count,
        dur,
        height,
        a.compose,
        scratch.path(),
        |_, at, height, compose| {
            let state = state.clone();
            let project = project.clone();
            let edl = edl.clone();
            let dir = dir.clone();
            let at_op = at_op.clone();
            async move {
                let (bytes, _fast) = scrub_frame_bytes_from_snapshot(
                    &state, project, edl, dir, at_op, at, height, compose,
                )
                .await?;
                Ok(bytes)
            }
        },
    )
    .await?;
    // Grid: roughly square, columns ≥ rows. Each frame letterboxed into a 16:9
    // box at the requested height so the tile filter gets uniform inputs.
    let cols = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(cols);
    let (tw, th) = (storyboard_tile_width(height), height.max(2));
    let out = frames_dir.join(format!("storyboard_{count}_{height}.jpg"));
    let pattern = scratch.path().join("f%03d.jpg");
    let vf = format!(
        "scale={tw}:{th}:force_original_aspect_ratio=decrease,\
         pad={tw}:{th}:-1:-1:color=black,tile={cols}x{rows}:padding=4:color=black"
    );
    let ff_args: Vec<String> = vec![
        "-y".into(),
        "-framerate".into(),
        "1".into(),
        "-i".into(),
        pattern.to_string_lossy().into_owned(),
        "-vf".into(),
        vf,
        "-frames:v".into(),
        "1".into(),
        out.to_string_lossy().into_owned(),
    ];
    run_blocking("render.storyboard", move || {
        cut_media::ffmpeg::run_ffmpeg(&ff_args)
    })
    .await
    .map_err(storyboard_assembly_error)?;
    let mut result = json!({
        "path": out, "mime": "image/jpeg",
        "count": count, "grid": [cols, rows],
        "frame_height": height, "duration_ms": dur,
        "fallback_frames": frame_warnings.len(),
        "frame_warnings": frame_warnings,
    });
    if a.inline {
        use base64::Engine;
        let bytes = std::fs::read(&out)?;
        result["base64"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes));
    }
    Ok(VerbResult::ok(result))
}

#[cfg(test)]
mod storyboard_internal_tests {
    use super::*;

    #[test]
    fn tile_width_is_ceiling_based_and_even() {
        for (height, expected) in [
            (90, 160),
            (120, 214),
            (150, 268),
            (180, 320),
            (200, 356),
            (240, 428),
            (360, 640),
        ] {
            let width = storyboard_tile_width(height);
            assert_eq!(width, expected, "height {height}");
            assert_eq!(width % 2, 0, "height {height} must produce even width");
            assert!(
                width.saturating_mul(9) >= height.saturating_mul(16),
                "height {height} must never floor below 16:9"
            );
        }
    }

    #[test]
    fn scratch_guard_cleans_after_an_error_exit() {
        fn fail_inside(path: PathBuf) -> Result<(), CutError> {
            let scratch = StoryboardScratch::prepare(path)?;
            std::fs::write(scratch.path().join("private-frame.jpg"), b"fixture")?;
            Err(CutError::new(
                error_codes::FFMPEG,
                "deliberate ffmpeg failure",
                "private stderr and scratch path",
            ))
        }

        let root = tempfile::tempdir().unwrap();
        let scratch = root.path().join(".sb_tmp");
        assert!(fail_inside(scratch.clone()).is_err());
        assert!(!scratch.exists(), "scratch must be removed on error");
    }

    #[test]
    fn assembly_error_envelope_drops_raw_stderr_and_private_paths() {
        let private = "/private/project/frames/.sb_tmp/f001.jpg";
        let mapped = storyboard_assembly_error(CutError::new(
            error_codes::FFMPEG,
            "ffmpeg exited with 234",
            format!("Padded dimensions failed for {private}\nraw stderr"),
        ));
        let encoded = serde_json::to_string(&mapped).unwrap();
        assert_eq!(mapped.code, error_codes::FFMPEG);
        assert!(!encoded.contains(private));
        assert!(!encoded.contains("raw stderr"));
        assert!(mapped.suggested_action.is_some());
    }
}

/// render.final{path?, preset?, profile?, fit?, resolution?, rationale?} —
/// render job; on success AUTO-runs verify.checks and assembles + persists +
/// broadcasts the RenderReceipt. Event order guaranteed: job_progress* →
/// render_done → receipt_ready (the event-ordering contract — published sequentially from this
/// one task).
///
/// Framing options — DEFAULTS UNCHANGED so existing op logs replay
/// byte-identical: `fit` (contain = current behavior = default | cover =
/// crop-to-fill) and `resolution` (project = settings geometry = default |
/// match_source = largest source video). Quality presets (draft/standard/high)
/// are independent of these (they pick encoder effort, not geometry).
/// Map a `"w:h"` aspect string to concrete even output dimensions, using the
/// social-video baseline of 1080 px on the SHORTER side (9:16 → 1080×1920,
/// 16:9 → 1920×1080, 1:1 → 1080×1080, 4:5 → 1080×1350, …). Ratio components
/// must be small positive integers (1..=100). Callers wanting non-1080 output
/// pass explicit width+height instead.
pub(super) fn dims_from_aspect(s: &str) -> Result<(u32, u32), CutError> {
    let bad = || {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("'{s}' is not a valid aspect ratio"),
            "use W:H with small integers, e.g. 9:16, 16:9, 1:1, 4:5",
        )
    };
    let (a, b) = s.split_once(':').ok_or_else(bad)?;
    let rw: u32 = a.trim().parse().map_err(|_| bad())?;
    let rh: u32 = b.trim().parse().map_err(|_| bad())?;
    if rw == 0 || rh == 0 || rw > 100 || rh > 100 {
        return Err(bad());
    }
    let base = 1080.0_f64;
    let even = |v: f64| (v.round() as u32) & !1u32;
    let (w, h) = if rw <= rh {
        (base, base * rh as f64 / rw as f64)
    } else {
        (base * rw as f64 / rh as f64, base)
    };
    Ok((even(w).max(2), even(h).max(2)))
}

/// Value following `flag` in an ffmpeg arg vec ("-c:v" → "libx264"|"hevc_nvenc").
/// Used to label the receipt with the encoder that actually ran.
fn video_arg_value(args: &[String], flag: &str) -> String {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_default()
}

/// Mark the checks whose facts come from the post-render sidecar as
/// UNMEASURED when that sidecar fails. Their `pass:false` keeps the aggregate
/// receipt non-green, while the structured status prevents the UI/autopilot
/// from misreporting a runtime gap as a content defect.
pub(super) fn mark_output_checks_unmeasured(
    checks: &mut [cut_core::CheckResult],
    error: &CutError,
) {
    for check in checks.iter_mut().filter(|check| {
        matches!(
            check.name.as_str(),
            cut_core::check_names::LUFS
                | cut_core::check_names::BLACK_OR_FROZEN_FRAMES
                | cut_core::check_names::UNIFORM_BORDER
                | cut_core::check_names::SILENCE_AT_EDGES
        )
    }) {
        let attempted_details = std::mem::replace(&mut check.details, Value::Null);
        check.pass = false;
        check.details = json!({
            "status": "unmeasured",
            "measured": false,
            "reason": "post-render perception instrumentation failed",
            "runtime_error": error,
            "attempted_details": attempted_details,
        });
    }
}

pub(super) fn target_size_video_kbps(
    target_size_mb: f64,
    duration_ms: u64,
    audio_kbps_budget: u32,
) -> Result<u32, CutError> {
    if !(target_size_mb.is_finite() && target_size_mb > 0.0) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("invalid target_size_mb {target_size_mb}"),
            "target_size_mb is a positive number of megabytes (e.g. 25)",
        ));
    }
    let dur_sec = std::time::Duration::from_millis(duration_ms)
        .as_secs_f64()
        .max(0.1);
    let total_kbps = (target_size_mb * 8.0 * 1024.0) / dur_sec;
    let vkbps = ((total_kbps - f64::from(audio_kbps_budget)) * 0.90).floor();
    if vkbps < 50.0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("target_size_mb {target_size_mb} is too small for a {dur_sec:.1}s timeline"),
            format!("after {audio_kbps_budget}k audio there is < 50 kbps left for video"),
        )
        .with_suggested_action(
            "raise target_size_mb, shorten the timeline, or lower audio_bitrate",
        ));
    }
    if !vkbps.is_finite() || vkbps > f64::from(u32::MAX) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("target_size_mb {target_size_mb} is too large for a {dur_sec:.1}s timeline"),
            format!("computed video bitrate {vkbps:.0} kbps exceeds u32::MAX"),
        )
        .with_suggested_action("lower target_size_mb or pass an explicit bitrate"));
    }
    floor_f64_to_u32(vkbps).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "target_size_mb could not be converted to video bitrate",
            format!("computed video bitrate {vkbps:.0} kbps is outside u32 range"),
        )
    })
}

fn floor_f64_to_u32(value: f64) -> Option<u32> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return None;
    }
    let mut lo = 0u32;
    let mut hi = u32::MAX;
    while lo < hi {
        let span = hi - lo;
        let mid = lo + (span / 2) + (span % 2);
        if f64::from(mid) <= value {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Some(lo)
}

fn receipt_id(prefix: &str, n: usize) -> String {
    format!("{prefix}_{n:03}")
}

fn receipt_id_taken(receipts: &Path, id: &str) -> bool {
    receipts.join(format!("{id}.json")).exists()
        || receipts.join(format!(".{id}.reserved")).exists()
}

pub(super) fn next_receipt_id_preview(receipts: &Path, prefix: &str) -> String {
    for n in 1.. {
        let id = receipt_id(prefix, n);
        if !receipt_id_taken(receipts, &id) {
            return id;
        }
    }
    unreachable!("unbounded receipt id iterator")
}

pub(super) fn reserve_receipt_id(
    receipts: &Path,
    prefix: &str,
) -> Result<(String, PathBuf), CutError> {
    std::fs::create_dir_all(receipts)?;
    for n in 1.. {
        let id = receipt_id(prefix, n);
        if receipts.join(format!("{id}.json")).exists() {
            continue;
        }
        let marker = receipts.join(format!(".{id}.reserved"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(mut file) => {
                use std::io::Write as _;
                let _ = writeln!(file, "{}", OpRecord::now_ts());
                return Ok((id, marker));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!("unbounded receipt id iterator")
}

pub(super) async fn render_final(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        path: Option<String>,
        preset: Option<String>,
        profile: Option<String>,
        fit: Option<String>,
        resolution: Option<String>,
        aspect: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
        normalize_loudness: Option<i32>,
        /// Output FILE FORMAT (codec+container): h264|hevc|vp9|prores|av1.
        /// Default h264/mp4 (byte-identical to a no-format render). The `preset`
        /// (draft/standard/high) is the quality tier WITHIN the chosen codec.
        format: Option<String>,
        /// Encoder tier: "auto" (default) uses the GPU/hardware encoder when one
        /// is present (NVENC/QSV/AMF/VideoToolbox — much faster, and AV1-HW is the
        /// quality ceiling); "off" forces the software encoder (byte-deterministic,
        /// max libx264 quality). Only h264/hevc/av1 have a hardware tier.
        hardware: Option<String>,
        /// Target VIDEO bitrate for rate-targeted publishing (e.g. "12M", "8000k",
        /// bare = kbps). Default: omitted = quality-targeted CRF (the byte-identical
        /// path). Setting it switches the encoder to a real bitrate target — needed
        /// to hit a platform spec. ProRes ignores it (profile-fixed).
        bitrate: Option<String>,
        /// Target output FILE SIZE in MB — "fit under X" (Discord 25, email, etc.).
        /// Computes the video bitrate that keeps the file under the size (from the
        /// timeline duration, reserving audio, 90% headroom) and renders VBR-under.
        /// Mutually exclusive with `bitrate`. ProRes ignores it.
        target_size_mb: Option<f64>,
        /// Rate control WHEN `bitrate` is set: "vbr" (default — average target,
        /// caps ~1.45× for motion headroom) or "cbr" (constant — pins the rate,
        /// for strict-ingest platforms/live). Ignored without `bitrate`.
        rate_control: Option<String>,
        /// Target AUDIO bitrate (e.g. "384k", "192k"). Default: the format's
        /// audio rate (192k AAC). Lossless formats (wav/prores PCM) ignore it.
        audio_bitrate: Option<String>,
        #[serde(default)]
        dry_run: bool,
    }
    // STRICT parse (not unwrap_or_default): a TYPE mismatch on a known key would
    // otherwise collapse the WHOLE struct to defaults silently — e.g.
    // `dry_run:"true"` (string) would become false and run a full slow encode
    // instead of returning the plan, and a bad width/height would render at the
    // wrong geometry. parse_args surfaces it as an actionable invalid_args error.
    let a: Args = parse_args(args)?;
    // Footage profile for the auto-run check battery — parsed
    // NOW so a typo fails fast, not after a long encode. None = the strict
    // talking_head battery (cut-perception's default).
    let profile = a
        .profile
        .as_deref()
        .map(str::parse::<cut_perception::FootageProfile>)
        .transpose()
        .map_err(|e| {
            CutError::new(error_codes::INVALID_ARGS, "unknown footage profile", e)
                .with_suggested_action("valid profiles: talking_head, silent_screen_demo")
        })?;
    // Framing options — parsed NOW (typo fails fast, not after a long
    // encode). Omitted = RenderOptions::default() = contain + project geometry
    // = the legacy render (byte-identical replay).
    // Parse fit but REMEMBER whether the caller set it — when reframing to an
    // explicit geometry we default fit to `cover` (contain would letterbox the
    // new frame, defeating the reframe), but never override an explicit choice.
    let fit_explicit = a
        .fit
        .as_deref()
        .map(str::parse::<cut_media::render::Fit>)
        .transpose()
        .map_err(|e| {
            CutError::new(error_codes::INVALID_ARGS, "unknown fit mode", e)
                .with_suggested_action("valid fit modes: contain (default), cover")
        })?;
    let mut resolution = a
        .resolution
        .as_deref()
        .map(str::parse::<cut_media::render::Resolution>)
        .transpose()
        .map_err(|e| {
            CutError::new(error_codes::INVALID_ARGS, "unknown resolution mode", e)
                .with_suggested_action("valid resolution modes: project (default), match_source")
        })?
        .unwrap_or_default();
    // Explicit output geometry for THIS render (reframe / multi-format publish):
    // `aspect` ("9:16","16:9","1:1","4:5",…) OR `width`+`height`. Sets
    // Resolution::Explicit, leaving the project untouched. Mutually exclusive
    // with `resolution` (project/match_source) — both would be ambiguous.
    let override_geo: Option<(u32, u32)> = match (a.aspect.as_deref(), a.width, a.height) {
        (None, None, None) => None,
        (Some(asp), None, None) => Some(dims_from_aspect(asp)?),
        (None, Some(w), Some(h)) => Some((w, h)),
        (Some(_), _, _) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "pass either aspect OR width+height, not both",
                "aspect is a shorthand that computes width/height for you",
            ))
        }
        (None, _, _) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "width and height must be given together",
                "an output frame needs both dimensions",
            ))
        }
    };
    if let Some((w, h)) = override_geo {
        if a.resolution.is_some() {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "aspect/width/height conflict with resolution",
                "explicit geometry replaces the resolution mode; pass one or the other",
            ));
        }
        if !(16..=7680).contains(&w) || !(16..=7680).contains(&h) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("output geometry {w}x{h} is out of range"),
                "width/height sit in 16..=7680 px",
            ));
        }
        resolution = cut_media::render::Resolution::Explicit {
            width: w,
            height: h,
        };
    }
    // fit default: cover when reframing to explicit geometry, else contain.
    let fit = fit_explicit.unwrap_or(
        if matches!(resolution, cut_media::render::Resolution::Explicit { .. }) {
            cut_media::render::Fit::Cover
        } else {
            cut_media::render::Fit::default()
        },
    );
    // Loudness normalization target (LUFS) — validated NOW (fails fast, not
    // after a long encode). Sane broadcast/streaming range; outside it is a
    // typo or a unit mistake. None = no normalization (byte-identical replay).
    let loudness_target =
        match a.normalize_loudness {
            Some(t) if !(-40..=-5).contains(&t) => return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("normalize_loudness {t} LUFS is out of range"),
                "integrated-loudness targets sit in roughly -40..-5 LUFS",
            )
            .with_suggested_action(
                "use a standard target: -16 (long-form), -14 (social), -23 (EBU R128 broadcast)",
            )),
            other => other,
        };
    let render_opts = cut_media::render::RenderOptions {
        fit,
        resolution,
        loudness_target,
    };
    let (project, edl, dir, at_op) = snapshot(state).await?;
    // render_id = next receipts/render_*.json index (unique per project).
    let receipts = dir.join("receipts");
    std::fs::create_dir_all(&receipts)?;
    // Count only the CANONICAL receipt files `render_NNN.json` — NOT the
    // `render_NNN.output.perception.json` sidecar each render also writes (which
    // also starts with "render_" and ends ".json"). Counting both double-counted
    // every render, so ids skipped (001 → 003 → 005); this keeps them sequential.
    let (render_id, render_reservation) = if a.dry_run {
        (next_receipt_id_preview(&receipts, "render"), None)
    } else {
        let (id, marker) = reserve_receipt_id(&receipts, "render")?;
        (id, Some(marker))
    };
    // Quality tier (draft|standard|high) — validate up front so an unknown name
    // errors BEFORE any encode. It shifts the rate knob within the chosen codec.
    let quality = a.preset.as_deref().unwrap_or("standard");
    if cut_media::render::RenderPreset::named(quality).is_none() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown preset '{quality}'"),
            format!(
                "presets: {} (default standard)",
                cut_media::render::PRESET_NAMES.join("|")
            ),
        )
        .with_suggested_action("draft = fast review, standard = default, high = hero asset"));
    }
    // Output FORMAT (codec + container) — "different file exports". Default
    // h264/mp4 reuses the named preset verbatim → byte-identical to a no-`format`
    // render. The format sets the encoder args AND the file extension.
    let format = a.format.as_deref().unwrap_or("h264");
    let (mut video_args, audio_args, mut ext) = cut_media::render::format_codec_args(format, quality).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown format '{format}'"),
            format!("formats: {} (default h264)", cut_media::render::FORMAT_NAMES.join("|")),
        )
        .with_suggested_action(
            "h264 = universal mp4 (default), hevc = ~30-50% smaller, vp9 = web/webm, prores = pro/mov, av1 = highest quality (slow without a GPU)",
        )
    })?;
    // GPU/HARDWARE encoder tier: use the GPU when the
    // desktop has one). "auto" (default) swaps in the detected HW encoder for the
    // codec — much faster (and AV1-HW is the quality ceiling); "off" forces the
    // software encoder. Only h264/hevc/av1 have a HW tier; vp9/prores stay
    // software. The HW encoder is PROBE-VERIFIED (a real test encode), so a swap
    // never lands on a non-working encoder. The receipt records what actually ran.
    let hw_mode = a.hardware.as_deref().unwrap_or("auto");
    let base_codec = match format {
        "h264" | "mp4" => "h264",
        "hevc" | "h265" => "hevc",
        "av1" => "av1",
        _ => "",
    };
    let mut encoder = video_arg_value(&video_args, "-c:v"); // software default label
    if hw_mode != "off" && !base_codec.is_empty() {
        let q = match quality {
            "draft" => 0usize,
            "high" => 2,
            _ => 1,
        };
        if let Some((hw_video, hw_ext)) = cut_media::hwencode::hw_codec_args(base_codec, q) {
            encoder = video_arg_value(&hw_video, "-c:v");
            video_args = hw_video;
            ext = hw_ext;
            tracing::info!(format, encoder = %encoder, "render using hardware encoder");
        }
    }
    // BITRATE / RATE CONTROL: platform-specific publishing needs a real
    // bitrate target, not just CRF). When `bitrate` is set, rewrite the codec's
    // rate knob from quality-targeted (CRF/CQ) to VBR/CBR at that target. Parsed
    // + validated NOW (typo fails fast, before the job spawns / any encode). The
    // rewrite is encoder-aware, so it works after the HW swap too. Omitted =
    // unchanged (the byte-identical CRF path). The receipt records the target.
    let cbr = match a.rate_control.as_deref() {
        None | Some("vbr") => false,
        Some("cbr") => true,
        Some(other) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown rate_control '{other}'"),
                "rate_control is vbr (default) or cbr",
            )
            .with_suggested_action(
                "omit for vbr, or pass rate_control:\"cbr\" for constant bitrate",
            ))
        }
    };
    let mut audio_args = audio_args; // rebind: audio bitrate override may rewrite it
                                     // Audio rate used both for the actual encode AND the target-size budget
                                     // (the override, else the format default 192k AAC).
    let audio_kbps_budget = a
        .audio_bitrate
        .as_deref()
        .and_then(cut_media::render::parse_bitrate_kbps)
        .unwrap_or(192);
    // Resolve the VIDEO bitrate target from EITHER `bitrate` (explicit) OR
    // `target_size_mb` (computed to fit the file under that size) — mutually
    // exclusive. None = quality-targeted CRF (byte-identical replay).
    let bitrate_label: Option<String> = match (a.bitrate.as_deref(), a.target_size_mb) {
        (Some(_), Some(_)) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "pass bitrate OR target_size_mb, not both",
                "they both set the video rate; target_size_mb computes it for you",
            ));
        }
        (Some(br), None) => {
            let kbps = cut_media::render::parse_bitrate_kbps(br).ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("invalid bitrate '{br}'"),
                    "bitrate is e.g. \"12M\", \"8000k\", or a bare kbps number (50..500000 kbps)",
                )
            })?;
            video_args = cut_media::render::apply_bitrate(video_args, kbps, cbr, &encoder);
            Some(format!("{kbps}k {}", if cbr { "cbr" } else { "vbr" }))
        }
        (None, Some(mb)) => {
            // total budget kbps = MB·8·1024 / duration_sec; reserve audio; 90%
            // headroom (VBR variance + container overhead) so the file lands UNDER.
            let kbps = target_size_video_kbps(mb, edl.duration_ms, audio_kbps_budget)?;
            // VBR-under (cbr would pad UP to the rate, defeating "stay under").
            video_args = cut_media::render::apply_bitrate(video_args, kbps, false, &encoder);
            Some(format!("≤{mb}MB→{kbps}k vbr"))
        }
        (None, None) => {
            if a.rate_control.is_some() {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "rate_control needs a bitrate target",
                    "rate_control (vbr/cbr) only applies when `bitrate` is set",
                )
                .with_suggested_action("pass bitrate (e.g. \"12M\") alongside rate_control"));
            }
            None
        }
    };
    if let Some(ab) = a.audio_bitrate.as_deref() {
        let akbps = cut_media::render::parse_bitrate_kbps(ab).ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("invalid audio_bitrate '{ab}'"),
                "audio_bitrate is e.g. \"384k\" or \"192k\"",
            )
        })?;
        audio_args = cut_media::render::set_audio_bitrate(audio_args, akbps);
    }
    let out = fence_output_path(
        &dir,
        a.path.as_deref(),
        &format!("exports/{render_id}.{ext}"),
    )?;
    // The receipt records the encoder that actually produced the output (e.g.
    // "standard" for the default software h264, "standard/hevc_nvenc" for a HW
    // HEVC render) — honest about software vs hardware + the exact codec.
    let base_name = if matches!(format, "h264" | "mp4") && encoder == "libx264" {
        quality.to_string() // byte-identical default keeps the bare name
    } else {
        format!("{quality}/{encoder}")
    };
    let preset = cut_media::render::RenderPreset {
        // A bitrate target is recorded on the receipt name (honest about the
        // exact encode); without it the bare name is preserved (byte-identical).
        name: match &bitrate_label {
            Some(b) => format!("{base_name} @{b}"),
            None => base_name,
        },
        video_args,
        audio_args,
    };
    // dry_run (workspace contract v1): return the render plan — output geometry, realized
    // duration, per-track manifest, the checks that will run — WITHOUT encoding,
    // so an agent can verify what will render (and diff against the receipt
    // afterward) before paying for a slow encode. No job, no op, no file write.
    if a.dry_run {
        let (w, h) = render_opts.output_geometry(&project, &edl);
        let tracks: Vec<serde_json::Value> = project
            .tracks
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "kind": format!("{:?}", t.kind).to_lowercase(),
                    "clips": t.clips.len(),
                })
            })
            .collect();
        // The canonical battery that verify.checks will run after a real render
        // (cut_on_beat is appended only for a music-bed edit; footage_profile is
        // metadata). Names mirror cut_core::check_names.
        use cut_core::check_names as cn;
        let checks = [
            cn::CUT_ON_WORD,
            cn::LUFS,
            cn::CAPTION_PRESENCE,
            cn::BLACK_OR_FROZEN_FRAMES,
            cn::UNIFORM_BORDER,
            cn::SILENCE_AT_EDGES,
            cn::DURATION_MATCHES_EDL,
        ];
        return Ok(VerbResult::ok(json!({
            "dry_run": true,
            "render_id": render_id, // the id a real render would claim next
            "at_op": at_op,
            "out_path": out,
            "preset": preset.name,
            "fit": render_opts.fit.as_str(),
            "resolution": render_opts.resolution.as_str(),
            "normalize_loudness": render_opts.loudness_target,
            "output": {
                "width": w, "height": h,
                "fps": project.settings.fps,
                "duration_ms": edl.duration_ms,
            },
            "segment_count": edl.segments.len(),
            "tracks": tracks,
            "checks": checks,
        })));
    }
    // Capture the already-authorized fence before the async job starts. A
    // one-off Save As UI temporarily selects the file's parent as the session
    // output root and restores the persistent default immediately after this
    // dispatch returns. Rebuilding the fence inside the task would therefore
    // revoke the user's explicit choice before ffmpeg opens the file.
    let output_fence = make_fence(&dir)?;
    let job = state.jobs.create("render");
    let job_id = job.job_id.clone();
    let render_id_out = render_id.clone(); // result copy — task moves the original
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn_limited(&job_id, "render", RENDER_MAX_RUNNING, async move {
        let jid = job.job_id.clone();
        st.jobs.progress(&jid, 0.01, Some("rendering".into()));
        // Progress callback bridges the render thread into job events.
        let st2 = st.clone();
        let jid2 = jid.clone();
        let on_progress: cut_media::render::ProgressFn =
            Box::new(move |f| st2.jobs.progress(&jid2, 0.01 + f * 0.84, None));
        let (p2, e2, o2, d2, fence) = (
            project.clone(),
            edl.clone(),
            out.clone(),
            dir.clone(),
            output_fence,
        );
        let rendered = run_blocking("render.final", move || {
            // The captured fence is the request-time authorization snapshot;
            // render_final still re-fences the resolved output against it.
            // BAKE nests (compound clips): render each nest's sub-timeline to a
            // content-addressed cache file and add it as a synthetic source asset, so
            // the renderer resolves the nest clips' source (mirrors the matte bake).
            // No-op CLONE when the project has no nest → the (p2, e2) below stay the
            // exact snapshot pair, so a non-nest render is byte-identical.
            let (p2, e2) = crate::nest::flatten_for_media_io(&p2, &e2, &d2)?;
            cut_media::render::render_final(
                &p2,
                &e2,
                &fence,
                &o2,
                &preset,
                render_opts,
                Some(on_progress),
            )
        })
        .await;
        let output = match rendered {
            Ok(o) => o,
            Err(e) => {
                st.events.publish(Event::RenderDone {
                    job_id: jid.clone(),
                    render_id: render_id.clone(),
                    ok: false,
                    path: None,
                });
                return st.jobs.fail(&jid, e);
            }
        };
        // AUTO-run the check battery (public verb contract render.final): perception over
        // the OUTPUT + source transcripts feed cut_perception::run_all.
        st.jobs.progress(&jid, 0.9, Some("running checks".into()));
        let (o2, rd, rid, h) = (
            output.path.clone(),
            receipts.clone(),
            render_id.clone(),
            output.hash.clone(),
        );
        let dur = output.duration_ms;
        let facts = run_blocking("verify.checks (output perception)", move || {
            let report = cut_perception::run_instruments(
                &o2,
                &rd,
                &format!("{rid}.output"),
                &h,
                cut_perception::InstrumentSet::RenderChecks,
                None,
            )?;
            Ok(cut_perception::RenderFacts {
                duration_ms: dur, // ffprobe truth from RenderOutput
                loudness: report.loudness.clone(),
                output_report: Some(report),
            })
        })
        .await;
        let (facts, checks_degraded) = match facts {
            Ok(facts) => (facts, None),
            Err(error) => (
                cut_perception::RenderFacts {
                    duration_ms: dur,
                    loudness: None,
                    output_report: None,
                },
                Some(error),
            ),
        };
        // Source transcripts (real reads; missing files just skip). These and
        // the EDL still prove structural checks when output perception is not
        // available; output-dependent checks receive missing facts and fail
        // explicitly in the persisted degraded receipt.
        let mut owned: Vec<cut_perception::Transcript> = Vec::new();
        for (aid, asset) in project.assets.iter() {
            if asset.transcript.is_some() {
                let p = dir.join(format!("receipts/{aid}.words.json"));
                if let Ok(s) = std::fs::read_to_string(&p) {
                    if let Ok(t) = serde_json::from_str(&s) {
                        owned.push(t);
                    }
                }
            }
        }
        // Beat grids for music-bed assets (`cut_on_beat` receipt).
        let base_audio = project
            .tracks
            .iter()
            .find(|t| t.kind == cut_core::TrackKind::Audio && !t.clips.is_empty())
            .map(|t| t.id.clone());
        let music_assets: std::collections::BTreeSet<String> = project
            .tracks
            .iter()
            .filter(|t| t.kind == cut_core::TrackKind::Audio && Some(&t.id) != base_audio.as_ref())
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                cut_core::Clip::Media(m) => Some(m.asset.clone()),
                _ => None,
            })
            .collect();
        let mut beats: Vec<(String, cut_perception::BeatGrid)> = Vec::new();
        for aid in &music_assets {
            if let Ok(Some(rep)) = cut_perception::load_report(&receipts, aid) {
                if let Some(grid) = rep.beats {
                    beats.push((aid.clone(), grid));
                }
            }
        }
        let p2 = project.clone();
        let e2 = edl.clone();
        let checks = run_blocking("verify.checks", move || {
            let refs: Vec<&cut_perception::Transcript> = owned.iter().collect();
            Ok(cut_perception::run_all_with_profile(
                &p2, &e2, &refs, &facts, &beats, profile,
            ))
        })
        .await;
        match checks {
            Ok(mut checks) => {
                if let Some(error) = checks_degraded.as_ref() {
                    mark_output_checks_unmeasured(&mut checks, error);
                }
                let mut receipt = cut_core::RenderReceipt {
                    render_id: render_id.clone(),
                    ts: OpRecord::now_ts(),
                    output_path: output.path.display().to_string(),
                    output_hash: output.hash.clone(),
                    duration_ms: output.duration_ms,
                    preset: output.preset.clone(),
                    at_op,
                    checks,
                    pass: false,
                    judge: None,
                    fix_actions: vec![], // populated by compute_pass()
                };
                receipt.compute_pass();
                let rpath = receipts.join(format!("{render_id}.json"));
                if let Err(e) = std::fs::write(
                    &rpath,
                    serde_json::to_string_pretty(&receipt).unwrap_or_default(),
                ) {
                    return st.jobs.fail(
                        &jid,
                        CutError::new(
                            error_codes::IO,
                            "failed to persist render receipt",
                            format!("{}: {e}", rpath.display()),
                        )
                        .with_suggested_action(
                            "check free disk space and project receipt-directory permissions",
                        ),
                    );
                }
                if let Some(marker) = &render_reservation {
                    let _ = std::fs::remove_file(marker);
                }
                // the event-ordering contract ordering: render_done THEN receipt_ready.
                st.events.publish(Event::RenderDone {
                    job_id: jid.clone(),
                    render_id: render_id.clone(),
                    ok: true,
                    path: Some(output.path.display().to_string()),
                });
                st.events.publish(Event::ReceiptReady {
                    receipt: receipt.clone(),
                });
                let mut result = json!({
                    "render_id": render_id,
                    "path": output.path,
                    "receipt": rpath,
                    "pass": receipt.pass,
                    "verified": checks_degraded.is_none(),
                    "verification_status": if checks_degraded.is_some() { "unmeasured" } else { "complete" },
                });
                if let Some(error) = checks_degraded {
                    result["checks_skipped"] = json!(format!(
                        "output-dependent checks unmeasured: {}: {}",
                        error.code, error.message
                    ));
                    result["verification_error"] = json!(error);
                    st.jobs.finish_with_warnings(&jid, result);
                } else {
                    st.jobs.finish(&jid, result);
                }
            }
            Err(e) => {
                // The render succeeded, but even the deterministic receipt
                // computation failed. Preserve the usable artifact as unverified;
                // unlike the output-analysis fallback above, there is no trustworthy
                // partial receipt to persist.
                st.events.publish(Event::RenderDone {
                    job_id: jid.clone(),
                    render_id: render_id.clone(),
                    ok: true,
                    path: Some(output.path.display().to_string()),
                });
                let summary = format!("{}: {}", e.code, e.message);
                st.jobs.finish_with_warnings(
                    &jid,
                    json!({
                        "render_id": render_id,
                        "path": output.path,
                        "receipt": null,
                        "verified": false,
                        "verification_status": "unmeasured",
                        "verification_error": e,
                        "checks_skipped": summary,
                    }),
                );
            }
        }
    });
    // Contract (verbs.json): {job_id, render_id} — the receipt arrives via
    // receipt_ready / verify.checks keyed on render_id.
    Ok(VerbResult::ok(
        json!({"job_id": job_id, "render_id": render_id_out}),
    ))
}

/// clip.candidates{asset?, count?, min_ms?, max_ms?} — rank the windows most
/// likely to work as standalone short-form clips (social repurposing). Pure
/// READ-ONLY analysis over the asset transcript(s); no op, no render. Honest
/// heuristic v1 (scoring:"heuristic" — an editorial prior on hook + retention,
/// NOT a trained virality model); every candidate carries a `reason`. With no
/// `asset`, pools across every asset that has a transcript and re-ranks globally.
pub(super) async fn clip_candidates(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        asset: Option<String>,
        count: Option<usize>,
        min_ms: Option<u64>,
        max_ms: Option<u64>,
    }
    let a: Args = parse_args(args)?;
    let opts = cut_perception::CandidateOpts {
        count: a.count.unwrap_or(5).clamp(1, 50),
        min_ms: a.min_ms.unwrap_or(12_000),
        max_ms: a.max_ms.unwrap_or(60_000),
    };
    if opts.max_ms <= opts.min_ms {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "max_ms must be greater than min_ms",
            format!("got min_ms={} max_ms={}", opts.min_ms, opts.max_ms),
        ));
    }
    let fillers: Vec<String> = speech_text::FILLERS.iter().map(|s| s.to_string()).collect();
    // Which assets to scan: the named one, else everything with a transcript.
    let assets: Vec<String> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        match a.asset {
            Some(id) => vec![id],
            None => store
                .project
                .assets
                .iter()
                .filter(|(_, asset)| asset.transcript.is_some())
                .map(|(k, _)| k.clone())
                .collect(),
        }
    };
    if assets.is_empty() {
        return Ok(VerbResult::ok(json!({
            "candidates": [], "count": 0, "scoring": "heuristic",
            "note": "no assets with a transcript yet — media.import + transcribe first",
        })));
    }
    let mut all: Vec<cut_perception::ClipCandidate> = Vec::new();
    for id in &assets {
        // load_transcript errors if an asset has no transcript; tolerate + skip.
        if let Ok(t) = load_transcript(state, id).await {
            all.extend(cut_perception::clip_candidates(&t, &fillers, opts));
        }
    }
    all.sort_by(|x, y| {
        y.score
            .partial_cmp(&x.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all.truncate(opts.count);
    Ok(VerbResult::ok(json!({
        "candidates": all,
        "count": all.len(),
        "scoring": "heuristic",
        "scoring_note": "editorial prior (opening-hook strength + pacing/filler retention proxy), \
                         not a trained virality model — read each candidate's `reason` and override freely",
    })))
}

/// SRT/VTT timestamp of `ms` (SRT uses ',' before millis; VTT uses '.').
fn fmt_ts(ms: u64, comma: bool) -> String {
    let (h, m, s, milli) = (
        ms / 3_600_000,
        (ms % 3_600_000) / 60_000,
        (ms % 60_000) / 1000,
        ms % 1000,
    );
    let sep = if comma { ',' } else { '.' };
    format!("{h:02}:{m:02}:{s:02}{sep}{milli:03}")
}

/// Build SRT + VTT for the caption cues overlapping the timeline window
/// [t0, t1), each rebased to clip-zero and clamped to the window. Returns
/// (srt, vtt, cue_count). A bundle clip carries ITS OWN captions, windowed —
/// not the full timeline's track.
fn bundle_caption_files(project: &cut_core::Project, t0: u64, t1: u64) -> (String, String, usize) {
    let mut cues: Vec<(u64, u64, String)> = project
        .tracks
        .iter()
        .filter(|tr| tr.kind == cut_core::TrackKind::Caption)
        .flat_map(|tr| tr.clips.iter())
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => {
                let [s, e] = cc.range_ms;
                if e <= t0 || s >= t1 || cc.text.trim().is_empty() {
                    None
                } else {
                    Some((s.max(t0) - t0, e.min(t1) - t0, cc.text.clone()))
                }
            }
            _ => None,
        })
        .collect();
    cues.sort_by_key(|c| c.0);
    let mut srt = String::new();
    let mut vtt = String::from("WEBVTT\n\n");
    for (i, (s, e, text)) in cues.iter().enumerate() {
        srt.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            fmt_ts(*s, true),
            fmt_ts(*e, true),
            text.trim()
        ));
        vtt.push_str(&format!(
            "{} --> {}\n{}\n\n",
            fmt_ts(*s, false),
            fmt_ts(*e, false),
            text.trim()
        ));
    }
    (srt, vtt, cues.len())
}

pub(super) fn write_bundle_caption_sidecars(
    plat_dir: &Path,
    srt: &str,
    vtt: &str,
    cue_count: usize,
) -> (Option<String>, Option<String>, Option<String>) {
    if cue_count == 0 {
        return (None, None, None);
    }
    let srt_path = plat_dir.join("clip.srt");
    let vtt_path = plat_dir.join("clip.vtt");
    let mut errors = Vec::new();
    let caption_path = match std::fs::write(&srt_path, srt) {
        Ok(()) => Some(srt_path.display().to_string()),
        Err(e) => {
            errors.push(format!("{}: {e}", srt_path.display()));
            None
        }
    };
    let vtt_path_out = match std::fs::write(&vtt_path, vtt) {
        Ok(()) => Some(vtt_path.display().to_string()),
        Err(e) => {
            errors.push(format!("{}: {e}", vtt_path.display()));
            None
        }
    };
    let error = (!errors.is_empty()).then(|| errors.join("; "));
    (caption_path, vtt_path_out, error)
}

/// A render.bundle clip's receipt: the OUTPUT-fact check subset that is
/// meaningful for a STANDALONE published clip — loudness on target, no baked-in
/// border (the reframe is the riskiest step), no black/frozen frames, clean
/// edges. The EDL-relative checks (cut_on_word, duration_matches_edl) are NOT
/// run here: they verify timeline-edit integrity on the SOURCE render, not a
/// reframed window (render_range trims the composed graph, the EDL stays full,
/// so duration_matches_edl would false-fail every window). Persisted like any
/// receipt; carries fix_actions (e.g. uniform_border → render.final{fit:cover}).
fn compute_bundle_receipt(
    facts: &cut_perception::RenderFacts,
    output: &cut_media::render::RenderOutput,
    render_id: &str,
    at_op: &str,
    loudness_target: i32,
    receipts_dir: &Path,
) -> Result<cut_core::RenderReceipt, CutError> {
    let checks = vec![
        cut_perception::checks::lufs(facts, loudness_target as f64, 2.0),
        cut_perception::checks::uniform_border(facts),
        cut_perception::checks::black_or_frozen_frames(facts),
        cut_perception::checks::silence_at_edges(facts, 500),
    ];
    let mut receipt = cut_core::RenderReceipt {
        render_id: render_id.to_string(),
        ts: OpRecord::now_ts(),
        output_path: output.path.display().to_string(),
        output_hash: output.hash.clone(),
        duration_ms: output.duration_ms,
        preset: output.preset.clone(),
        at_op: at_op.to_string(),
        checks,
        pass: false,
        judge: None,
        fix_actions: vec![],
    };
    receipt.compute_pass();
    let rpath = receipts_dir.join(format!("{render_id}.json"));
    std::fs::write(
        &rpath,
        serde_json::to_string_pretty(&receipt).unwrap_or_default(),
    )
    .map_err(|e| {
        CutError::new(
            error_codes::IO,
            "failed to persist bundle receipt",
            format!("{}: {e}", rpath.display()),
        )
    })?;
    Ok(receipt)
}

/// render.reframe{aspect, preset?, path?} — SUBJECT-AWARE auto-reframe (perception contract
/// reframe rework). Renders the timeline to a TEMP at project geometry (the
/// "finished edit"), runs the `subject` perception instrument on it (local CV:
/// detect+track+saliency → a normalized subject track), then drives a
/// subject-tracked moving-crop POST-PASS (cut_media::render::reframe_video) to the
/// target aspect. The original render is untouched; the receipt reports honesty
/// metrics (subject-in-frame %, the device it analyzed on, "reframe ≠ lossless").
///
/// This is the HONEST replacement for render.final{aspect, fit:cover} — that is a
/// naive STATIC centre-crop (loses the sides, ignores the subject); this FOLLOWS
/// the subject with a smoothed pan. Returns {reframe_id, job_id} (the background-job contract); the
/// output path + receipt land in the job result.
/// Director-model path: render the finished edit to a temp and
/// build the SPARSE per-scene contact sheet the foundation model reads to direct
/// the whole clip in one pass. Returns a job whose result carries the contact-sheet
/// image path + per-scene candidate subjects (labeled A/B/C left→right). The agent
/// looks at the sheet, then calls render.reframe with a per-scene `direction` brief.
pub(super) async fn render_direct(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        preset: Option<String>, // class set: talking_head|sports|pets|cars|general
        #[allow(dead_code)]
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let preset = a.preset.clone().unwrap_or_else(|| "talking_head".into());

    let (project, edl, dir, _at_op) = snapshot_for_media_io(state, "render.direct.nests").await?;
    if edl.duration_ms == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "timeline is empty — nothing to direct",
            "insert at least one clip before reframe.direct",
        ));
    }
    let receipts = dir.join("receipts");
    std::fs::create_dir_all(&receipts)?;
    std::fs::create_dir_all(dir.join("exports"))?; // the draft temp renders here
    let n = std::fs::read_dir(&receipts)?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("direct_"))
        .count();
    let direct_id = format!("direct_{:03}", n + 1);

    let job = state.jobs.create("reframe-direct");
    let job_id = job.job_id.clone();
    let direct_id_ret = direct_id.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn_limited(&job_id, "render", RENDER_MAX_RUNNING, async move {
        let jid = job.job_id.clone();
        // 1) Render the finished edit to a TEMP (the director reads the cut, not the
        //    raw clips), then 2) build the contact sheet from it.
        st.jobs.progress(&jid, 0.05, Some("rendering edit".into()));
        let temp = dir.join(format!("exports/{direct_id}.src.mp4"));
        let (p2, e2, t2, d2) = (project.clone(), edl.clone(), temp.clone(), dir.clone());
        let (sa, ja) = (st.clone(), jid.clone());
        let prog1: cut_media::render::ProgressFn =
            Box::new(move |f| sa.jobs.progress(&ja, 0.05 + f * 0.5, None));
        let rendered = run_blocking("direct.render", move || {
            let fence = make_fence(&d2)?;
            cut_media::render::render_final(
                &p2,
                &e2,
                &fence,
                &t2,
                &cut_media::render::RenderPreset::default(),
                cut_media::render::RenderOptions::default(),
                Some(prog1),
            )
        })
        .await;
        if let Err(e) = rendered {
            let _ = std::fs::remove_file(&temp);
            return st.jobs.fail(&jid, e);
        }

        st.jobs
            .progress(&jid, 0.6, Some("building contact sheet".into()));
        let cs_dir = receipts.join(format!("{direct_id}.contact"));
        let (t3, csd, pr) = (temp.clone(), cs_dir.clone(), preset.clone());
        let sheet = run_blocking("direct.contact", move || {
            cut_perception::build_contact_sheet(&t3, &csd, &pr)
        })
        .await;
        let _ = std::fs::remove_file(&temp); // draft no longer needed
        let sheet = match sheet {
            Ok(s) => s,
            Err(e) => return st.jobs.fail(&jid, e),
        };
        // Copy the sheet into the served frames/ dir so the UI can fetch it over
        // HTTP (receipts/ is not served). The absolute path is still returned for a
        // local agent that reads files directly.
        let sheet_url = sheet
            .get("contact_sheet")
            .and_then(|v| v.as_str())
            .and_then(|src| {
                let frames = dir.join("frames");
                let _ = std::fs::create_dir_all(&frames);
                let name = format!("{direct_id}.contact.jpg");
                std::fs::copy(src, frames.join(&name))
                    .ok()
                    .map(|_| format!("/frames/{name}"))
            });
        st.jobs.finish(
            &jid,
            json!({
                "direct_id": direct_id,
                "contact_sheet": sheet.get("contact_sheet"),
                "contact_sheet_url": sheet_url,
                "scene_count": sheet.get("scene_count"),
                "scenes": sheet.get("scenes"),
                "preset": preset,
                "note": "read the contact_sheet image, then call render.reframe{aspect, direction:{scene: {cx}|{mode:\"widen\"}}} — cx is each candidate's normalized x from scenes[].candidates",
            }),
        );
    });
    Ok(VerbResult::ok(
        json!({"job_id": job_id, "direct_id": direct_id_ret}),
    ))
}

/// Director-model QC: review a reframed OUTPUT.
/// Builds a per-scene review sheet (frames + composition hints: subject present,
/// face centering, headroom, `needs_review`) for the model to judge "wrong subject /
/// bad framing" and re-issue a corrected `render.reframe{direction}`. The model's
/// vision is the real judge; the hints focus its attention.
pub(super) fn resolve_reframe_output_for_qc(dir: &Path, rid: &str) -> Result<PathBuf, CutError> {
    let suggested = "run render.reframe first, then pass the returned reframe_id";
    let mut candidates = Vec::new();
    let receipt_path = dir.join(format!("receipts/{rid}.json"));
    if let Ok(text) = std::fs::read_to_string(&receipt_path) {
        if let Ok(receipt) = serde_json::from_str::<Value>(&text) {
            if let Some(path) = receipt.get("output_path").and_then(|v| v.as_str()) {
                if !path.trim().is_empty() {
                    candidates.push(PathBuf::from(path));
                }
            }
        }
    }
    candidates.push(dir.join(format!("exports/{rid}.mp4")));

    let fence = make_fence(dir)?;
    let mut causes = Vec::new();
    for candidate in candidates {
        match fence.fence_output_path(&candidate) {
            Ok(path) if path.is_file() => return Ok(path),
            Ok(path) => causes.push(format!("{} is not a file", path.display())),
            Err(err) => causes.push(format!("{}: {}", candidate.display(), err.message)),
        }
    }

    Err(CutError::new(
        error_codes::INVALID_ARGS,
        "no such reframe output to QC",
        if causes.is_empty() {
            format!("no receipt output_path or exports/{rid}.mp4 was found")
        } else {
            causes.join("; ")
        },
    )
    .with_suggested_action(suggested))
}

pub(super) async fn render_qc(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        reframe_id: String, // the reframe output to review (receipt output_path or legacy default)
        preset: Option<String>, // candidate class set (same as render.reframe)
        #[allow(dead_code)]
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let rid = a.reframe_id.trim().to_string();
    if rid.is_empty() || rid.contains('/') || rid.contains('\\') || rid.contains("..") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "render.qc needs a valid reframe_id",
            "pass the reframe_id returned by render.reframe (e.g. reframe_001)",
        ));
    }
    let preset = a.preset.clone().unwrap_or_else(|| "talking_head".into());
    let (_project, _edl, dir, _at_op) = snapshot(state).await?;
    let output = resolve_reframe_output_for_qc(&dir, &rid)?;
    let receipts = dir.join("receipts");
    std::fs::create_dir_all(&receipts)?;
    let rid_ret = rid.clone();
    let job = state.jobs.create("reframe-qc");
    let job_id = job.job_id.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn_limited(&job_id, "analysis", ANALYSIS_MAX_RUNNING, async move {
        let jid = job.job_id.clone();
        st.jobs.progress(&jid, 0.1, Some("reviewing output".into()));
        let qc_dir = receipts.join(format!("{rid}.qc"));
        let (o2, q2, p2) = (output.clone(), qc_dir.clone(), preset.clone());
        let sheet = run_blocking("reframe.qc", move || {
            cut_perception::build_qc_sheet(&o2, &q2, &p2)
        })
        .await;
        let sheet = match sheet {
            Ok(s) => s,
            Err(e) => return st.jobs.fail(&jid, e),
        };
        // Serve the QC sheet via frames/ for the UI (receipts/ is not served).
        let sheet_url = sheet
            .get("qc_sheet")
            .and_then(|v| v.as_str())
            .and_then(|src| {
                let frames = dir.join("frames");
                let _ = std::fs::create_dir_all(&frames);
                let name = format!("{rid}.qc.jpg");
                std::fs::copy(src, frames.join(&name))
                    .ok()
                    .map(|_| format!("/frames/{name}"))
            });
        st.jobs.finish(
            &jid,
            json!({
                "reframe_id": rid,
                "qc_sheet": sheet.get("qc_sheet"),
                "qc_sheet_url": sheet_url,
                "scene_count": sheet.get("scene_count"),
                "review_count": sheet.get("review_count"),
                "scenes": sheet.get("scenes"),
                "note": "read the qc_sheet image; for any needs_review scene re-issue render.reframe{aspect, direction:{\"<scene>\":{\"cx\":<a better candidate from render.direct>}|{\"mode\":\"widen\"}}}",
            }),
        );
    });
    Ok(VerbResult::ok(
        json!({"job_id": job_id, "reframe_id": rid_ret}),
    ))
}

pub(super) async fn render_reframe(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        aspect: String,         // target ratio: "9:16" | "1:1" | "4:5" | "16:9" …
        preset: Option<String>, // reframe preset: talking_head|sports|pets|cars|general
        path: Option<String>,   // optional explicit output path (fenced)
        // Director brief: per-scene framing decision from the
        // foundation model — {scene_idx: {"cx": 0..1} | {"mode": "widen"}}. From
        // reframe.direct's contact sheet. Omit for pure CV ranker framing.
        #[allow(dead_code)]
        direction: Option<Value>,
    }
    let a: Args = parse_args(args)?;
    if a.aspect.trim().is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "reframe needs a target aspect",
            "pass aspect, e.g. \"9:16\"",
        )
        .with_suggested_action("aspect: 9:16 (vertical), 1:1 (square), 4:5 (portrait)"));
    }
    // Output dims (even, base 1080) — also the crop ratio (base_crop reads w/h).
    let (ow, oh) = dims_from_aspect(&a.aspect)?;
    let reframe_preset = a.preset.clone().unwrap_or_else(|| "talking_head".into());
    let params = cut_media::reframe::ReframeParams::for_preset(&reframe_preset);

    let (project, edl, dir, at_op) = snapshot_for_media_io(state, "render.reframe.nests").await?;
    if edl.duration_ms == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "timeline is empty — nothing to reframe",
            "insert at least one clip before render.reframe",
        ));
    }
    let receipts = dir.join("receipts");
    std::fs::create_dir_all(&receipts)?;
    let n = std::fs::read_dir(&receipts)?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("reframe_"))
        .count();
    let render_id = format!("reframe_{:03}", n + 1);
    let out = fence_output_path(&dir, a.path.as_deref(), &format!("exports/{render_id}.mp4"))?;

    let job = state.jobs.create("reframe");
    let job_id = job.job_id.clone();
    let render_id_ret = render_id.clone(); // result copy — the task moves the original
    let st = state.clone();
    let aspect = a.aspect.clone();
    let direction = a.direction.clone(); // director brief (moved into the task)
    let jobs = state.jobs.clone();
    jobs.spawn_limited(&job_id, "render", RENDER_MAX_RUNNING, async move {
        let jid = job.job_id.clone();
        // 1) Render the finished edit to a TEMP at project geometry (no crop).
        st.jobs.progress(&jid, 0.01, Some("rendering edit".into()));
        let temp = dir.join(format!("exports/{render_id}.src.mp4"));
        let (p2, e2, t2, d2) = (project.clone(), edl.clone(), temp.clone(), dir.clone());
        let (sa, ja) = (st.clone(), jid.clone());
        let prog1: cut_media::render::ProgressFn =
            Box::new(move |f| sa.jobs.progress(&ja, 0.01 + f * 0.44, None));
        let rendered = run_blocking("reframe.render", move || {
            let fence = make_fence(&d2)?;
            cut_media::render::render_final(
                &p2,
                &e2,
                &fence,
                &t2,
                &cut_media::render::RenderPreset::default(),
                cut_media::render::RenderOptions::default(),
                Some(prog1),
            )
        })
        .await;
        let rendered = match rendered {
            Ok(o) => o,
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                return st.jobs.fail(&jid, e);
            }
        };

        // 2) Subject instrument on the rendered edit → SubjectTrack. `run_subject`
        // passes the reframe preset (class selection — fixes it never reaching the
        // instrument) + the optional director brief (per-scene framing override).
        st.jobs
            .progress(&jid, 0.5, Some("analysing subject".into()));
        let (t3, h) = (temp.clone(), rendered.hash.clone());
        let (preset_c, dir_c) = (reframe_preset.clone(), direction.clone());
        let track = run_blocking("reframe.subject", move || {
            cut_perception::run_subject(&t3, &h, &preset_c, dir_c)
        })
        .await;
        let track = match track {
            Ok(t) => t,
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                return st.jobs.fail(&jid, e);
            }
        };

        // 3) Map SubjectTrack → FrameObs, then the moving-crop post-pass.
        st.jobs.progress(&jid, 0.6, Some("reframing".into()));
        let frames: Vec<cut_media::reframe::FrameObs> = track
            .frames
            .iter()
            .map(|sf| cut_media::reframe::FrameObs {
                focus: match (sf.fx1, sf.fy1, sf.fx2, sf.fy2) {
                    (Some(x1), Some(y1), Some(x2), Some(y2)) => {
                        Some([x1 as f64, y1 as f64, x2 as f64, y2 as f64])
                    }
                    _ => None,
                },
                conf: sf.conf as f64,
                scene: sf.scene,
            })
            .collect();
        let scene_starts = track.scenes.clone();
        let total = frames.len().max(1);
        let n_subj = frames.iter().filter(|f| f.focus.is_some()).count();
        let in_frame_pct = (1000.0 * n_subj as f64 / total as f64).round() / 10.0;
        let device = track.device.clone();
        let subject_fps = track.fps;
        let subject_fps_source = track.fps_source.clone();
        let subject_fps_warning = track.fps_warning.clone();
        let speaker_aware = track.speaker_aware; // audio-gated active-speaker applied?
        let face_aware = track.face_aware; // face/eye-line framing available?
        let directed_scenes = track.directed_scenes.clone(); // director-decided scenes
        let (t4, o4) = (temp.clone(), out.clone());
        let (sb, jb) = (st.clone(), jid.clone());
        let prog2: cut_media::render::ProgressFn =
            Box::new(move |f| sb.jobs.progress(&jb, 0.6 + f * 0.35, None));
        let reframed = run_blocking("reframe.crop", move || {
            cut_media::render::reframe_video(
                &t4,
                &o4,
                &frames,
                ow,
                oh,
                ow,
                oh,
                &params,
                &scene_starts,
                &cut_media::render::RenderPreset::default(),
                Some(prog2),
            )
        })
        .await;
        let _ = std::fs::remove_file(&temp); // intermediate no longer needed
        let reframed = match reframed {
            Ok(o) => o,
            Err(e) => return st.jobs.fail(&jid, e),
        };

        // 4) Honest receipt : subject-in-frame %, device, "reframe ≠ lossless".
        let mut receipt = json!({
            "reframe_id": render_id,
            "ts": OpRecord::now_ts(),
            "output_path": reframed.path.display().to_string(),
            "output_hash": reframed.hash,
            "duration_ms": reframed.duration_ms,
            "aspect": aspect,
            "preset": reframe_preset,
            "subject_in_frame_pct": in_frame_pct,
            "analyzed_on": device,
            "subject_fps": subject_fps,
            "subject_fps_source": subject_fps_source,
            "active_speaker": speaker_aware,
            "face_aware": face_aware,
            "directed_scenes": directed_scenes,
            "framed_by": if directed_scenes.is_empty() { "ranker" } else { "director+ranker" },
            "at_op": at_op,
            "honest_note": "reframe is a LOSSY subject-tracked crop, not a lossless conversion — content outside the moving crop window is discarded. v1 = constant-zoom smoothed pan; composed titles/graphics outside the crop are not yet repositioned.",
            "framing_note": match (face_aware, speaker_aware) {
                (true, true) => "framing follows the face eye-line and prefers the active speaker (audio-gated mouth motion) in multi-person dialogue",
                (true, false) => "framing follows the face eye-line / salient subject (no audio ⇒ active-speaker cue unavailable)",
                (false, true) => "framing follows body/saliency centers and prefers the active speaker when mouth motion is measurable (face detector unavailable)",
                (false, false) => "framing follows body/saliency centers (face detector unavailable; no audio ⇒ active-speaker cue unavailable)",
            },
        });
        if let Some(warning) = subject_fps_warning {
            receipt["subject_fps_warning"] = json!(warning);
        }
        let rpath = receipts.join(format!("{render_id}.json"));
        let receipt_text = match serde_json::to_string_pretty(&receipt) {
            Ok(text) => text,
            Err(e) => {
                return st.jobs.fail(
                    &jid,
                    CutError::new(
                        error_codes::IO,
                        "failed to serialize reframe receipt",
                        e.to_string(),
                    ),
                )
            }
        };
        if let Err(e) = std::fs::write(&rpath, receipt_text) {
            return st.jobs.fail(
                &jid,
                CutError::new(
                    error_codes::IO,
                    "failed to persist reframe receipt",
                    format!("{}: {e}", rpath.display()),
                ),
            );
        }
        st.events.publish(Event::RenderDone {
            job_id: jid.clone(),
            render_id: render_id.clone(),
            ok: true,
            path: Some(reframed.path.display().to_string()),
        });
        st.jobs.finish(
            &jid,
            json!({
                "reframe_id": render_id,
                "path": reframed.path,
                "receipt": rpath,
                "subject_in_frame_pct": in_frame_pct,
                "active_speaker": speaker_aware,
                "face_aware": face_aware,
            }),
        );
    });

    Ok(VerbResult::ok(
        json!({"reframe_id": render_id_ret, "job_id": job_id, "aspect": a.aspect}),
    ))
}

/// render.bundle{range_ms?|candidate?, platforms?, preset?, normalize_loudness?,
/// brand_ref?} — the social repurposing capstone: render ONE timeline window
/// into a publish-ready pack per platform (reframe via render_range with the
/// aspect geometry + fit:cover), each with windowed captions (srt+vtt), a
/// thumbnail, and a receipt. the background-job contract: returns {job_id, bundle_id}; the pack
/// payload lands in the job result. Default platforms 9:16 + 1:1 + 16:9; default
/// loudness -14 LUFS (social). `candidate:{at_ms,dur_ms}` is treated as a
/// timeline window (the Clips panel maps source→timeline before calling).
pub(super) async fn render_bundle(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct CandRange {
        at_ms: u64,
        dur_ms: u64,
    }
    #[derive(serde::Deserialize, Default)]
    struct Args {
        range_ms: Option<[u64; 2]>,
        candidate: Option<CandRange>,
        platforms: Option<Vec<String>>,
        preset: Option<String>,
        normalize_loudness: Option<i32>,
        brand_ref: Option<cut_core::BrandKit>,
    }
    let a: Args = parse_args(args)?;
    let (project, edl, dir, at_op) = snapshot_for_media_io(state, "render.bundle.nests").await?;
    let total = edl.duration_ms;
    if total == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "timeline is empty — nothing to bundle",
            "import + place at least one clip first",
        ));
    }
    // Resolve the window (timeline coords): explicit range, candidate, or whole.
    let mut range = match (a.range_ms, a.candidate.as_ref()) {
        (Some(r), _) => r,
        (None, Some(c)) => [c.at_ms, c.at_ms.saturating_add(c.dur_ms)],
        (None, None) => [0, total],
    };
    range[1] = range[1].min(total);
    if range[1] <= range[0] {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "range is empty after clamping to the timeline",
            format!("got [{}, {}) within {} ms", range[0], range[1], total),
        ));
    }
    // Platforms → validated geometry (fail fast on a bad aspect, before the job).
    let platforms = a
        .platforms
        .clone()
        .unwrap_or_else(|| vec!["9:16".to_string(), "1:1".to_string(), "16:9".to_string()]);
    let dims: Vec<(String, (u32, u32))> = platforms
        .iter()
        .map(|p| dims_from_aspect(p).map(|d| (p.clone(), d)))
        .collect::<Result<_, _>>()?;
    let preset = match a.preset.as_deref() {
        None => cut_media::render::RenderPreset::default(),
        Some(name) => cut_media::render::RenderPreset::named(name).ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown preset '{name}'"),
                format!("presets: {}", cut_media::render::PRESET_NAMES.join("|")),
            )
        })?,
    };
    let loud = a.normalize_loudness.unwrap_or(-14); // social default
    let bundle_id = format!("bundle_{}_{}", range[0], range[1]);
    let (brand, brand_source) = if let Some(explicit) = a.brand_ref {
        (
            Some(super::brand::normalize_brand(explicit, "explicit")?),
            Some("explicit"),
        )
    } else if let Some(stored) = project.brand.clone() {
        (
            Some(super::brand::normalize_brand(stored, "stored")?),
            Some("stored"),
        )
    } else {
        (None, None)
    };

    let job = state.jobs.create("bundle");
    let job_id = job.job_id.clone();
    let bundle_id_out = bundle_id.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn_limited(&job_id, "render", RENDER_MAX_RUNNING, async move {
        let jid = job.job_id.clone();
        let receipts = dir.join("receipts");
        let _ = std::fs::create_dir_all(&receipts);
        let n = dims.len().max(1);
        let mut platforms_out: Vec<Value> = Vec::new();
        let mut receipt_ids: Vec<String> = Vec::new();
        for (i, (aspect, (w, h))) in dims.iter().enumerate() {
            let frac0 = i as f32 / n as f32;
            st.jobs.progress(
                &jid,
                0.02 + frac0 * 0.9,
                Some(format!("rendering {aspect}")),
            );
            let safe = aspect.replace(':', "x");
            let rel = format!("exports/{bundle_id}/{safe}/clip.mp4");
            let out_path = match fence_output_path(&dir, None, &rel) {
                Ok(p) => p,
                Err(e) => return st.jobs.fail(&jid, e),
            };
            if let Some(parent) = out_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let opts = cut_media::render::RenderOptions {
                fit: cut_media::render::Fit::Cover,
                resolution: cut_media::render::Resolution::Explicit {
                    width: *w,
                    height: *h,
                },
                loudness_target: Some(loud),
            };
            let (p2, e2, o2, d2, rng, pr) = (
                project.clone(),
                edl.clone(),
                out_path.clone(),
                dir.clone(),
                range,
                preset.clone(),
            );
            let rendered = run_blocking("render.bundle", move || {
                let fence = make_fence(&d2)?;
                cut_media::render::render_range(&p2, &e2, &fence, &o2, &pr, rng, opts, None)
            })
            .await;
            let output = match rendered {
                Ok(o) => o,
                Err(e) => return st.jobs.fail(&jid, e),
            };
            // Output-fact receipt for this platform clip.
            let render_id = format!("{bundle_id}_{safe}");
            let (o2, rd, rid, hh) = (
                output.path.clone(),
                receipts.clone(),
                render_id.clone(),
                output.hash.clone(),
            );
            let dur = output.duration_ms;
            let facts = run_blocking("render.bundle (perception)", move || {
                // RenderChecks = silence + scenes + loudness (no whisperX): the
                // bundle receipt only inspects output facts, never the output's
                // words — skipping transcribe keeps per-clip perception ~1-2s.
                let report = cut_perception::run_instruments(
                    &o2,
                    &rd,
                    &format!("{rid}.output"),
                    &hh,
                    cut_perception::InstrumentSet::RenderChecks,
                    None,
                )?;
                Ok(cut_perception::RenderFacts {
                    duration_ms: dur,
                    loudness: report.loudness.clone(),
                    output_report: Some(report),
                })
            })
            .await;
            let (receipt_id, pass, receipt_persist_failed): (
                Option<String>,
                Option<bool>,
                Option<String>,
            ) = match facts {
                Ok(f) => {
                    match compute_bundle_receipt(&f, &output, &render_id, &at_op, loud, &receipts) {
                        Ok(r) => {
                            receipt_ids.push(render_id.clone());
                            (Some(render_id.clone()), Some(r.pass), None)
                        }
                        Err(e) => (None, None, Some(e.cause)),
                    }
                }
                // Render is fine; perception sidecar absent → unverified clip.
                Err(_) => (None, None, None),
            };
            // Windowed captions (srt + vtt) for this clip.
            let (srt, vtt, cue_count) = bundle_caption_files(&project, range[0], range[1]);
            let plat_dir = out_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| dir.clone());
            let (caption_path, vtt_path, caption_write_failed) =
                write_bundle_caption_sidecars(&plat_dir, &srt, &vtt, cue_count);
            let (caption_hash, caption_hash_error) =
                optional_artifact_hash(caption_path.as_deref());
            let (vtt_hash, vtt_hash_error) = optional_artifact_hash(vtt_path.as_deref());
            // Thumbnail at the clip midpoint, extracted from the rendered mp4.
            let thumb_path = plat_dir.join("thumb.jpg");
            let mid_s = (output.duration_ms as f64 / 2000.0).max(0.0);
            let thumb_ok = cut_media::ffmpeg::run_ffmpeg(&[
                "-ss".into(),
                format!("{mid_s:.3}"),
                "-i".into(),
                output.path.display().to_string(),
                "-frames:v".into(),
                "1".into(),
                "-q:v".into(),
                "3".into(),
                thumb_path.display().to_string(),
            ])
            .is_ok();
            let thumb = thumb_ok.then(|| thumb_path.display().to_string());
            let (thumb_hash, thumb_hash_error) = optional_artifact_hash(thumb.as_deref());
            let artifact_hash_failed = [caption_hash_error, vtt_hash_error, thumb_hash_error]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let artifact_hash_failed =
                (!artifact_hash_failed.is_empty()).then(|| artifact_hash_failed.join("; "));
            platforms_out.push(json!({
                "aspect": aspect,
                "width": w, "height": h,
                "path": output.path.display().to_string(),
                "hash": output.hash,
                "caption_path": caption_path,
                "caption_hash": caption_hash,
                "vtt_path": vtt_path,
                "vtt_hash": vtt_hash,
                "caption_count": cue_count,
                "caption_write_failed": caption_write_failed,
                "thumb": thumb,
                "thumb_hash": thumb_hash,
                "artifact_hash_failed": artifact_hash_failed,
                "receipt_id": receipt_id,
                "pass": pass,
                "receipt_persist_failed": receipt_persist_failed,
                "duration_ms": output.duration_ms,
            }));
        }
        // Optional brand conformance (client-deliverable path).
        let brand_result = brand
            .as_ref()
            .zip(brand_source)
            .map(|(spec, source)| super::brand::check_bundle_brand(&project, spec, &dims, source));
        let assessment = assess_publish_package(&platforms_out, brand_result.as_ref());
        let manifest = publish_package_manifest(
            &bundle_id,
            range,
            &at_op,
            &platforms_out,
            brand_result.as_ref(),
            &assessment,
        );
        let manifest_path = dir.join("exports").join(&bundle_id).join("manifest.json");
        let manifest_bytes = match serde_json::to_vec_pretty(&manifest) {
            Ok(bytes) => bytes,
            Err(error) => {
                return st.jobs.fail(
                    &jid,
                    CutError::new(
                        error_codes::IO,
                        "could not serialize publish-package manifest",
                        error.to_string(),
                    ),
                )
            }
        };
        if let Err(error) = write_output_atomic(&manifest_path, manifest_bytes) {
            return st.jobs.fail(&jid, error);
        }
        let manifest_hash = match cut_core::hash_file(&manifest_path) {
            Ok(hash) => hash,
            Err(error) => return st.jobs.fail(&jid, error),
        };
        let warnings = assessment
            .issues
            .iter()
            .map(|issue| match issue.aspect.as_deref() {
                Some(aspect) => format!("{aspect}: {}", issue.detail),
                None => issue.detail.clone(),
            })
            .collect::<Vec<_>>();
        let result = json!({
            "bundle_id": bundle_id,
            "range_ms": range,
            "platforms": platforms_out,
            "receipt_ids": receipt_ids,
            "status": assessment.status,
            "pass": assessment.pass,
            "issues": assessment.issues,
            "warnings": warnings,
            "brand": brand_result,
            "manifest_path": manifest_path.display().to_string(),
            "manifest_hash": manifest_hash,
        });
        if assessment.pass {
            st.jobs.finish(&jid, result);
        } else {
            st.jobs.finish_with_warnings(&jid, result);
        }
    });
    Ok(VerbResult::ok(
        json!({"job_id": job_id, "bundle_id": bundle_id_out}),
    ))
}

/// render.queue{jobs:[{output?|path?, …render.final args}], rationale?} — BATCH
/// DELIVERY: a batch render queue. Fan the
/// ONE current timeline out into N renders, each with its own delivery settings
/// (output, format, preset, bitrate, geometry, loudness, …), every one running
/// through the SAME render.final path (job + segmented OOM-bounded encode + the
/// auto verify.checks → RenderReceipt).
///
/// PURE DELIVERY ORCHESTRATOR (the autopilot.run / recipe.run mould): it records
/// NO op of its own and makes NO timeline mutation — it only dispatches
/// render.final per entry and reuses the existing job infrastructure. NO
/// checkpoint (render.final writes only artifacts; there is nothing to revert).
///
/// SEQUENTIAL, not concurrent — a deliberate, documented choice. Render is
/// memory-heavy and render.final already bounds peak RSS WITHIN one render
/// (adaptive segmentation + a Linux cgroup soft-limit). Running N renders AT ONCE
/// would multiply peak memory by N and defeat that governance (and thrash the
/// encoder/GPU). So the queue starts the next render only after the prior one
/// reaches a terminal state — the safe, predictable batch behavior a deliver
/// page has. Entries are INDEPENDENT deliveries: a failed one is recorded and the
/// queue CONTINUES (a render queue marks that job failed and moves on), never
/// aborting the whole batch.
///
/// `output` is accepted as the deliver-page alias for render.final's `path`.
/// Each entry is otherwise a render.final arg subset, VALIDATED UP FRONT by a
/// render.final dry_run per entry — a bad entry (unknown arg, bad
/// profile/format/geometry) fails the WHOLE queue HERE, before a single encode
/// starts, naming the offending index. The dry_run also resolves each entry's
/// planned output path for the synchronous return.
///
/// Returns {queue_id, count, jobs:[{idx, output}]} immediately. The queue job's
/// result (jobs.status{queue_id}) fills in per-entry {idx, job_id, render_id,
/// output, ok, pass, receipt, error?} as each render completes — one poll shows
/// the whole batch, and each render's job_id is then individually pollable
/// (jobs.status). the background-job contract (a job-returning orchestrator). Requires an open project.
pub(super) async fn render_queue(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        #[serde(default)]
        jobs: Vec<Value>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    if a.jobs.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "render.queue needs a non-empty `jobs` array",
            "each entry is a render.final arg-set, e.g. jobs:[{output:\"a.mp4\"}, {output:\"b.mov\", format:\"prores\"}]",
        )
        .with_suggested_action("add at least one delivery; `output` aliases render.final's `path`"));
    }
    // Require an open project up front (fail fast, not inside the spawned queue).
    {
        let g = state.project.read().await;
        g.as_ref().ok_or_else(no_project)?;
    }
    // Normalize each entry to render.final args (map the deliver-page `output`
    // alias → render.final's `path`) and VALIDATE it via a render.final dry_run:
    // a bad entry (unknown arg, bad profile/format/geometry) fails the WHOLE queue
    // HERE — before a single encode runs — naming the offending idx. The dry_run
    // also returns each entry's resolved output path for the synchronous return.
    let mut render_args: Vec<Value> = Vec::new();
    let mut slots: Vec<Value> = Vec::new();
    for (idx, entry) in a.jobs.iter().enumerate() {
        let Some(obj) = entry.as_object() else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("render.queue job #{idx} is not an object"),
                "each entry is a render.final arg-set (a JSON object)",
            ));
        };
        // `output` → `path` (deliver-page vocabulary; render.final uses `path`).
        let mut m = obj.clone();
        if let Some(out) = m.remove("output") {
            if m.contains_key("path") {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("render.queue job #{idx} sets both `output` and `path`"),
                    "`output` is an alias for `path` — pass only one",
                ));
            }
            m.insert("path".into(), out);
        }
        let base = Value::Object(m);
        // dry_run validation: clone + force dry_run:true so render.final returns
        // the PLAN (resolved geometry + out_path) WITHOUT encoding.
        let mut probe = base.clone();
        if let Value::Object(p) = &mut probe {
            p.insert("dry_run".into(), json!(true));
        }
        let dr = dispatch_send(state, "render.final", probe, actor.clone()).await;
        if !dr.ok {
            // Surface the entry's own error, tagged with its queue index so the
            // caller knows WHICH delivery is malformed.
            let mut e = dr.error.unwrap_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "render.final rejected the entry",
                    "render.queue entry failed dry_run validation",
                )
            });
            e.message = format!("render.queue job #{idx}: {}", e.message);
            return Err(e);
        }
        let output = dr
            .result
            .as_ref()
            .and_then(|r| r.get("out_path"))
            .cloned()
            .unwrap_or(Value::Null);
        slots.push(json!({"idx": idx, "output": output}));
        render_args.push(base);
    }
    let count = render_args.len();

    let queue = state.jobs.create("render_queue");
    let queue_id = queue.job_id.clone();
    let queue_id_out = queue_id.clone();
    let slots_out = slots.clone();
    let rationale = a.rationale.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    // The queue is an orchestrator that waits on child render.final jobs. It must
    // not hold the "render" permit while those children wait for that same permit.
    jobs.spawn_limited(
        &queue_id,
        "render_queue",
        RENDER_QUEUE_MAX_RUNNING,
        async move {
            let jid = queue.job_id.clone();
            let mut results: Vec<Value> = Vec::new();
            let mut succeeded = 0usize;
            let mut failed = 0usize;
            for (idx, rargs) in render_args.into_iter().enumerate() {
                st.jobs.progress(
                    &jid,
                    0.02 + (idx as f32 / count as f32) * 0.96,
                    Some(format!("rendering delivery {}/{count}", idx + 1)),
                );
                // SAME path render.final uses (its handler + its own job-spawn) — no
                // render logic duplicated here.
                let rr = dispatch_send(&st, "render.final", rargs, actor.clone()).await;
                let render_job = rr
                    .ok
                    .then(|| {
                        rr.result
                            .as_ref()
                            .and_then(|r| r.get("job_id"))
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    })
                    .flatten();
                let render_id = rr
                    .result
                    .as_ref()
                    .and_then(|r| r.get("render_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let Some(render_job) = render_job else {
                    // render.final refused to even start (dry_run already validated, so
                    // this is a rare race — e.g. the project closed mid-queue). Record
                    // and move on to keep the batch going.
                    failed += 1;
                    results.push(json!({"idx": idx, "ok": false, "error": rr.error}));
                    continue;
                };
                // Poll this render to a terminal state — the NEXT delivery starts only
                // after this returns (the sequential guarantee). 30-min cap per entry.
                match poll_sub_job(&st, &render_job, 1_800_000).await {
                    Ok(jr) => {
                        let res = jr.result.clone().unwrap_or(Value::Null);
                        let output = res.get("path").cloned().unwrap_or(Value::Null);
                        let receipt = res.get("receipt").cloned().unwrap_or(Value::Null);
                        succeeded += 1;
                        let mut row = json!({
                            "idx": idx,
                            "job_id": render_job,
                            "render_id": render_id,
                            "output": output,
                            "ok": true,
                            "receipt": receipt,
                        });
                        // `pass` is present only when the auto-check battery ran
                        // (a sidecar-less box renders but reports verified:false).
                        if let Some(p) = res.get("pass") {
                            row["pass"] = p.clone();
                        }
                        results.push(row);
                    }
                    Err(e) => {
                        failed += 1;
                        results.push(json!({
                            "idx": idx,
                            "job_id": render_job,
                            "render_id": render_id,
                            "ok": false,
                            "error": e,
                        }));
                    }
                }
            }
            let summary = if failed == 0 {
                format!("render queue: {succeeded}/{count} delivered")
            } else {
                format!("render queue: {succeeded}/{count} delivered, {failed} failed")
            };
            st.jobs.finish(
                &jid,
                json!({
                    "summary_line": summary,
                    "count": count,
                    "succeeded": succeeded,
                    "failed": failed,
                    "jobs": results,
                    "rationale": rationale,
                }),
            );
        },
    );

    Ok(VerbResult::ok(json!({
        "queue_id": queue_id_out,
        "count": count,
        "jobs": slots_out,
    })))
}

/// `dispatch` as a boxed `Send` future. The autopilot calls verbs from inside a
/// `tokio::spawn`ed task; because `dispatch` is recursive-async (it can reach
/// `autopilot_run` again), its opaque future type can't be proven `Send`, which
/// `tokio::spawn` requires. Erasing it to `dyn Future + Send` breaks the type
/// cycle and asserts Send concretely.
pub(crate) fn dispatch_send<'a>(
    state: &'a AppState,
    name: &'a str,
    args: Value,
    actor: Actor,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = VerbResult> + Send + 'a>> {
    Box::pin(dispatch(state, name, args, actor))
}

/// Read a persisted receipt into the typed RenderReceipt (with fix_actions).
pub(crate) fn read_receipt(
    receipts_dir: &Path,
    render_id: Option<&str>,
) -> Result<cut_core::RenderReceipt, CutError> {
    let path = resolve_receipt_path(receipts_dir, render_id)?;
    let text = std::fs::read_to_string(&path)?;
    serde_json::from_str(&text).map_err(|e| {
        CutError::new(
            error_codes::JOB_FAILED,
            "receipt parse failed",
            e.to_string(),
        )
    })
}

/// autopilot.run{goal?, comment_id?, policy?, max_fix_iters?} — the Receipted
/// Autopilot (receipted workflow): render → verify → mechanically self-fix from the typed
/// receipt fix_actions → re-verify, capped, all under ONE auto-checkpoint so the
/// whole run reverts in one step.
///
/// The self-fix loop is the deterministic loop — no model in the loop. Each
/// failing check already carries a typed FixAction: a render-parameter fix
/// (lufs→normalize_loudness, uniform_border→fit:cover) is merged into the NEXT
/// render's args; a timeline-edit fix (cut_on_word→edit.trim snap-to-word-edge,
/// silence_at_edges→edit.trim_edges, caption overlap→captions.reflow) is applied
/// as its verb. The loop stops on a green receipt, when nothing more is
/// auto-fixable, or at max_fix_iters.
///
/// policy: "preview" (DEFAULT) renders + verifies once and returns the PLAN (what
/// it WOULD fix) WITHOUT applying — the user approves first; "auto_low_risk" runs
/// the self-fix loop applying the mechanical fixes. comment_id pre-applies that
/// review note's drafted changes (comment.apply) before the loop. Returns
/// {job_id, checkpoint}; the clean report lands in the job result. Job (the background-job contract).
pub(super) async fn autopilot_run(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        goal: Option<String>,
        comment_id: Option<String>,
        policy: Option<String>,
        max_fix_iters: Option<u32>,
    }
    let a: Args = parse_args(args)?;
    let policy = a.policy.unwrap_or_else(|| "preview".into());
    if !matches!(policy.as_str(), "preview" | "auto_low_risk") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown policy '{policy}'"),
            "valid: preview (default) | auto_low_risk",
        ));
    }
    let max_iters = a.max_fix_iters.unwrap_or(3).clamp(1, 6);
    // Require an open project up front (fail fast, not inside the job).
    {
        let g = state.project.read().await;
        g.as_ref().ok_or_else(no_project)?;
    }
    // Auto-checkpoint: the whole run reverts to here in one edit.restore/revert.
    let goal_label = a.goal.clone().unwrap_or_else(|| "autopilot polish".into());
    let cp = Box::pin(dispatch(
        state,
        "project.checkpoint",
        json!({"name": "autopilot-start", "rationale": format!("autopilot: {goal_label}")}),
        actor.clone(),
    ))
    .await;
    if !cp.ok {
        return Err(cp.error.unwrap_or_else(|| {
            CutError::new(
                error_codes::JOB_FAILED,
                "autopilot checkpoint failed",
                "project.checkpoint",
            )
        }));
    }
    let cp_obj = cp.result.unwrap_or(Value::Null);
    let start_checkpoint = cp_obj
        .get("checkpoint")
        .and_then(|c| c.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let start_at_op = cp_obj
        .get("checkpoint")
        .and_then(|c| c.get("at_op"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Optional: pre-apply a review note's drafted changes before the loop.
    if let Some(cid) = &a.comment_id {
        let apply = Box::pin(dispatch(
            state,
            "comment.apply",
            json!({"comment_id": cid}),
            actor.clone(),
        ))
        .await;
        if !apply.ok {
            return Err(apply.error.unwrap_or_else(|| {
                CutError::new(
                    error_codes::JOB_FAILED,
                    "autopilot comment.apply failed",
                    "comment.apply returned ok:false without an error",
                )
            }));
        }
        if apply
            .result
            .as_ref()
            .and_then(|r| r.get("status"))
            .and_then(|v| v.as_str())
            == Some("failed")
        {
            let result = apply.result.unwrap_or(Value::Null);
            return Err(CutError::new(
                error_codes::JOB_FAILED,
                format!("autopilot comment.apply failed for '{cid}'"),
                result.to_string(),
            )
            .with_suggested_action(
                "inspect the comment.apply result and revert its checkpoint if it partially applied",
            ));
        }
    }

    let job = state.jobs.create("autopilot");
    let job_id = job.job_id.clone();
    let start_checkpoint_out = start_checkpoint.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn(&job_id, async move {
        let jid = job.job_id.clone();
        let receipts_dir = {
            let g = st.project.read().await;
            match g.as_ref() {
                Some(s) => s.receipts_dir(),
                None => return st.jobs.fail(&jid, no_project()),
            }
        };
        // Render args accumulate render-param fixes (normalize_loudness, fit)
        // across iterations. Draft preset keeps the verify loop fast.
        let mut render_args = serde_json::Map::new();
        render_args.insert("preset".into(), json!("draft"));
        render_args.insert(
            "rationale".into(),
            json!(format!("autopilot render: {goal_label}")),
        );
        let mut fixes_applied: Vec<Value> = Vec::new();
        let mut receipt_ids: Vec<String> = Vec::new();
        let mut iters = 0u32;
        let mut final_pass = false;
        let mut last_plan: Vec<Value> = Vec::new();
        let mut unverified: Option<Value> = None;
        // No-progress guard state: the failing-check signature of the previous
        // pass, and whether the loop stopped because fixes weren't converging.
        let mut last_fail_sig: Option<Vec<String>> = None;
        let mut stalled = false;

        loop {
            st.jobs.progress(
                &jid,
                0.1 + (iters as f32 * 0.25).min(0.8),
                Some(format!("render + verify (pass {})", iters + 1)),
            );
            // 1. Render (a job) and wait for it.
            let rr = dispatch_send(
                &st,
                "render.final",
                Value::Object(render_args.clone()),
                actor.clone(),
            )
            .await;
            let render_job = match rr
                .ok
                .then(|| {
                    rr.result
                        .as_ref()
                        .and_then(|r| r.get("job_id"))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .flatten()
            {
                Some(j) => j,
                None => {
                    return st.jobs.fail(
                        &jid,
                        rr.error.unwrap_or_else(|| {
                            CutError::new(
                                error_codes::JOB_FAILED,
                                "autopilot render dispatch failed",
                                "render.final",
                            )
                        }),
                    )
                }
            };
            let render_id = match rr
                .result
                .as_ref()
                .and_then(|r| r.get("render_id"))
                .and_then(|v| v.as_str())
                .map(String::from)
            {
                Some(id) => id,
                None => {
                    return st.jobs.fail(
                        &jid,
                        CutError::new(
                            error_codes::JOB_FAILED,
                            "autopilot render did not return a render_id",
                            "render.final result was missing render_id",
                        ),
                    )
                }
            };
            // Poll the render job to completion (cap ~180s).
            let mut waited = 0u64;
            let render_result: Option<Value> = loop {
                match st.jobs.get(&render_job) {
                    Some(j) if matches!(j.state, crate::jobs::JobState::Done) => {
                        break j.result.clone();
                    }
                    Some(j) if matches!(j.state, crate::jobs::JobState::Failed) => {
                        return st.jobs.fail(
                            &jid,
                            j.error.unwrap_or_else(|| {
                                CutError::new(
                                    error_codes::JOB_FAILED,
                                    "autopilot render failed",
                                    &render_job,
                                )
                            }),
                        );
                    }
                    _ => {
                        if waited > 180_000 {
                            return st.jobs.fail(
                                &jid,
                                CutError::new(
                                    error_codes::JOB_FAILED,
                                    "autopilot render timed out",
                                    "render exceeded 180s",
                                ),
                            );
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        waited += 500;
                    }
                }
            };
            // 2. Read the fresh receipt for THIS render id + its fix_actions.
            // Never fall back to "latest": an unverified render writes no
            // receipt, and reading an older green receipt would fake a pass.
            let receipt = match read_receipt(&receipts_dir, Some(&render_id)) {
                Ok(r) => r,
                Err(e) if e.code == error_codes::NOT_FOUND => {
                    unverified = Some(json!({
                        "render_id": render_id,
                        "render_result": render_result,
                        "receipt_error": e,
                    }));
                    break;
                }
                Err(e) => return st.jobs.fail(&jid, e),
            };
            receipt_ids.push(receipt.render_id.clone());
            if receipt.pass {
                final_pass = true;
                break;
            }
            // The plan = the auto-fixable actions this receipt exposes.
            last_plan = receipt
                .fix_actions
                .iter()
                .map(|fa| json!({"check": fa.check, "fix_verb": fa.fix_verb, "auto_fixable": fa.auto_fixable, "rationale": fa.rationale}))
                .collect();
            // NO-PROGRESS GUARD: if we applied fixes last pass and the SAME set of
            // checks still fails, the fixes aren't converging (e.g. a caption
            // overlap reflow can't resolve, or a defect the mapped verb doesn't
            // actually repair). Stop honestly instead of burning every iteration
            // re-applying a fix that does nothing — and never fake a pass.
            let fail_sig: Vec<String> = {
                let mut v: Vec<String> = receipt
                    .checks
                    .iter()
                    .filter(|c| !c.pass)
                    .map(|c| c.name.clone())
                    .collect();
                v.sort();
                v
            };
            if iters > 0 && last_fail_sig.as_ref() == Some(&fail_sig) {
                stalled = true;
                break;
            }
            last_fail_sig = Some(fail_sig);
            // 3. PREVIEW policy: stop here and report the plan (don't apply).
            if policy == "preview" {
                break;
            }
            // 4. auto_low_risk: apply each auto-fixable fix mechanically.
            let mut applied = 0u32;
            for fa in &receipt.fix_actions {
                if !fa.auto_fixable {
                    continue;
                }
                match fa.fix_verb.as_str() {
                    // Render-param fixes: merge into the NEXT render's args.
                    "render.final" => {
                        if let Some(obj) = fa.fix_args.as_object() {
                            for (k, v) in obj {
                                render_args.insert(k.clone(), v.clone());
                            }
                            applied += 1;
                            fixes_applied.push(json!({"check": fa.check, "via": "render.final", "args": fa.fix_args}));
                        }
                    }
                    // cut_on_word: snap the offending boundary to the nearest word
                    // edge via edit.trim (translate boundary→src_in/out).
                    "edit.trim" => {
                        let clip = fa.fix_args.get("clip").cloned().unwrap_or(Value::Null);
                        let snap = fa
                            .fix_args
                            .get("snap_src_ms")
                            .cloned()
                            .unwrap_or(Value::Null);
                        let boundary = fa
                            .fix_args
                            .get("boundary")
                            .and_then(|v| v.as_str())
                            .unwrap_or("src_out");
                        if clip.is_null() || snap.is_null() {
                            continue;
                        }
                        let targs = if boundary == "src_in" {
                            json!({"clip": clip, "src_in_ms": snap, "rationale": "autopilot: snap cut to word edge"})
                        } else {
                            json!({"clip": clip, "src_out_ms": snap, "rationale": "autopilot: snap cut to word edge"})
                        };
                        let r = dispatch_send(&st, "edit.trim", targs, actor.clone()).await;
                        if r.ok {
                            applied += 1;
                            fixes_applied
                                .push(json!({"check": fa.check, "via": "edit.trim", "clip": clip}));
                        }
                    }
                    // No-arg timeline fixes. NB: edit.trim_edges + captions.reflow
                    // declare additionalProperties:false and do NOT accept a
                    // `rationale` — passing one is invalid_args and the fix would
                    // silently not apply. Dispatch with the bare {} they accept.
                    "edit.trim_edges" | "captions.reflow" => {
                        let r = dispatch_send(&st, &fa.fix_verb, json!({}), actor.clone()).await;
                        if r.ok {
                            applied += 1;
                            fixes_applied.push(json!({"check": fa.check, "via": fa.fix_verb}));
                        } else {
                            // Surface a failed fix (honest — never a silent skip
                            // that reads as "nothing to fix").
                            fixes_applied.push(json!({"check": fa.check, "via": fa.fix_verb, "failed": r.error.map(|e| e.message)}));
                        }
                    }
                    _ => {}
                }
            }
            iters += 1;
            // Nothing mechanically fixable, or out of iterations → stop.
            if applied == 0 || iters >= max_iters {
                break;
            }
        }

        // Clean report: what changed (diff start→tip) + verdict.
        let tip = snapshot(&st)
            .await
            .map(|(_, _, _, at)| at)
            .unwrap_or_default();
        let diff = dispatch_send(
            &st,
            "project.diff",
            json!({"from": start_at_op, "to": tip}),
            actor.clone(),
        )
        .await;
        let diff_res = diff.result.unwrap_or(Value::Null);
        let summary = if policy == "preview" {
            if last_plan.is_empty() {
                "All checks already pass — nothing to fix.".to_string()
            } else {
                format!(
                    "Preview: {} issue(s) fixable — approve (policy:auto_low_risk) to apply.",
                    last_plan.len()
                )
            }
        } else if final_pass {
            format!(
                "Done: {} fix(es) applied over {} pass(es), all checks pass.",
                fixes_applied.len(),
                iters + 1
            )
        } else if unverified.is_some() {
            "Stopped: render completed but no receipt was produced, so checks are unverified.".to_string()
        } else if stalled {
            format!("Stopped: applied {} fix(es) but the remaining checks did not improve (the mapped fix can't resolve them — needs a human or verify.judge).", fixes_applied.len())
        } else {
            format!("Stopped after {} pass(es): {} fix(es) applied, some checks still fail (see receipt).", iters, fixes_applied.len())
        };
        st.jobs.finish(
            &jid,
            json!({
                "summary_line": summary,
                "policy": policy,
                "goal": goal_label,
                "checks_pass": final_pass,
                "verified": unverified.is_none(),
                "unverified": unverified,
                "stalled": stalled,
                "iterations": iters,
                "fixes_applied": fixes_applied,
                "plan": last_plan,
                "changed": diff_res,
                "checkpoint": start_checkpoint,
                "receipt_ids": receipt_ids,
                "restore_hint": format!("edit.restore or project.revert to checkpoint {start_checkpoint} undoes the whole run"),
            }),
        );
    });
    Ok(VerbResult::ok(
        json!({"job_id": job_id, "checkpoint": start_checkpoint_out}),
    ))
}
