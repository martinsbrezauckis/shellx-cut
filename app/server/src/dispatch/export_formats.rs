use super::*;

/// The open project's timeline as the timeline/op-log contract JSON cut-export consumes,
/// with relative asset paths absolutized against the project dir (the XML's
/// file:// URIs must be relinkable from anywhere, not project-relative).
fn export_timeline_json(
    project: &cut_core::Project,
    project_dir: &Path,
) -> Result<Value, CutError> {
    let mut v = serde_json::to_value(project)?;
    if let Some(assets) = v.get_mut("assets").and_then(|a| a.as_object_mut()) {
        for asset in assets.values_mut() {
            if let Some(p) = asset.get("path").and_then(|p| p.as_str()) {
                if Path::new(p).is_relative() {
                    let abs = project_dir.join(p).display().to_string();
                    asset["path"] = json!(abs);
                }
            }
        }
    }
    Ok(v)
}

/// Map a cut-export serializer failure onto the actionable verb error
/// envelope. The crate is pure (no I/O) — fencing and writes stay here.
pub(super) fn export_error(e: cut_export::ExportError) -> CutError {
    use cut_export::ExportError as E;
    let (code, clip) = match &e {
        E::MissingAsset { clip_id, .. } => (error_codes::NOT_FOUND, Some(clip_id.clone())),
        E::EmptyTimeline | E::NoCaptions => (error_codes::NOT_FOUND, None),
        E::EmptyClip { clip_id, .. }
        | E::BadClip { clip_id, .. }
        | E::ResolveStream { clip_id, .. } => (error_codes::INVALID_ARGS, Some(clip_id.clone())),
        E::BadInput(_)
        | E::BadFormat(_)
        | E::BadFps(_)
        | E::BadSubtitle(_)
        | E::TimeOverflow { .. } => (error_codes::INVALID_ARGS, None),
    };
    let mut err = CutError::new(
        code,
        "export serializer refused the timeline",
        e.to_string(),
    );
    if let Some(c) = clip {
        err = err.with_clip(c);
    }
    err
}

#[derive(Clone, Copy)]
pub(super) enum ExportWarningTarget {
    Xml(cut_export::XmlFormat),
    Otio,
    Edl,
}

pub(super) fn export_richness_warnings(
    project: &cut_core::Project,
    target: ExportWarningTarget,
) -> Vec<cut_core::VerbWarning> {
    let mut dropped = std::collections::BTreeSet::<&'static str>::new();
    let media = || project.tracks.iter().flat_map(|t| t.clips.iter());

    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if !m.effects.is_empty())) {
        dropped.insert("effects");
    }
    if media().any(|c| {
        matches!(
            c,
            cut_core::Clip::Media(m)
                if m.grade.is_some() || !m.grade_stack.is_empty() || !m.grade_windows.is_empty()
        )
    }) {
        dropped.insert("grades");
    }
    if media().any(
        |c| matches!(c, cut_core::Clip::Media(m) if m.xfade_in_ms > 0 || m.xfade_kind.is_some()),
    ) {
        dropped.insert("transitions");
    }
    let drops_clip_gain = !matches!(target, ExportWarningTarget::Xml(cut_export::XmlFormat::Mlt));
    if drops_clip_gain && media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.gain_db != 0.0))
    {
        dropped.insert("clip gain");
    }
    if media().any(|c| {
        matches!(
            c,
            cut_core::Clip::Media(m)
                if (m.speed - 1.0).abs() > f64::EPSILON || m.speed_ramp.is_some()
        )
    }) {
        dropped.insert("speed changes");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.transform.is_some())) {
        dropped.insert("transforms");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.crop.is_some())) {
        dropped.insert("crops");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.fade.is_some())) {
        dropped.insert("fades");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.matte.is_some())) {
        dropped.insert("mattes");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.mask.is_some())) {
        dropped.insert("masks");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.reverse)) {
        dropped.insert("reverse playback");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.freeze.is_some())) {
        dropped.insert("freeze frames");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.animation.is_some())) {
        dropped.insert("animation");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if !m.keyframes.is_empty())) {
        dropped.insert("keyframes");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.eq.is_some())) {
        dropped.insert("audio EQ");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.stabilize.is_some())) {
        dropped.insert("stabilization");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.input_color_space.is_some())) {
        dropped.insert("input color-space tags");
    }
    if media().any(|c| matches!(c, cut_core::Clip::Media(m) if m.nest.is_some()))
        || !project.nests.is_empty()
    {
        dropped.insert("nests");
    }
    if !project.adjustments.is_empty() {
        dropped.insert("adjustment layers");
    }

    let overlay_tracks = project
        .tracks
        .iter()
        .filter(|t| t.kind == cut_core::TrackKind::Video && !t.clips.is_empty())
        .count()
        .saturating_sub(1);
    let drops_overlay_tracks = matches!(
        target,
        ExportWarningTarget::Edl
            | ExportWarningTarget::Xml(cut_export::XmlFormat::Fcpxml)
            | ExportWarningTarget::Xml(cut_export::XmlFormat::Resolve)
            | ExportWarningTarget::Xml(cut_export::XmlFormat::Mlt)
    );
    if overlay_tracks > 0 && drops_overlay_tracks {
        dropped.insert("overlay video tracks");
    }
    if matches!(target, ExportWarningTarget::Edl)
        && project
            .tracks
            .iter()
            .any(|t| t.kind == cut_core::TrackKind::Caption && !t.clips.is_empty())
    {
        dropped.insert("captions");
    }

    if dropped.is_empty() {
        return Vec::new();
    }

    let format = match target {
        ExportWarningTarget::Xml(cut_export::XmlFormat::Fcpxml) => "fcpxml",
        ExportWarningTarget::Xml(cut_export::XmlFormat::Resolve) => "resolve-fcpxml",
        ExportWarningTarget::Xml(cut_export::XmlFormat::Premiere) => "premiere-xmeml",
        ExportWarningTarget::Xml(cut_export::XmlFormat::Mlt) => "mlt",
        ExportWarningTarget::Otio => "otio",
        ExportWarningTarget::Edl => "edl",
    };
    let dropped_vec: Vec<&str> = dropped.iter().copied().collect();
    let mut detail = serde_json::Map::new();
    detail.insert("format".into(), json!(format));
    detail.insert("dropped".into(), json!(dropped_vec));

    vec![cut_core::VerbWarning {
        code: "richness_dropped".into(),
        message: format!(
            "{} export does not include {}; use render.final for a flattened delivery when these edits must be preserved",
            format,
            dropped.iter().copied().collect::<Vec<_>>().join(", ")
        ),
        detail,
    }]
}

/// export.xml{format, path?} — NLE interchange XML (fenced path, the output-fencing contract).
/// Serialization is owned by cut-export and checked against
/// serializers — structurally matches the known-good public fixtures and
/// frame-quantizes every time value by construction). Caption tracks are NOT
/// representable in the XML formats — surfaced as an in-band warning.
/// export.otio{path?} — write the timeline as OpenTimelineIO JSON, the
/// industry-standard interchange that round-trips with Resolve/Premiere/FCP. Mirrors
/// export.xml: snapshots the project, maps it to OTIO (cut_export::export_otio), and
/// fences the output path. Caption tracks aren't representable in OTIO → a warning.
pub(super) async fn export_otio(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        path: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (project, _edl, dir, _at) = snapshot_for_media_io(state, "export.otio.nests").await?;
    let out = fence_output_path(&dir, a.path.as_deref(), "exports/timeline.otio")?;
    let timeline = export_timeline_json(&project, &dir)?;
    let name = project.name.clone();
    let otio = run_blocking("export.otio", move || {
        cut_export::otio::export_otio(&timeline, &name).map_err(export_error)
    })
    .await?;
    write_output_atomic(&out, otio)?;
    let mut res = VerbResult::ok(json!({"path": out, "format": "otio"}));
    let mut warnings = export_richness_warnings(&project, ExportWarningTarget::Otio);
    let has_captions = project
        .tracks
        .iter()
        .any(|t| t.kind == cut_core::TrackKind::Caption && !t.clips.is_empty());
    if has_captions {
        warnings.push(cut_core::VerbWarning {
            code: "captions_not_in_otio".into(),
            message: "Caption tracks are not representable in OTIO and were omitted; export captions separately (export.srt/vtt/ass).".into(),
            detail: Default::default(),
        });
    }
    if !warnings.is_empty() {
        res = res.with_warnings(warnings);
    }
    Ok(res)
}
pub(super) async fn export_xml(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        format: String,
        path: Option<String>,
    }
    let a: Args = parse_args(args)?;
    // Verb enum is fcpxml|premiere|resolve (public verb contract); cut-export's stretch
    // "mlt" format stays off the verb surface until verbs.json lists it.
    if !matches!(a.format.as_str(), "fcpxml" | "premiere" | "resolve") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown export format '{}'", a.format),
            "must be fcpxml|premiere|resolve",
        ));
    }
    let format = cut_export::XmlFormat::from_str(&a.format).map_err(export_error)?;
    let (project, _edl, dir, _at) = snapshot_for_media_io(state, "export.xml.nests").await?;
    let default = format!("exports/timeline.{}", format.extension());
    let out = fence_output_path(&dir, a.path.as_deref(), &default)?;
    let timeline = export_timeline_json(&project, &dir)?;
    let xml = run_blocking("export.xml", move || {
        cut_export::export_xml(&timeline, format).map_err(export_error)
    })
    .await?;
    write_output_atomic(&out, xml)?;
    let mut res = VerbResult::ok(json!({"path": out, "format": a.format}));
    let mut warnings = export_richness_warnings(&project, ExportWarningTarget::Xml(format));
    let has_captions = project
        .tracks
        .iter()
        .any(|t| t.kind == cut_core::TrackKind::Caption && !t.clips.is_empty());
    if has_captions {
        warnings.push(cut_core::VerbWarning {
            code: "captions_not_in_xml".into(),
            message: cut_export::CAPTIONS_NOT_IN_XML_NOTE.into(),
            detail: Default::default(),
        });
    }
    if !warnings.is_empty() {
        res = res.with_warnings(warnings);
    }
    Ok(res)
}

/// export.edl{path?, title?} — write the timeline as a CMX3600 EDL, the
/// universal edit-decision-list interchange (Resolve/Premiere/Avid/FCP). Same
/// frame-quantized timeline as export.xml/otio, so the cuts line up to the
/// frame. EDL is a CUTS-ONLY format: transitions, effects, grades, per-clip
/// gain and captions cannot be represented and are dropped — when the project
/// carries any of those, a `richness_dropped` warning lists what was omitted so
/// the user is never silently surprised. Default <project>/exports/timeline.edl;
/// caller path FENCED (the output-fencing contract).
pub(super) async fn export_edl(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        path: Option<String>,
        title: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (project, _edl, dir, _at) = snapshot_for_media_io(state, "export.edl.nests").await?;
    let out = fence_output_path(&dir, a.path.as_deref(), "exports/timeline.edl")?;
    // Default the sequence title to the project folder name (…/<name>.cutproj).
    let title = a.title.clone().unwrap_or_else(|| {
        dir.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("ShellX Cut")
            .to_string()
    });
    let timeline = export_timeline_json(&project, &dir)?;
    let warnings = export_richness_warnings(&project, ExportWarningTarget::Edl);
    let edl = run_blocking("export.edl", move || {
        cut_export::export_edl(&timeline, &title).map_err(export_error)
    })
    .await?;
    // event_count = the EDL's numbered edit events (one per clip per channel).
    let event_count = edl
        .lines()
        .filter(|l| l.contains("FROM CLIP NAME:"))
        .count();
    write_output_atomic(&out, edl)?;
    let mut res = VerbResult::ok(json!({"path": out, "event_count": event_count}));
    if !warnings.is_empty() {
        res = res.with_warnings(warnings);
    }
    Ok(res)
}

/// export.srt{path?} — the ONLY SRT exporter (the canonical-export contract); cut-export owns the
/// serialization (skip-rules + hour rollover + actionable no-captions error).
pub(super) async fn export_srt(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        path: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (project, _edl, dir, _at) = snapshot(state).await?;
    let out = fence_output_path(&dir, a.path.as_deref(), "exports/captions.srt")?;
    let timeline = export_timeline_json(&project, &dir)?;
    let srt = run_blocking("export.srt", move || {
        cut_export::export_srt(&timeline).map_err(export_error)
    })
    .await?;
    // Contract (verbs.json): {path, caption_count}. Count the serialized SRT
    // blocks — the serializer's skip rules (empty/zero-length clips) make
    // this the honest number, not the raw caption-track clip count.
    let caption_count = srt.split("\n\n").filter(|b| !b.trim().is_empty()).count();
    write_output_atomic(&out, srt)?;
    Ok(VerbResult::ok(
        json!({"path": out, "caption_count": caption_count}),
    ))
}

/// export.vtt{path?} — WebVTT captions (HTML5 <track> standard for web video).
/// Same caption track as export.srt; default <project>/exports/captions.vtt,
/// caller path FENCED (the output-fencing contract).
pub(super) async fn export_vtt(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        path: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (project, _edl, dir, _at) = snapshot(state).await?;
    let out = fence_output_path(&dir, a.path.as_deref(), "exports/captions.vtt")?;
    let timeline = export_timeline_json(&project, &dir)?;
    let vtt = run_blocking("export.vtt", move || {
        cut_export::export_vtt(&timeline).map_err(export_error)
    })
    .await?;
    // One cue per "-->" line (the serializer's skip rules already excluded
    // empty/zero-length clips); the WEBVTT header carries no arrow.
    let caption_count = vtt.matches("-->").count();
    write_output_atomic(&out, vtt)?;
    Ok(VerbResult::ok(
        json!({"path": out, "caption_count": caption_count}),
    ))
}

/// export.ass{path?, karaoke?} — write the caption track to an ASS/SSA file (the
/// portable STYLED-caption standard; SRT/VTT carry no styling). `karaoke:false`
/// (default) = one styled Dialogue per caption line. `karaoke:true` = WORD-LEVEL
/// `\k` fill (the TikTok/Hormozi look) — each line's words highlight in turn,
/// timed across the clip, with a dim SecondaryColour so the fill shows. Default
/// <project>/exports/captions.ass; caller path FENCED (the output-fencing contract). Round-trips
/// with captions.import (which now also parses .ass).
pub(super) async fn export_ass(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        path: Option<String>,
        karaoke: Option<bool>,
    }
    let a: Args = parse_args(args)?;
    let karaoke = a.karaoke.unwrap_or(false);
    let (project, _edl, dir, _at) = snapshot(state).await?;
    let out = fence_output_path(&dir, a.path.as_deref(), "exports/captions.ass")?;
    let ass = run_blocking("export.ass", move || {
        if karaoke {
            cut_media::captions_to_ass_karaoke(&project)
        } else {
            cut_media::captions_to_ass(&project)
        }
    })
    .await?;
    // One Dialogue line per caption cue (the serializers skip empty cues).
    let caption_count = ass.lines().filter(|l| l.starts_with("Dialogue:")).count();
    write_output_atomic(&out, ass)?;
    Ok(VerbResult::ok(
        json!({"path": out, "caption_count": caption_count, "karaoke": karaoke}),
    ))
}

/// export.transcript{format?, timestamps?, path?} — write the caption track as
/// a readable transcript (the script of the final cut) for show notes /
/// repurposing. format txt (default, plain paragraphs) | md (heading +
/// paragraphs, with `[m:ss]` prefixes when timestamps:true). Default path
/// <project>/exports/transcript.<ext>; caller path fenced (the output-fencing contract).
pub(super) async fn export_transcript(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        format: Option<String>,
        #[serde(default)]
        timestamps: bool,
        path: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let format = a
        .format
        .as_deref()
        .map(cut_export::transcript::TranscriptFormat::from_str)
        .transpose()
        .map_err(export_error)?
        .unwrap_or(cut_export::transcript::TranscriptFormat::Txt);
    let timestamps = a.timestamps;
    let (project, _edl, dir, _at) = snapshot(state).await?;
    let default_rel = format!("exports/transcript.{}", format.extension());
    let out = fence_output_path(&dir, a.path.as_deref(), &default_rel)?;
    let timeline = export_timeline_json(&project, &dir)?;
    let text = run_blocking("export.transcript", move || {
        cut_export::export_transcript(&timeline, format, timestamps).map_err(export_error)
    })
    .await?;
    // Paragraph count = blank-line-separated blocks of the body (drop the md
    // heading); a simple, honest "how much prose" signal for the agent.
    let body = text.strip_prefix("# Transcript\n\n").unwrap_or(&text);
    let paragraphs = body.split("\n\n").filter(|p| !p.trim().is_empty()).count();
    let char_count = text.chars().count();
    write_output_atomic(&out, text)?;
    Ok(VerbResult::ok(
        json!({"path": out, "format": format.extension(), "paragraphs": paragraphs, "char_count": char_count}),
    ))
}

/// export.chapters{path?} — write the timeline markers as a chapter list
/// ("M:SS Label" / "H:MM:SS Label" lines, time-sorted) for YouTube/podcast
/// chapters. Default <project>/exports/chapters.txt, caller path fenced
/// (the output-fencing contract). `first_at_zero` flags whether chapter 1 is at 0:00 (YouTube
/// requires it) so the agent can prepend an intro marker if not.
pub(super) async fn export_chapters(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        path: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (project, _edl, dir, _at) = snapshot(state).await?;
    let mut markers: Vec<&cut_core::Marker> = project.markers.iter().collect();
    markers.sort_by_key(|m| m.at_ms);
    if markers.is_empty() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "no markers to export as chapters",
            "the timeline has no markers",
        )
        .with_suggested_action("add markers with edit.add_marker first"));
    }
    let fmt_ts = |ms: u64| -> String {
        let (h, m, s) = (
            ms / 3_600_000,
            (ms % 3_600_000) / 60_000,
            (ms % 60_000) / 1000,
        );
        if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m}:{s:02}")
        }
    };
    let body: String = markers
        .iter()
        .map(|m| format!("{} {}\n", fmt_ts(m.at_ms), m.label))
        .collect();
    // Default goes through the same output-dir resolver as other exports. If
    // chapters.txt already exists, the resolver picks chapters-2.txt; explicit
    // caller paths stay exact and keep the fence's overwrite rules.
    let out = fence_output_path(&dir, a.path.as_deref(), "exports/chapters.txt")?;
    write_output_atomic(&out, body)?;
    Ok(VerbResult::ok(json!({
        "path": out,
        "chapter_count": markers.len(),
        "first_at_zero": markers[0].at_ms == 0,
    })))
}
