//! edl.rs — Edit Decision List derivation (timeline/op-log contract).
//!
//! Role: flatten the clip model into absolute timeline segments — the shape
//! cut-media turns into an ffmpeg filter_complex graph and cut-perception
//! checks against (cut_on_word compares EDL boundaries vs word spans).
//! Dependencies: types.rs. Primary callers: cut-media (render), cut-perception
//! (verify.checks), server (render.frame/preview).

use crate::types::{Clip, ClipCrop, ClipFade, MediaClip, Project, Track, TrackKind};
use serde::{Deserialize, Serialize};

/// serde skip-predicate mirroring types.rs: a zero crossfade is omitted from
/// EDL JSON so legacy EDLs round-trip byte-identical.
fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

/// One contiguous segment of one track on the timeline.
/// Media segments carry source ranges; gap segments have `asset: None`.
/// Caption segments carry their text in `caption_text`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdlSegment {
    pub track: String,
    pub track_kind: TrackKind,
    /// Clip id (None for gaps — they are anonymous).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<String>,
    /// Asset id for media segments; None for gaps and captions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// Absolute timeline position [in, out) in ms.
    pub timeline_in_ms: u64,
    pub timeline_out_ms: u64,
    /// Source range in the asset [in, out) in ms (media segments only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_out_ms: Option<u64>,
    /// Effective gain (clip gain + track gain) in dB, audio-bearing segments.
    #[serde(default)]
    pub gain_db: f64,
    /// Linear fade in/out (edit.fade), media segments only. Times are
    /// SEGMENT-LOCAL durations; the renderer applies them after PTS reset
    /// and clamps to the segment length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade: Option<ClipFade>,
    /// Source crop rectangle (edit.crop), media segments only. Source px,
    /// applied by the renderer BEFORE the conform scale/pad (crop → conform →
    /// transform). None = no crop (whole source frame).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<ClipCrop>,
    /// Crossfade-IN length, ms (edit.crossfade), media segments only. > 0
    /// means this segment dissolves from the PRECEDING media segment on the
    /// same track over this many ms (video xfade / audio acrossfade) instead of
    /// a hard cut. The EDL has ALREADY pulled this segment (and everything
    /// after it on the track) back by the overlap, so `timeline_in_ms` is the
    /// segment's REAL start; the renderer reads `xfade_in_ms` to emit the
    /// dissolve at the seam. 0 = hard cut. Resolved to 0 by the EDL when the
    /// preceding segment is not a media clip (nothing to dissolve from).
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub xfade_in_ms: u64,
    /// Crossfade transition STYLE carried from MediaClip.xfade_kind (edit.crossfade
    /// `transition`). None / "fade" = dissolve. Only meaningful when xfade_in_ms > 0
    /// (the renderer emits `xfade=transition=<kind>` at the seam); cleared to None
    /// alongside xfade_in_ms when the seam can't dissolve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xfade_kind: Option<String>,
    /// Playback speed of the source clip (MediaClip.speed), carried onto the
    /// segment so the renderer emits `setpts`/`atempo` and the source↔timeline
    /// mapping (source_to_timeline, scene/beat maps) divides/multiplies by it.
    /// 1.0 for gaps, captions, and normal-speed clips. serde-skip at 1.0 keeps
    /// pre-speed EDLs byte-identical.
    #[serde(
        default = "crate::types::default_speed",
        skip_serializing_if = "crate::types::is_unit_speed"
    )]
    pub speed: f64,
    /// Color grade carried from the source clip (MediaClip.grade) so the
    /// renderer emits the grade filter stage. None for gaps, captions, and
    /// ungraded clips. serde-skip-None keeps pre-grade EDLs byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<crate::types::ClipGrade>,
    /// LAYERED grade stack carried from the source clip (MediaClip.grade_stack;
    /// edit.grade_stack) so the renderer emits each layer's grade filter IN ORDER.
    /// Empty for gaps, captions, and clips using the single `grade` field above (the
    /// legacy path). serde-skip-empty keeps pre-stack EDLs byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grade_stack: Vec<crate::types::ClipGrade>,
    /// GEOMETRIC POWER WINDOWS carried from the source clip (MediaClip.grade_windows;
    /// edit.grade_window) — region-scoped grades the renderer composites INSIDE each
    /// window region. Empty for gaps, captions, and window-free clips; serde-skip-empty
    /// keeps pre-window EDLs byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grade_windows: Vec<crate::types::GradeWindow>,
    /// AI background matte carried from the source clip (MediaClip.matte) so the
    /// renderer composites the baked alpha (remove → reveal lower track; replace
    /// → fill bg). None for gaps, captions, and un-matted clips. serde-skip-None
    /// keeps pre-matte EDLs byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matte: Option<crate::types::ClipMatte>,
    /// Vector/freeform mask carried from MediaClip.mask (edit.add_mask) — a region
    /// effect (blur/pixelate/black) the renderer scopes via a baked alpha. None for
    /// gaps/captions/un-masked clips. serde-skip-None keeps pre-mask EDLs identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<crate::types::ClipMask>,
    /// Per-clip visual effects carried from MediaClip.effects (edit.effect), in
    /// order, so the renderer emits each effect's filter. Empty for gaps/captions/
    /// effect-free clips; serde-skip-empty keeps pre-effects EDLs byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<crate::types::ClipEffect>,
    /// Reverse playback carried from MediaClip.reverse (edit.reverse). When true
    /// the renderer emits `reverse`/`areverse`. false for gaps/captions/normal
    /// clips; serde-skip at false keeps pre-reverse EDLs byte-identical.
    #[serde(default, skip_serializing_if = "crate::types::is_false")]
    pub reverse: bool,
    /// Freeze-frame carried from MediaClip.freeze (edit.freeze). Some(at_ms) holds
    /// one source frame for the whole slot. None for gaps/captions/normal clips;
    /// serde-skip-None keeps pre-freeze EDLs byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze: Option<crate::types::ClipFreeze>,
    /// Ken Burns animation carried from MediaClip.animation (edit.animate). Some =
    /// the renderer emits a zoompan pan/zoom. None for gaps/captions/static clips;
    /// serde-skip-None keeps pre-animate EDLs byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<crate::types::ClipAnimation>,
    /// Parameter keyframes carried from MediaClip.keyframes (edit.keyframe). Each
    /// animates one param (opacity/volume) over time. Empty for gaps/captions/
    /// un-keyframed clips; serde-skip-empty keeps pre-keyframe EDLs byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<crate::types::Keyframe>,
    /// Parametric audio EQ carried from MediaClip.eq (edit.eq); high-pass +
    /// peaking bands + low-pass on the clip audio. None for gaps/captions/un-EQ'd
    /// clips; serde-skip-None keeps pre-EQ EDLs byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<crate::types::ClipEq>,
    /// Non-destructive mute ranges carried from MediaClip.mute_ranges
    /// (edit.mute_range / transcript.mute_words), SOURCE-asset ms. The renderer
    /// gates audio volume to 0 over each range's overlap with [src_in, src_out),
    /// mapped to post-speed segment time (mirrored under reverse). Empty for
    /// gaps/captions/unmuted clips; serde-skip-empty keeps pre-mute EDLs
    /// byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mute_ranges: Vec<[u64; 2]>,
    /// Video stabilization carried from MediaClip.stabilize (edit.stabilize). Some =
    /// the renderer emits `vidstabtransform` (once the detect pre-pass has cached the
    /// clip's `.trf`). None for gaps/captions/un-stabilized clips; serde-skip-None
    /// keeps pre-stabilize EDLs byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stabilize: Option<crate::types::ClipStabilize>,
    /// INPUT color space carried from the source clip (MediaClip.input_color_space;
    /// edit.color_space) so the renderer converts this segment's source INTO the
    /// project working space (and on to output) before grade/effects. None for gaps,
    /// captions, and untagged clips; serde-skip-None keeps pre-color-management EDLs
    /// byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_color_space: Option<crate::types::ColorSpace>,
    /// Caption text (caption segments only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_text: Option<String>,
    /// Caption style ref (caption segments only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_ref: Option<String>,
}

/// A non-destructive ADJUSTMENT LAYER flattened for the renderer (edit.adjustment).
/// A grade / look effect applied to the COMPOSITE of everything beneath it, gated
/// to `range_ms` (composition-local ms, ALREADY rebased to a window by
/// [`Edl::window`] so render.frame / segmented render.final gate correctly). The
/// renderer composites a TIME-GATED pass on the intermediate composite for each of
/// these. Empty for any timeline without an adjustment → that render is
/// byte-identical (the field is serde-skipped when empty and the render path is off).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdlAdjustment {
    /// Active span [start, end) in composition-local ms (window-rebased).
    pub range_ms: [u64; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<crate::types::ClipGrade>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<crate::types::ClipEffect>,
}

/// The whole flattened timeline. Segments are grouped per track, ordered by
/// timeline_in_ms; every cut boundary in the composition is a segment edge.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Edl {
    pub segments: Vec<EdlSegment>,
    /// Total composition duration (longest track), ms.
    pub duration_ms: u64,
    /// Non-destructive adjustment layers active over the composition (edit.adjustment),
    /// each clipped/rebased to this EDL's time base. Empty = none; serde-skipped when
    /// empty so pre-adjustment EDLs round-trip byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustments: Vec<EdlAdjustment>,
}

impl Edl {
    /// Segments of one track, in timeline order.
    pub fn track_segments<'a>(&'a self, track_id: &'a str) -> impl Iterator<Item = &'a EdlSegment> {
        self.segments.iter().filter(move |s| s.track == track_id)
    }

    /// All EDITORIAL cut boundaries (timeline_in/out of media segments) in
    /// ms, sorted + deduped — the boundary set cut_on_word reasons about.
    ///
    /// Audio-bearing tracks only (see `is_audio_bearing_track`): OVERLAY
    /// video tracks are compositing-only — a PiP insert never cuts the
    /// program audio, so its boundaries are not cut points. Including them
    /// previously produced false `cut_on_word` findings on PiP edits.
    pub fn cut_points_ms(&self) -> Vec<u64> {
        let mut pts: Vec<u64> = self
            .segments
            .iter()
            .filter(|s| s.asset.is_some() && self.is_audio_bearing_track(&s.track))
            .flat_map(|s| [s.timeline_in_ms, s.timeline_out_ms])
            .collect();
        pts.sort_unstable();
        pts.dedup();
        pts
    }

    /// Track id of the BASE video track: the first video track contributing
    /// segments — the same "first video track with clips is the base canvas"
    /// rule the renderer (cut-media build_graph) uses. Every later video
    /// track is an overlay composited above it.
    pub fn base_video_track(&self) -> Option<&str> {
        self.segments
            .iter()
            .find(|s| s.track_kind == TrackKind::Video)
            .map(|s| s.track.as_str())
    }

    /// Slice a sub-EDL covering the timeline window `[w0, w1)` (ms), rebased so
    /// it starts at 0. This is the foundation of SEGMENTED rendering (cut-media
    /// `render_segmented`): each window's sub-EDL feeds `build_graph`
    /// independently, so the filtergraph only ever holds ONE window's clips —
    /// peak memory is bounded by the WINDOW size, not the timeline length (the
    /// long-composite OOM fix; an overlay track is otherwise built as a
    /// continuous full-length alpha stream that grows with the timeline).
    ///
    /// Every segment that overlaps the window is clamped + rebased:
    /// - timeline range → `[max(in,w0), min(out,w1)) − w0`.
    /// - a media SOURCE range advances by the trimmed timeline span × `speed`
    ///   (the source plays `speed`× relative to the timeline), so the windowed
    ///   segment shows exactly the right source frames. `crop` is source-space
    ///   and therefore unchanged.
    /// - per-clip fades are SEGMENT-LOCAL durations: a front trim consumes that
    ///   much of the fade-IN, a back trim that much of the fade-OUT — so a clip
    ///   split across two windows fades IN only in the window holding its real
    ///   start and OUT only in the window holding its real end (no replayed
    ///   fade at the internal seam).
    /// - a crossfade-IN (`xfade_in_ms`) is kept ONLY when the head is intact
    ///   (no front trim); a front-trimmed segment is a mid-clip continuation and
    ///   is hard-joined. Window boundaries are chosen by the caller
    ///   (`plan_windows`) so a dissolve seam is never split.
    /// - captions/gaps clamp+rebase identically (text/style/kind preserved).
    ///
    /// `duration_ms` is the window length (`w1 − w0`) so `build_graph` pads /
    /// fills overlays to the window end, not to the whole composition. The caller
    /// passes `w1 ≤ self.duration_ms`.
    pub fn window(&self, w0: u64, w1: u64) -> Edl {
        let mut segments = Vec::new();
        for seg in &self.segments {
            let a = seg.timeline_in_ms.max(w0);
            let b = seg.timeline_out_ms.min(w1);
            if b <= a {
                continue; // no overlap with this window
            }
            let dt_in = a - seg.timeline_in_ms; // timeline ms trimmed off the front
            let dt_out = seg.timeline_out_ms - b; // timeline ms trimmed off the back
            let mut s = seg.clone();
            s.timeline_in_ms = a - w0;
            s.timeline_out_ms = b - w0;
            let new_dur = s.timeline_out_ms - s.timeline_in_ms;

            // Source range (media only): advance by the trimmed timeline span ×
            // speed. Guard rounding so the range can never invert.
            if let (Some(src_in), Some(src_out)) = (seg.src_in_ms, seg.src_out_ms) {
                let adv_in = (dt_in as f64 * seg.speed).round() as u64;
                let adv_out = (dt_out as f64 * seg.speed).round() as u64;
                let ni = src_in.saturating_add(adv_in).min(src_out);
                let no = src_out.saturating_sub(adv_out).max(ni);
                s.src_in_ms = Some(ni);
                s.src_out_ms = Some(no);
            }

            // Fade clamp: front trim eats the fade-in, back trim eats the
            // fade-out; clamp each to the new (shorter) segment duration.
            if let Some(fade) = &seg.fade {
                let in_ms = fade.in_ms.saturating_sub(dt_in).min(new_dur);
                let out_ms = fade.out_ms.saturating_sub(dt_out).min(new_dur);
                s.fade = if in_ms == 0 && out_ms == 0 {
                    None
                } else {
                    Some(crate::types::ClipFade {
                        in_ms,
                        out_ms,
                        kind: fade.kind, // FadeKind: Copy
                    })
                };
            }

            // A front-trimmed segment is a continuation, not a dissolve head.
            if dt_in > 0 {
                s.xfade_in_ms = 0;
                s.xfade_kind = None;
            }

            segments.push(s);
        }
        // Adjustment layers rebase exactly like segment times: clip to [w0,w1) and
        // shift to window-local 0, dropping any that fall fully outside. This is what
        // keeps a windowed render.frame / segmented render.final gate the grade to the
        // right span (the renderer reads window-local `t`).
        let adjustments = self
            .adjustments
            .iter()
            .filter_map(|adj| clip_adjustment(adj, w0, w1))
            .collect();
        Edl {
            segments,
            duration_ms: w1.saturating_sub(w0),
            adjustments,
        }
    }

    /// True when `track_id` carries program audio in the edit model: any
    /// audio track, or the base video track (its cuts are the editorial AV
    /// cuts — transcript verbs ripple video+audio together). Overlay video
    /// tracks (compositing-only, audio never rendered — cut-media mixes
    /// TrackKind::Audio tracks only) and caption tracks are NOT audio-bearing.
    pub fn is_audio_bearing_track(&self, track_id: &str) -> bool {
        let Some(kind) = self
            .segments
            .iter()
            .find(|s| s.track == track_id)
            .map(|s| s.track_kind)
        else {
            return false;
        };
        match kind {
            TrackKind::Audio => true,
            TrackKind::Video => self.base_video_track() == Some(track_id),
            TrackKind::Caption => false,
        }
    }
}

/// Clip a project/EDL adjustment to the window `[w0, w1)` and rebase it to
/// window-local 0. Returns None when the layer does not overlap the window (it is
/// dropped). Shared by `edl_from_project` (window = `[0, duration_ms)`) and
/// `Edl::window` (a sub-range), so derivation and segmentation clip identically.
fn clip_adjustment(adj: &EdlAdjustment, w0: u64, w1: u64) -> Option<EdlAdjustment> {
    let a = adj.range_ms[0].max(w0);
    let b = adj.range_ms[1].min(w1);
    if b <= a {
        return None; // no overlap with this window
    }
    Some(EdlAdjustment {
        range_ms: [a - w0, b - w0],
        grade: adj.grade.clone(),
        effects: adj.effects.clone(),
    })
}

/// Expand a VARIABLE-speed (edit.speed_ramp) media clip into its constant-speed
/// sub-segments as EDL segments, laid contiguously from timeline `start` (the
/// clip's `timeline_in_ms` AFTER any crossfade pullback). This is the whole
/// speed-ramp render strategy: the curve is sampled into `ramp.segments`
/// constant-speed slices ([`crate::types::speed_ramp_segments`]) and each is a
/// normal EDL segment the renderer already knows how to time-stretch
/// (setpts/atempo) — no render change, no new clip ids.
///
/// - The sub-segments tile the clip's `[0, total_dur)` timeline span exactly
///   (their durations sum to `Clip::timeline_duration_ms`, the SAME function), so
///   the caller's `cursor += dur` after the match lands every later clip
///   correctly — identical to a normal clip.
/// - Crossfade-IN (`xfade`/`xfade_kind`) rides the FIRST sub-segment (the clip's
///   head dissolves from its predecessor); later sub-segments are hard internal
///   joins (a contiguous source replay = a seamless ramp).
/// - The clip's fade-in/out are DISTRIBUTED across the sub-segments exactly as
///   [`Edl::window`] splits a fade across a clamp (front trim eats the fade-in,
///   back trim the fade-out), so a long fade spanning several sub-segments stays
///   smooth and is never replayed at an internal seam.
/// - Time-INVARIANT looks (gain/grade/effects/eq/crop) are carried onto every
///   sub-segment; the time-warping / per-frame-baked features
///   (reverse/freeze/animation/keyframes/matte/mask/stabilize) are refused at
///   verb time, so they are always absent here and emitted as None.
///
/// Returns the segments and the TAIL sub-segment's duration (what a FOLLOWING
/// clip can crossfade from). An empty result (degenerate sub-1ms expansion) is
/// possible; the caller then advances by `dur` (also 0) and pushes nothing.
fn ramp_segments(
    track: &Track,
    c: &MediaClip,
    start: u64,
    total_dur: u64,
    xfade: u64,
    xfade_kind: Option<String>,
) -> (Vec<EdlSegment>, u64) {
    let ramp = c
        .speed_ramp
        .as_ref()
        .expect("ramp_segments called on a non-ramped clip");
    let subs = crate::types::speed_ramp_segments(c.src_in_ms, c.src_out_ms, ramp);
    let mut out = Vec::new();
    let mut cursor = start;
    let mut tail = 0u64;
    for (i, sub) in subs.iter().enumerate() {
        let t_in = cursor;
        let t_out = cursor + sub.dur_ms;
        // Fade split (mirror Edl::window): how much of the clip's timeline lies
        // before / after this sub-segment, within the clip's own [0,total_dur).
        let fade = c.fade.as_ref().and_then(|f| {
            let dt_in = t_in - start;
            let dt_out = total_dur.saturating_sub(t_out - start);
            let in_ms = f.in_ms.saturating_sub(dt_in).min(sub.dur_ms);
            let out_ms = f.out_ms.saturating_sub(dt_out).min(sub.dur_ms);
            if in_ms == 0 && out_ms == 0 {
                None
            } else {
                Some(ClipFade {
                    in_ms,
                    out_ms,
                    kind: f.kind,
                })
            }
        });
        out.push(EdlSegment {
            track: track.id.clone(),
            track_kind: track.kind,
            clip_id: Some(c.id.clone()),
            asset: Some(c.asset.clone()),
            timeline_in_ms: t_in,
            timeline_out_ms: t_out,
            src_in_ms: Some(sub.src_in),
            src_out_ms: Some(sub.src_out),
            gain_db: c.gain_db + track.gain_db,
            fade,
            crop: c.crop.clone(),
            // Crossfade-in only at the clip head; internal seams are hard joins.
            xfade_in_ms: if i == 0 { xfade } else { 0 },
            xfade_kind: if i == 0 && xfade > 0 {
                xfade_kind.clone()
            } else {
                None
            },
            // The defining field: each sub-segment plays at its CONSTANT sampled
            // speed, so the proven per-segment setpts/atempo path renders the ramp.
            speed: sub.speed,
            grade: c.grade.clone(),
            grade_stack: c.grade_stack.clone(),
            grade_windows: c.grade_windows.clone(),
            // Refused at verb time on a ramped clip → always None here.
            matte: None,
            mask: None,
            effects: c.effects.clone(),
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: c.eq.clone(),
            mute_ranges: c.mute_ranges.clone(),
            stabilize: None,
            input_color_space: c.input_color_space,
            caption_text: None,
            style_ref: None,
        });
        cursor = t_out;
        tail = sub.dur_ms;
    }
    (out, tail)
}

/// Derive the EDL from a project: walk each track accumulating timeline
/// position (clips are ordered + non-overlapping per timeline/op-log contract; gaps advance
/// the cursor and ALSO emit a gap segment so renderers can fill black/silence).
pub fn edl_from_project(project: &Project) -> Edl {
    let mut segments = Vec::new();
    // Total crossfade overlap consumed across ALL tracks — the realized
    // composition is shorter than the nominal cumulative duration by the
    // longest single track's overlap (durations are per-track maxima). We
    // track the largest realized track end below instead of the nominal max.
    let mut max_track_end: u64 = 0;
    for track in &project.tracks {
        if track.kind == TrackKind::Caption && !track.visible {
            continue;
        }
        let mut cursor: u64 = 0;
        let caption_track_duration =
            (track.kind == TrackKind::Caption).then(|| track.duration_ms());
        // Was the IMMEDIATELY preceding clip on this track a media clip? Only a
        // media→media adjacency can crossfade (you cannot dissolve from a gap
        // or the track head). Reset by gaps and at the track start.
        let mut prev_media_dur: Option<u64> = None;
        for clip in &track.clips {
            let dur = clip.timeline_duration_ms();
            match clip {
                Clip::Media(c) => {
                    // Resolve the crossfade overlap: 0 when there is no media
                    // predecessor to dissolve FROM, else clamp to both this
                    // clip's and the predecessor's duration (a dissolve can
                    // never be longer than the material on either side).
                    let xfade = match prev_media_dur {
                        Some(prev) if c.xfade_in_ms > 0 => c.xfade_in_ms.min(prev).min(dur),
                        _ => 0,
                    };
                    // Pull this clip (and, via the shared cursor, everything
                    // after it) back by the overlap — the realized timeline
                    // shortens by `xfade`, matching ffmpeg xfade's len_a+len_b-D.
                    cursor = cursor.saturating_sub(xfade);
                    if c.speed_ramp.is_some() {
                        // VARIABLE speed (edit.speed_ramp): expand into constant-
                        // speed sub-segments that tile [cursor, cursor+dur) exactly
                        // (their durations sum to `dur` = timeline_duration_ms, so the
                        // post-match `cursor += dur` still lands later clips right).
                        let (subs, tail) =
                            ramp_segments(track, c, cursor, dur, xfade, c.xfade_kind.clone());
                        // A following clip can crossfade only from the TAIL sub-segment
                        // (the contiguous material at the clip's end); None if degenerate.
                        prev_media_dur = if subs.is_empty() { None } else { Some(tail) };
                        segments.extend(subs);
                    } else {
                        segments.push(EdlSegment {
                            track: track.id.clone(),
                            track_kind: track.kind,
                            clip_id: Some(c.id.clone()),
                            asset: Some(c.asset.clone()),
                            timeline_in_ms: cursor,
                            timeline_out_ms: cursor + dur,
                            src_in_ms: Some(c.src_in_ms),
                            src_out_ms: Some(c.src_out_ms),
                            gain_db: c.gain_db + track.gain_db,
                            fade: c.fade.clone(),
                            crop: c.crop.clone(),
                            xfade_in_ms: xfade,
                            // Carry the transition style only when the dissolve is live
                            // (xfade resolved > 0); a cleared seam has no style.
                            xfade_kind: if xfade > 0 {
                                c.xfade_kind.clone()
                            } else {
                                None
                            },
                            speed: c.speed,
                            grade: c.grade.clone(),
                            grade_stack: c.grade_stack.clone(),
                            grade_windows: c.grade_windows.clone(),
                            matte: c.matte.clone(),
                            mask: c.mask.clone(),
                            effects: c.effects.clone(),
                            reverse: c.reverse,
                            freeze: c.freeze.clone(),
                            animation: c.animation.clone(),
                            keyframes: c.keyframes.clone(),
                            eq: c.eq.clone(),
                            mute_ranges: c.mute_ranges.clone(),
                            stabilize: c.stabilize.clone(),
                            input_color_space: c.input_color_space,
                            caption_text: None,
                            style_ref: None,
                        });
                        prev_media_dur = Some(dur);
                    }
                }
                Clip::Gap(_) => {
                    segments.push(EdlSegment {
                        track: track.id.clone(),
                        track_kind: track.kind,
                        clip_id: None,
                        asset: None,
                        timeline_in_ms: cursor,
                        timeline_out_ms: cursor + dur,
                        src_in_ms: None,
                        src_out_ms: None,
                        gain_db: 0.0,
                        fade: None,
                        crop: None,
                        xfade_in_ms: 0,
                        xfade_kind: None,
                        speed: 1.0,
                        grade: None,
                        grade_stack: vec![],
                        grade_windows: vec![],
                        matte: None,
                        mask: None,
                        effects: vec![],
                        reverse: false,
                        freeze: None,
                        animation: None,
                        keyframes: vec![],
                        eq: None,
                        mute_ranges: vec![],
                        stabilize: None,
                        input_color_space: None,
                        caption_text: None,
                        style_ref: None,
                    });
                    prev_media_dur = None; // a gap breaks media→media adjacency
                }
                Clip::Caption(c) => segments.push(EdlSegment {
                    track: track.id.clone(),
                    track_kind: track.kind,
                    clip_id: Some(c.id.clone()),
                    asset: None,
                    // Captions carry absolute ranges already (timeline time).
                    timeline_in_ms: c.range_ms[0],
                    timeline_out_ms: c.range_ms[1],
                    src_in_ms: None,
                    src_out_ms: None,
                    gain_db: 0.0,
                    fade: None,
                    crop: None,
                    xfade_in_ms: 0,
                    xfade_kind: None,
                    speed: 1.0,
                    grade: None,
                    grade_stack: vec![],
                    grade_windows: vec![],
                    matte: None,
                    mask: None,
                    effects: vec![],
                    reverse: false,
                    freeze: None,
                    animation: None,
                    keyframes: vec![],
                    eq: None,
                    mute_ranges: vec![],
                    stabilize: None,
                    input_color_space: None,
                    caption_text: Some(c.text.clone()),
                    style_ref: c.style_ref.clone(),
                }),
            }
            // Caption clips on caption tracks still advance the cursor by their
            // own duration ONLY when modeled sequentially; public contract keeps caption
            // ranges absolute, so the cursor is advanced for non-caption tracks.
            if track.kind != TrackKind::Caption {
                cursor += dur;
                max_track_end = max_track_end.max(cursor);
            } else if let Some(duration) = caption_track_duration {
                // Caption track end is the max absolute range end (Track::
                // duration_ms convention) — keep the longest-track duration honest.
                max_track_end = max_track_end.max(duration);
            }
        }
    }
    // Realized composition duration = the longest track's REALIZED end (after
    // crossfade pullbacks), NOT the nominal cumulative sum. Falls back to the
    // project's nominal duration when no track produced segments (empty).
    let duration_ms = if max_track_end > 0 {
        max_track_end
    } else {
        project.duration_ms()
    };
    // Flatten the project's adjustment layers, clipping each to the realized
    // composition [0, duration_ms). A layer whose span falls entirely past the end
    // is dropped (it would grade nothing). Empty when the project has none → the
    // EDL (and the render) is byte-identical to a pre-adjustment timeline.
    let adjustments = project
        .adjustments
        .iter()
        .filter_map(|a| {
            clip_adjustment(
                &EdlAdjustment {
                    range_ms: a.range_ms,
                    grade: a.grade.clone(),
                    effects: a.effects.clone(),
                },
                0,
                duration_ms,
            )
        })
        .collect();
    Edl {
        segments,
        duration_ms,
        adjustments,
    }
}

#[cfg(test)]
mod window_tests {
    //! Tests for `Edl::window` — the slicer under segmented rendering. The
    //! invariants here are what keep a segmented render frame-identical to the
    //! whole-graph render at window seams.
    use super::*;
    use crate::types::{ClipFade, FadeKind};

    /// A base-track media segment [tl_in,tl_out) ← src[src_in,src_out), speed.
    fn media(tl_in: u64, tl_out: u64, src_in: u64, src_out: u64, speed: f64) -> EdlSegment {
        EdlSegment {
            track: "v1".into(),
            track_kind: TrackKind::Video,
            clip_id: Some("c1".into()),
            asset: Some("a1".into()),
            timeline_in_ms: tl_in,
            timeline_out_ms: tl_out,
            src_in_ms: Some(src_in),
            src_out_ms: Some(src_out),
            gain_db: 0.0,
            fade: None,
            crop: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed,
            grade: None,
            grade_stack: vec![],
            grade_windows: vec![],
            matte: None,
            mask: None,
            effects: vec![],
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            input_color_space: None,
            caption_text: None,
            style_ref: None,
        }
    }

    fn one(edl: &Edl) -> &EdlSegment {
        assert_eq!(edl.segments.len(), 1, "expected exactly one segment");
        &edl.segments[0]
    }

    #[test]
    fn duration_is_window_length_and_nonoverlap_dropped() {
        let edl = Edl {
            segments: vec![media(0, 4000, 0, 4000, 1.0)],
            duration_ms: 10000,
            adjustments: vec![],
        };
        // Window past the segment → it is dropped, duration is still the window.
        let w = edl.window(5000, 8000);
        assert_eq!(w.duration_ms, 3000);
        assert!(w.segments.is_empty());
    }

    #[test]
    fn split_reconstructs_source_and_timeline_at_unit_speed() {
        // One 0..10s clip, source 0..10s. Split at 6s.
        let edl = Edl {
            segments: vec![media(0, 10000, 0, 10000, 1.0)],
            duration_ms: 10000,
            adjustments: vec![],
        };
        let left = edl.window(0, 6000);
        let right = edl.window(6000, 10000);
        let l = one(&left);
        let r = one(&right);
        // Timeline rebased to each window's local 0.
        assert_eq!((l.timeline_in_ms, l.timeline_out_ms), (0, 6000));
        assert_eq!((r.timeline_in_ms, r.timeline_out_ms), (0, 4000));
        // Source ranges partition the original exactly, no gap/overlap.
        assert_eq!((l.src_in_ms, l.src_out_ms), (Some(0), Some(6000)));
        assert_eq!((r.src_in_ms, r.src_out_ms), (Some(6000), Some(10000)));
    }

    #[test]
    fn source_advance_scales_with_speed() {
        // 2× speed: 10s of source plays in 5s of timeline. Window the back half
        // of the timeline [2500,5000) → source advances by 2500*2 = 5000ms.
        let edl = Edl {
            segments: vec![media(0, 5000, 0, 10000, 2.0)],
            duration_ms: 5000,
            adjustments: vec![],
        };
        let w = edl.window(2500, 5000);
        let s = one(&w);
        assert_eq!((s.timeline_in_ms, s.timeline_out_ms), (0, 2500));
        assert_eq!((s.src_in_ms, s.src_out_ms), (Some(5000), Some(10000)));
        assert_eq!(s.speed, 2.0);
    }

    #[test]
    fn window_saturates_extreme_source_advance() {
        let mut seg = media(0, 10, u64::MAX - 2, u64::MAX, f64::MAX);
        seg.speed = f64::MAX;
        let edl = Edl {
            segments: vec![seg],
            duration_ms: 10,
            adjustments: vec![],
        };
        let w = edl.window(1, 10);
        let s = one(&w);
        assert_eq!(s.src_in_ms, Some(u64::MAX));
        assert_eq!(s.src_out_ms, Some(u64::MAX));
    }

    #[test]
    fn fade_in_consumed_by_front_trim_out_by_back_trim() {
        let mut seg = media(0, 10000, 0, 10000, 1.0);
        seg.fade = Some(ClipFade {
            in_ms: 1000,
            out_ms: 1000,
            kind: FadeKind::Both,
        });
        let edl = Edl {
            segments: vec![seg],
            duration_ms: 10000,
            adjustments: vec![],
        };
        // Left window holds the real start (fade-in survives) but not the end
        // (no fade-out — the clip continues into the next window).
        let l = one(&edl.window(0, 6000)).clone();
        let lf = l.fade.unwrap();
        assert_eq!((lf.in_ms, lf.out_ms), (1000, 0));
        // Right window holds the real end (fade-out survives) but not the start.
        let r = one(&edl.window(6000, 10000)).clone();
        let rf = r.fade.unwrap();
        assert_eq!((rf.in_ms, rf.out_ms), (0, 1000));
        // A window deep inside the clip carries NEITHER fade → None.
        let mid = edl.window(3000, 5000);
        assert!(one(&mid).fade.is_none());
    }

    #[test]
    fn crossfade_kept_when_head_intact_dropped_when_front_trimmed() {
        let mut seg = media(2000, 8000, 0, 6000, 1.0);
        seg.xfade_in_ms = 500; // dissolves from its predecessor over [2000,2500)
        let edl = Edl {
            segments: vec![seg],
            duration_ms: 8000,
            adjustments: vec![],
        };
        // Window starting at the seg head (no front trim) keeps the dissolve.
        let head = edl.window(2000, 6000);
        assert_eq!(one(&head).xfade_in_ms, 500);
        // Window starting mid-segment (front-trimmed) is a continuation: no xfade.
        let tail = edl.window(4000, 8000);
        assert_eq!(one(&tail).xfade_in_ms, 0);
    }

    #[test]
    fn overlay_straddling_boundary_clamps_both_sides() {
        // Overlay [3000,8000) ← src[0,5000). Boundary at 5000 splits it.
        let mut ov = media(3000, 8000, 0, 5000, 1.0);
        ov.track = "v2".into();
        let edl = Edl {
            segments: vec![ov],
            duration_ms: 10000,
            adjustments: vec![],
        };
        let left = one(&edl.window(0, 5000)).clone(); // shows [3000,5000) → src[0,2000)
        assert_eq!((left.timeline_in_ms, left.timeline_out_ms), (3000, 5000));
        assert_eq!((left.src_in_ms, left.src_out_ms), (Some(0), Some(2000)));
        let right = one(&edl.window(5000, 10000)).clone(); // [5000,8000)→rebased[0,3000), src[2000,5000)
        assert_eq!((right.timeline_in_ms, right.timeline_out_ms), (0, 3000));
        assert_eq!(
            (right.src_in_ms, right.src_out_ms),
            (Some(2000), Some(5000))
        );
    }

    #[test]
    fn caption_clamps_and_rebases_preserving_text() {
        let cap = EdlSegment {
            track: "cap1".into(),
            track_kind: TrackKind::Caption,
            clip_id: Some("cap_c1".into()),
            asset: None,
            timeline_in_ms: 2000,
            timeline_out_ms: 7000,
            src_in_ms: None,
            src_out_ms: None,
            gain_db: 0.0,
            fade: None,
            crop: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            grade_stack: vec![],
            grade_windows: vec![],
            matte: None,
            mask: None,
            effects: vec![],
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            input_color_space: None,
            caption_text: Some("hello".into()),
            style_ref: Some("brand1".into()),
        };
        let edl = Edl {
            segments: vec![cap],
            duration_ms: 10000,
            adjustments: vec![],
        };
        let w = edl.window(5000, 10000); // caption visible [5000,7000) → rebased [0,2000)
        let s = one(&w);
        assert_eq!((s.timeline_in_ms, s.timeline_out_ms), (0, 2000));
        assert_eq!(s.caption_text.as_deref(), Some("hello"));
        assert_eq!(s.style_ref.as_deref(), Some("brand1"));
        assert!(s.src_in_ms.is_none());
    }

    #[test]
    fn hidden_caption_tracks_do_not_enter_edl() {
        let mut p = crate::types::Project::new("t", Default::default());
        p.tracks.push(crate::types::Track {
            id: "cap1".into(),
            kind: TrackKind::Caption,
            clips: vec![Clip::Caption(crate::types::CaptionClip {
                id: "cap_c1".into(),
                text: "hidden caption".into(),
                style_ref: None,
                range_ms: [1000, 3000],
            })],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: false,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });

        let edl = edl_from_project(&p);
        assert!(
            edl.segments
                .iter()
                .all(|s| s.caption_text.as_deref() != Some("hidden caption")),
            "hidden caption tracks must not burn into preview/export"
        );
    }

    /// `clip_adjustment` (shared by edl_from_project + Edl::window): clips a layer
    /// span to a window and shifts it to window-local 0; a non-overlapping layer is
    /// dropped. This is the logic that keeps a windowed/segmented render gate the
    /// grade to the right span.
    #[test]
    fn clip_adjustment_clips_shifts_and_drops() {
        let adj = EdlAdjustment {
            range_ms: [2000, 12000],
            grade: None,
            effects: vec![crate::types::ClipEffect::Vignette { amount: 0.5 }],
        };
        // Clip to the realized composition [0, 10000): [2000,12000) → [2000,10000).
        let c = clip_adjustment(&adj, 0, 10000).unwrap();
        assert_eq!(c.range_ms, [2000, 10000]);
        // Window [5000,10000): overlap [5000,10000) → window-local [0,5000).
        let w = clip_adjustment(&adj, 5000, 10000).unwrap();
        assert_eq!(w.range_ms, [0, 5000]);
        // Window starting AFTER the layer ends → no overlap → dropped.
        let adj2 = EdlAdjustment {
            range_ms: [2000, 4000],
            grade: None,
            effects: vec![crate::types::ClipEffect::Vignette { amount: 0.5 }],
        };
        assert!(clip_adjustment(&adj2, 5000, 10000).is_none());
    }

    /// A VARIABLE-speed (edit.speed_ramp) clip is EXPANDED by `edl_from_project`
    /// into contiguous constant-speed sub-segments that tile the clip's timeline
    /// span exactly, and a FOLLOWING clip starts right where the ramp ends — the
    /// whole speed-ramp render strategy in one assertion.
    #[test]
    fn ramped_clip_expands_into_contiguous_constant_speed_segments() {
        use crate::types::{SpeedRamp, SpeedRampPoint};
        let mut p = crate::types::Project::new("t", Default::default());
        let mut ramped = crate::edit::make_media_clip("c1", "a1", 0, 4000);
        ramped.speed_ramp = Some(SpeedRamp {
            points: vec![
                SpeedRampPoint {
                    at_ms: 0,
                    factor: 1.0,
                },
                SpeedRampPoint {
                    at_ms: 2000,
                    factor: 4.0,
                },
                SpeedRampPoint {
                    at_ms: 4000,
                    factor: 1.0,
                },
            ],
            segments: 20,
        });
        let next = crate::edit::make_media_clip("c2", "a1", 0, 1000); // a normal follower
        p.track_mut("v1").unwrap().clips = vec![Clip::Media(ramped.clone()), Clip::Media(next)];

        let edl = edl_from_project(&p);
        let segs: Vec<&EdlSegment> = edl.track_segments("v1").collect();
        // The ramp expanded into multiple sub-segments + the follower (> 2 total).
        let ramp_segs: Vec<&&EdlSegment> = segs
            .iter()
            .filter(|s| s.clip_id.as_deref() == Some("c1"))
            .collect();
        assert!(
            ramp_segs.len() >= 2,
            "ramp must expand into ≥2 sub-segments, got {}",
            ramp_segs.len()
        );
        // Sub-segments are contiguous in BOTH timeline and source, and tile the
        // whole source window [0,4000).
        assert_eq!(ramp_segs.first().unwrap().src_in_ms, Some(0));
        assert_eq!(ramp_segs.last().unwrap().src_out_ms, Some(4000));
        for w in ramp_segs.windows(2) {
            assert_eq!(w[0].timeline_out_ms, w[1].timeline_in_ms, "timeline gap");
            assert_eq!(w[0].src_out_ms, w[1].src_in_ms, "source gap");
        }
        // Their durations sum to the clip's timeline_duration_ms (cursor invariant),
        // and the FAST middle makes the clip SHORTER than its 4000 ms source.
        let total: u64 = ramp_segs
            .iter()
            .map(|s| s.timeline_out_ms - s.timeline_in_ms)
            .sum();
        let expected = Clip::Media(ramped).timeline_duration_ms();
        assert_eq!(
            total, expected,
            "sub-segment durations must sum to the clip duration"
        );
        assert!(
            total < 4000,
            "the fast middle nets a shorter clip ({total} ms)"
        );
        // The middle sub-segments are faster than the edge ones (the ramp shape).
        let speeds: Vec<f64> = ramp_segs.iter().map(|s| s.speed).collect();
        let mid = speeds[speeds.len() / 2];
        assert!(
            mid > speeds[0] && mid > *speeds.last().unwrap(),
            "middle should be fastest"
        );
        // The follower starts exactly where the ramp ends (no gap, no overlap).
        let follower = segs
            .iter()
            .find(|s| s.clip_id.as_deref() == Some("c2"))
            .unwrap();
        assert_eq!(
            follower.timeline_in_ms, total,
            "follower must butt the ramp end"
        );
    }

    /// Edl::window carries + rebases adjustment layers exactly like segments: the
    /// layer is clipped to the window and shifted to local 0, and a layer outside
    /// the window is dropped — so a render.frame inside a deep window still gates
    /// the grade correctly.
    #[test]
    fn window_rebases_adjustments() {
        let edl = Edl {
            segments: vec![media(0, 10000, 0, 10000, 1.0)],
            duration_ms: 10000,
            adjustments: vec![EdlAdjustment {
                range_ms: [2000, 8000],
                grade: None,
                effects: vec![crate::types::ClipEffect::Vignette { amount: 0.5 }],
            }],
        };
        // [5000,10000): adjustment [2000,8000) ∩ window = [5000,8000) → local [0,3000).
        let w = edl.window(5000, 10000);
        assert_eq!(w.adjustments.len(), 1);
        assert_eq!(w.adjustments[0].range_ms, [0, 3000]);
        // A window past the layer → the layer is dropped.
        let w2 = edl.window(8500, 10000);
        assert!(w2.adjustments.is_empty());
    }
}
