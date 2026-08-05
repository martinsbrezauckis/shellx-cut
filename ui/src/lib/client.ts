// client.ts — typed verb client (public verb contract).
// Role: the ONLY way the UI talks to the engine — POST /api/verb/{name} with
// JSON args, universal envelope back. Hand-written 1:1 from schema/verbs.json
// (the single source of truth); if you change verbs.json, change THIS file in
// the same commit. Zero local mutation: panels call verbs, state comes back
// via project.state / WS events.
// Callers: every panel + App.tsx. Dependencies: fetch only.

import { API_BASE } from './clientUrls'

export { API_BASE, sourceUrl, exportUrl, frameUrl } from './clientUrls'
export { getWaveform, getWindowThumbs } from './clientMedia'
export type { WindowThumbs } from './clientMedia'

import type {
  BrandKit,
  CaptionStyle,
  ClipEffect,
  CutError,
  KfInterp,
  KfParam,
  MarkerColor,
  Project,
  ProjectSettings,
} from './clientModel'
import { UI_OPEN_SURFACE_IDS } from '../app/uiSurfaceRegistry'
import type { VerbResults } from './clientResults'

export type {
  AnimState,
  Asset,
  CaptionStyle,
  CheckResult,
  Checkpoint,
  Clip,
  ClipAnimation,
  ClipCrop,
  ClipEffect,
  ClipEq,
  ClipFade,
  ClipFreeze,
  ClipGrade,
  ClipGradeWindow,
  ClipTransform,
  ColorSpace,
  BrandKit,
  Comment,
  CommentDraft,
  CutError,
  EqBand,
  FixAction,
  GainWindow,
  JobRecord,
  JudgeEnvelope,
  JudgeIssue,
  JudgeReview,
  Keyframe,
  KfInterp,
  KfParam,
  Marker,
  MotionClipLink,
  OpRecord,
  Project,
  ProjectSettings,
  Sequence,
  SequenceSummary,
  RenderReceipt,
  TimelineTranscript,
  TimelineWord,
  Track,
  TrackKind,
  Transcript,
  Waveform,
  WordSpan,
} from './clientModel'
export { isIdentityTransform, mediaClipTimelineDurationMs } from './clientModel'
export type * from './clientResults'

// ---------------------------------------------------------------------------
// Envelope (public verb contract: every verb returns exactly this)
// ---------------------------------------------------------------------------

export interface VerbResult<T = unknown> {
  ok: boolean
  result?: T
  op_ids?: string[]
  /** Non-fatal findings (public verb contract) — e.g. media.relink's relink_shorter_than_used. */
  warnings?: Array<{ code: string; message: string }>
  error?: CutError
}

// ---------------------------------------------------------------------------
// Back-compatible export for typed verb args. The shared surface registry is
// the source of truth for this type, human launchers, ui.state, and tests.
export const UI_OPEN_PANELS = UI_OPEN_SURFACE_IDS
export type UiOpenPanel = (typeof UI_OPEN_PANELS)[number]

// Verb args map — one entry per verb in schema/verbs.json (260 verbs, 32 domains).
// Keys ARE the wire names; keep in sync with the registry. Later additions
// edit.crop, edit.crossfade, edit.move_marker, audio.add_music,
// captions.set_range + ripple flags on edit.ripple_delete (lift) / edit.move.
// ---------------------------------------------------------------------------

export interface VerbArgs {
  'project.create': { name: string; settings?: Partial<ProjectSettings>; dir?: string; starter?: 'first-edit' }
  'project.open': { path: string }
  'project.save': Record<string, never>
  'project.state': Record<string, never>
  'project.sequence_list': Record<string, never>
  'project.sequence_index': { query?: string; kind?: 'all' | 'clip' | 'marker'; sequence?: string; track_kind?: 'video' | 'audio' | 'caption'; status?: 'all' | 'issues' | 'offline' | 'gaps' | 'effects' | 'hidden' | 'locked' | 'muted'; limit?: number }
  'project.sequence_create': { name: string; from?: 'empty' | 'active'; rationale?: string }
  'project.sequence_switch': { id: string; rationale?: string }
  'project.sequence_rename': { id: string; name: string; rationale?: string }
  'project.sequence_delete': { id: string; rationale?: string }
  'project.ops': { since?: string }
  'project.checkpoint': { name: string; rationale?: string }
  'project.rename': { name: string; rationale?: string }
  'project.format': { width?: number; height?: number; fps?: number; rationale?: string }
  'project.color': { working?: 'rec709' | 'rec2020' | 'srgb' | 'linear'; output?: 'rec709' | 'rec2020' | 'srgb' | 'linear'; rationale?: string }
  'project.brand': { brand?: BrandKit; clear?: boolean; rationale?: string }
  'project.set_output_dir': { dir?: string; rationale?: string }
  'project.revert': { to: string; if_tip?: string; rationale?: string }
  'project.undo': Record<string, never>
  'project.redo': Record<string, never>
  'project.diff': { from: string; to: string }
  'project.close': Record<string, never>
  // Projects index (discovery): list recent projects + forget one (forget != delete).
  'project.list': { sort?: 'recent' | 'name' | 'created'; q?: string }
  'project.forget': { id?: string; path?: string; missing?: boolean }
  'project.delete': { id?: string; path?: string }

  // Global asset library (cross-project media: video/audio/image).
  'library.list': {
    type?: 'video' | 'audio' | 'image'
    folder?: string
    tag?: string
    q?: string
    sort?: 'added' | 'name' | 'recent' | 'uses'
    collection?: 'all' | 'favorites' | 'missing'
    ids?: string[]
    offset?: number
    limit?: number
  }
  'library.add': { path?: string; asset?: string; name?: string; tags?: string[]; folder?: string; copy?: boolean; source?: 'user' | 'agent' }
  'library.relink': { id: string; path: string }
  'library.remove': { id: string }
  'library.move': { id: string; folder?: string }
  'library.tag': { id: string; tags: string[] }
  'library.favorite': { id: string; on: boolean }
  'library.use': { id: string }
  'library.add_to_project': { id: string }
  'library.folder_add': { name: string }
  'library.folder_rename': { old: string; new: string }
  'library.folder_remove': { name: string }

  'media.import': { path: string; capture_manifest?: string; proxy?: boolean; rationale?: string; include_inverse?: boolean }
  // Drop an asset from the open project (+ unlink its regenerable derived files);
  // the SOURCE file is never touched. Refuses (conflict) while clips still use it.
  'media.remove': { asset: string; rationale?: string }
  // Repoint an asset at a new source path (offline-media recovery). Same content
  // hash = pure repath (derived kept); new hash = derived cleared + import chain
  // rerun (result.job_id). Refuses kind mismatch; warns if shorter than used.
  'media.relink': { asset: string; path: string; rationale?: string }
  // Read-only offline report: {count, offline_count, assets:[{asset, path,
  // exists, modified_ms?, referenced}]} — existence computed from the fs at call time.
  'media.check': { asset?: string }
  // Smart bins: named saved searches over the asset tray; membership
  // computed at list time. AND-combined criteria; at least one required.
  'media.bin_save': { name: string; kind?: 'video' | 'audio' | 'image'; text?: string; unused?: boolean; min_width?: number; min_height?: number; offline?: boolean; modified_after_ms?: number; modified_before_ms?: number; rationale?: string }
  'media.bin_delete': { name: string; rationale?: string }
  'media.bin_list': Record<string, never>
  'media.probe': { asset: string }
  'media.transcribe': { asset: string; model?: string }
  'media.perception': { asset: string }
  'media.diarize': { asset: string; max_speakers?: number }
  'media.index_status': { asset?: string }
  'media.index': { asset: string; fps?: number; rationale?: string }
  'media.search': { query?: string; query_vector?: number[]; asset?: string; top_k?: number; max_gap_ms?: number; rationale?: string }
  'effects.list': Record<string, never>
  // transitions-as-data discovery catalog (xfade styles for edit.crossfade).
  'transitions.list': Record<string, never>
  // Per-bucket audio peaks (0..1) for the timeline waveform overlay; buckets
  // optional (engine default 1000). Synchronous + deterministic; errors when
  // the asset has no audio stream (callers treat that as "no waveform").
  'media.waveform': { asset: string; buckets?: number }
  // Whole-asset base strip (no range_ms) OR windowed/zoom mode (range_ms given:
  // sample just that SOURCE sub-range at `count` frames — the per-zoom density
  // path; returns {thumbs,range_ms,count,h} instead of {filmstrip}).
  'media.filmstrip': { asset: string; range_ms?: [number, number]; count?: number; h?: number }

  // the background-job contract: jobs domain replaces media.status; every job-creating verb
  // returns {job_id} polled here (WS job_progress events are the fast path).
  'jobs.status': { job_id: string }
  'jobs.list': Record<string, never>
  'jobs.cancel': { job_id: string }

  'edit.split': { track: string; at_ms: number; rationale?: string }
  // Cut/align clips to MUSIC BEATS. Beat source =
  // the audio.add_music beat:N markers. mode "split" (default) cuts the track at
  // each selected beat; mode "snap" rolls existing cut boundaries onto beats.
  // every_n thins beats (2 = every 2nd, 4 = every bar); range_ms limits the span.
  'edit.cut_to_beat': { track?: string; mode?: 'split' | 'snap'; every_n?: number; range_ms?: [number, number]; max_snap_ms?: number; rationale?: string }
  // ripple: true (default) = close the gap (extract — later content shifts
  // left, captions/markers/duck-windows remap); false = LIFT (leave a gap of
  // equal length, nothing downstream moves). Omitting it replays as close-gap.
  'edit.ripple_delete': { track?: string; range_ms: [number, number]; ripple?: boolean; rationale?: string; group_id?: string }
  'edit.trim': { clip: string; src_in_ms?: number; src_out_ms?: number; linked?: boolean; rationale?: string }
  // Per-clip retime (slow-mo / speed-up): factor 0.25–4, 1 clears. Pitch is
  // preserved (preserve_pitch:false / varispeed is a reserved v2, rejected).
  'edit.speed': { clip: string; factor: number; preserve_pitch?: boolean; rationale?: string }
  // Variable speed / time remap (the "speed curve"): a piecewise-linear speed
  // ramp over the clip, vs constant edit.speed. points:[] clears it. segments =
  // sampling granularity (default 24). Factors 0.25–4.0.
  'edit.speed_ramp': {
    clip: string
    points: Array<{ at_ms: number; factor: number }>
    preserve_pitch?: boolean
    segments?: number
    rationale?: string
  }
  // Per-clip color grade (ffmpeg eq + white balance + optional
  // .cube LUT). Omitted params keep identity; an all-identity grade clears it.
  'edit.grade': { clip: string; contrast?: number; brightness?: number; saturation?: number; gamma?: number; temperature_k?: number; lut?: string; rationale?: string }
  // LAYERED grading — a node-stack of grade layers applied in serial
  // nodes). Each layer is the same shape as edit.grade; empty/single-layer stays
  // byte-identical to the single edit.grade.
  'edit.grade_stack': { clip: string; grades: Array<{ contrast?: number; brightness?: number; saturation?: number; gamma?: number; temperature_k?: number; lut?: string }>; rationale?: string }
  // GEOMETRIC POWER WINDOW — a region-scoped grade (a geometric grade window). shape/
  // points/feather/invert define the REGION (same geometry as edit.add_mask); the grade
  // knobs (same as edit.grade) apply ONLY inside it. Windows STACK (each call appends);
  // enabled:false clears all. v1: BASE-track video clips. No window = byte-identical.
  'edit.grade_window': { clip: string; shape?: 'rect' | 'ellipse' | 'polygon'; points?: [number, number][]; feather?: number; invert?: boolean; contrast?: number; brightness?: number; saturation?: number; gamma?: number; temperature_k?: number; lut?: string; enabled?: boolean; remove_index?: number; rationale?: string }
  // Grade GALLERY (the grade gallery — copy a look between shots).
  // grade.save snapshots a clip's grade as a named preset; grade.apply copies a saved
  // look onto a target clip (lowers to edit.grade); grade.list reads the gallery.
  'grade.save': { clip?: string; name: string; rationale?: string }
  'grade.apply': { clip: string; name: string; rationale?: string }
  'grade.list': Record<string, never>
  'edit.color_space': { clip: string; input?: 'rec709' | 'rec2020' | 'srgb' | 'linear'; rationale?: string }
  // Match a clip's COLOUR/tonality to a REFERENCE clip ("make this shot match
  // that shot"). Samples one mid-clip frame per side, derives an edit.grade
  // correction (RGB mean/std transfer) scaled by strength (0..1), applies it via
  // the normal grade path. Committed as an edit.grade op (replay-safe).
  'edit.color_match': { clip: string; reference: string; strength?: number; rationale?: string }
  // ONE-CLICK REFERENCE-FREE auto white-balance + exposure ("Auto Color" /
  // "Balance Color"). Samples one mid-clip frame, neutralises the frame's OWN
  // colour cast (gray_world: whole-frame average → grey; white_patch: bright
  // near-neutral highlights → white) + nudges exposure to a mid target, derives
  // + applies an edit.grade scaled by strength (0..1; 0 = identity). Committed as
  // an edit.grade op (replay-safe).
  'edit.auto_balance': { clip: string; strength?: number; mode?: 'gray_world' | 'white_patch'; rationale?: string }
  // AI background removal/replace (per-clip MATTE; baked straight-alpha, no green
  // screen). mode=remove (default) reveals the lower track (overlay clips only);
  // replace fills behind with bg. enabled:false clears. model rvm (default) | matanyone.
  'edit.matte': { clip: string; mode?: 'remove' | 'replace'; model?: 'rvm' | 'matanyone'; quality?: 'fast' | 'good'; seed?: { at_ms?: number; point?: [number, number]; bbox?: [number, number, number, number] }; bg?: { type: 'color'; color: string } | { type: 'asset'; asset: string }; enabled?: boolean; rationale?: string }
  // Per-clip visual effects (SET semantics — replaces the list; [] clears). Each
  // item is one ClipEffect (typed by `type`); chroma_key is overlay-clips only.
  'edit.effect': { clip: string; effects?: ClipEffect[]; rationale?: string }
  // ADJUSTMENT LAYER (non-destructive): a grade/look effect over range_ms applied
  // to the COMPOSITE of everything beneath it (adjustment layer),
  // NOT per-clip. v1 = the top-most composite layer (a TIME-GATED pass). `grade` is the
  // edit.grade object shape; effect/effects are edit.effect's VISUAL look effects (audio
  // effects + chroma_key are refused). At least one of grade/effect(s). track is advisory.
  'edit.adjustment': { range_ms: [number, number]; grade?: { contrast?: number; brightness?: number; saturation?: number; gamma?: number; temperature_k?: number; lut?: string }; effect?: ClipEffect; effects?: ClipEffect[]; track?: string; rationale?: string }
  // Reverse playback (enabled defaults true; false restores normal). Duration unchanged.
  'edit.reverse': { clip: string; enabled?: boolean; rationale?: string }
  // Video stabilization (2-pass vidstab; smooths camera shake). enabled defaults true.
  'edit.stabilize': { clip: string; smoothing?: number; enabled?: boolean; rationale?: string }
  // Freeze-frame: hold one source frame (at_ms into the clip) for the whole slot.
  'edit.freeze': { clip: string; at_ms?: number; enabled?: boolean; rationale?: string }
  // Keyframe a parameter over a clip (opacity → video alpha; the base blends
  // against black, overlays over lower layers; volume → audio gain).
  'edit.keyframe': {
    clip: string
    // scale = animated zoom (multiplier, 1=native, clamped [1,10]); the eased
    // multi-point generalization of edit.animate (mutually exclusive with it).
    param: KfParam
    points?: { t_ms: number; value: number }[]
    // linear/hold OR a Penner ease_* curve (quad/cubic/expo/back/elastic/bounce ×
    // in/out/in_out) — eased motion reads professional rather than mechanical.
    interp?: KfInterp
    group_id?: string
    rationale?: string
  }
  // Emphasis-driven PUNCH-IN zooms (the short-form / talking-head look). Detects
  // loud beats (trigger 'energy', default — momentary-loudness peaks) or sentence
  // starts (trigger 'transcript') from perception, and lowers each to a scale
  // keyframe ramp 1.0→1+intensity→1.0, committed via edit.keyframe(param:'scale').
  // intensity default 0.12, clamped [0,0.5]; 0 / no peaks = clean no-op.
  'edit.auto_zoom': { clip: string; intensity?: number; max_zooms?: number; hold_ms?: number; trigger?: 'energy' | 'transcript'; rationale?: string }
  // Vector/freeform mask: a region effect (blur/pixelate/black) scoped to a shape on
  // a base-track video clip. enabled:false clears. Points are fractions of frame W/H.
  'edit.add_mask': {
    clip: string
    shape?: 'rect' | 'ellipse' | 'polygon'
    points?: [number, number][]
    feather?: number
    invert?: boolean
    effect?: 'blur' | 'pixelate' | 'black'
    strength?: number
    enabled?: boolean
    rationale?: string
  }
  // Redaction: time-bounded (range_ms) blur/pixelate/box of a shape region — privacy
  // for passwords/keys/faces/PII. Security-framed mask with an over-blur fail-safe.
  'edit.redact': {
    clip: string
    shape?: 'rect' | 'ellipse' | 'polygon'
    points?: [number, number][]
    mode?: 'blur' | 'pixelate' | 'box'
    feather?: number
    invert?: boolean
    strength?: number
    range_ms?: [number, number]
    track?: { t_ms: number; cx: number; cy: number }[]
    // Multi-region: extra boxes blur at once (faces / plates), sharing mode/strength.
    boxes?: { shape: 'rect' | 'ellipse' | 'polygon'; points: [number, number][]; track?: { t_ms: number; cx: number; cy: number }[] }[]
    ocr_auto?: boolean
    // Face auto-detect: blur every face at `at_ms` (YuNet). Computes shape/points/boxes.
    faces?: boolean
    track_faces?: boolean
    at_ms?: number
    pii?: ('email' | 'api_key' | 'aws_key' | 'jwt' | 'ssn' | 'ip' | 'credit_card' | 'keyword' | 'secret')[]
    enabled?: boolean
    rationale?: string
  }
  // Multicam audio sync: align ≥2 clips of the same event by cross-correlating audio.
  'edit.multicam_sync': { clips: string[]; reference?: string; max_offset_ms?: number; rationale?: string }
  // AUTO active-speaker multicam switching: cut a `program` video track to the loudest
  // synced angle over time (energy from each angle's Loudness.windows; orchestrator).
  'edit.multicam_switch': { tracks?: string[]; min_shot_ms?: number; reference_track?: string; mode?: 'speaker' | 'energy'; diarize_asset?: string; rationale?: string }
  // Motion tracking (cv2 CSRT→template): seed bbox/point → pos_x/pos_y(/scale) keyframe
  // arrays that follow the subject. Measurement (non-mutating); pipe into edit.keyframe.
  'edit.track': {
    clip: string
    bbox?: [number, number, number, number]
    point?: [number, number]
    start_ms?: number
    end_ms?: number
    every_ms?: number
    engine?: 'auto' | 'csrt' | 'kcf' | 'mil' | 'template'
    track_scale?: boolean
    rationale?: string
  }
  // Ken Burns pan/zoom: a preset (+amount) OR raw from/to {zoom,x,y}. enabled:false clears.
  'edit.animate': {
    clip: string
    preset?: 'zoom_in' | 'zoom_out' | 'pan_left' | 'pan_right' | 'pan_up' | 'pan_down'
    amount?: number
    from?: { zoom?: number; x?: number; y?: number }
    to?: { zoom?: number; x?: number; y?: number }
    enabled?: boolean
    rationale?: string
  }
  // Parametric audio EQ: a preset OR raw high/low-pass + peaking bands. enabled:false clears.
  'edit.eq': {
    clip: string
    preset?: 'voice' | 'warmth' | 'de_rumble' | 'phone' | 'de_ess' | 'brighten'
    high_pass_hz?: number
    low_pass_hz?: number
    bands?: { freq_hz: number; gain_db: number; q?: number }[]
    enabled?: boolean
    rationale?: string
  }
  // One-shot talking-head/podcast voice chain (eq:voice + denoise+gate+compressor) on
  // audio clip(s) under one auto-checkpoint. clip = one; track = that audio track; neither = all.
  'audio.cleanup_voice': {
    clip?: string
    track?: string
    strength?: 'light' | 'medium' | 'strong'
    rationale?: string
  }
  // Animated-PiP slide: slide an overlay in/out from a screen edge (lowers to pos_x/pos_y keyframes).
  'edit.slide': {
    clip: string
    edge: 'left' | 'right' | 'top' | 'bottom'
    mode?: 'in' | 'out'
    slide_ms?: number
    rationale?: string
  }
  // Review comments → agent change loop.
  'comment.add': { at_ms: number; text: string; end_ms?: number; author?: string; rationale?: string }
  'comment.export': { path?: string; allow_stale?: boolean }
  'comment.import': { path: string; allow_stale?: boolean; rationale?: string }
  'comment.list': { status?: 'open' | 'addressed' | 'dismissed' }
  'comment.resolve': { comment_id: string; status: 'open' | 'addressed' | 'dismissed'; rationale?: string }
  'comment.draft': { comment_id: string; rationale?: string }
  'comment.apply': { comment_id: string; rationale?: string }
  // linked defaults true for live calls: an exact imported A/V counterpart
  // moves in the same atomic action. linked:false deliberately moves one clip.
  // ripple false keeps unrelated tracks fixed; true opens time on every other
  // media track after both linked destinations are populated.
  'edit.move': { clip: string; to_track: string; at_ms: number; ripple?: boolean; linked?: boolean; rationale?: string }
  // duration_ms is REQUIRED for still-image assets (no intrinsic duration),
  // invalid for timed media; mutually exclusive with src_range_ms. ripple:
  // omitted = resolved from the target track (base → true for AV sync,
  // overlay/extra → false, overlays float).
  'edit.insert': { asset: string; track: string; at_ms: number; src_range_ms?: [number, number]; duration_ms?: number; ripple?: boolean; rationale?: string; group_id?: string }
  // Duplicate a clip (NLE Ctrl+D): copy a clip + place the copy IMMEDIATELY AFTER
  // it on the same track, rippling the rest. The copy carries the SAME asset +
  // source range AND ALL per-clip attributes (effects/grade/transform/crop/fade/
  // speed/speed_ramp/reverse/freeze/matte/mask/eq/stabilize/keyframes/gain — the
  // whole clip minus its id; a TRUE copy, unlike edit.paste's pristine clip). A
  // muxed video clip's linked audio sibling is duplicated too (aligned pair,
  // grouped for undo). Lowers to one (or two, grouped) replay-safe edit.duplicate
  // core ops. v1 copies the whole clip (no sub-range/overwrite).
  'edit.duplicate': { clip: string; rationale?: string }
  // Compound clip / NEST: collapse a contiguous run of clips on one track into a
  // single nested sub-timeline; baked at render time.
  'edit.nest': { clips: string[]; name?: string; rationale?: string }
  // 3-point REPLACE EDIT (a three-point replace edit): swap target_clip's slot with new
  // source (asset OR source_clip), preserving the target's id, position + slot
  // duration. Keeps the look; resets speed/ramp/freeze; short source is clamped and
  // the slot remainder padded with a gap (gap_ms in the receipt). A muxed video
  // clip's linked audio sibling is replaced too (link_audio default true; in-place
  // equal-duration swaps — no ripple, grouped for undo). Replay-safe (no new id).
  'edit.replace': { target_clip: string; asset?: string; source_clip?: string; source_in_ms?: number; source_out_ms?: number; link_audio?: boolean; group_id?: string; rationale?: string }
  // FIT TO FILL: drop source (asset OR source_clip) into an EMPTY slot/gap
  // and SPEED-ADJUST it (speed = source_span/slot) so it fills the slot exactly with
  // NO downstream shift. duration_ms omitted = fill the gap at at_ms. The fit speed
  // must be within the 0.25–4.0× retime range. Single track, no linked audio. Lowers
  // to one replay-safe edit.fit_to_fill core op (placed id pinned via added_clip).
  'edit.fit_to_fill': { track: string; at_ms: number; duration_ms?: number; asset?: string; source_clip?: string; source_in_ms?: number; source_out_ms?: number; rationale?: string }
  // EXTRACT/promote a video clip's audio onto its own editable audio track (the
  // "Detach Audio" affordance). This engine has no clip-level A/V link and a
  // video clip's audio never renders, so this does NOT unlink — it RECOVERS the
  // silently-dropped audio as an independent, movable, J/L-splittable clip
  // (added_to_render:true). If a sibling audio clip already exists → a clean
  // no-op {detached:false}. Lowers to one edit.insert (+ add_track only if no
  // audio track exists). Refuses a retimed (speed!=1) clip in v1.
  'edit.detach_audio': { clip: string; rationale?: string }
  // J-cut / L-cut (split edit): roll the AUDIO transition relative to the VIDEO
  // cut at a clip boundary so one clip's audio leads ("j") or lags ("l") its
  // video. A pure roll of the two linked a1t clips around the cut (lowers to two
  // edit.trim ops; the video is untouched). Requires the audio to already be two
  // clips butted at the cut; refuses a retimed audio clip.
  'edit.split_edit': {
    at_ms: number
    kind: 'j' | 'l'
    offset_ms: number
    video_track?: string
    audio_track?: string
    rationale?: string
  }
  // Copy/Cut/Paste's PASTE half — a THIN verb that resolves the source `clip`
  // → (asset, src_in, src_out) and lowers to edit.insert. `clip` OR a snapshot
  // fallback {asset, src_range_ms} (a since-deleted source still pastes). On a
  // retimed clip a `src_range_ms` sub-range is refused. link_audio (default
  // true) also pastes a muxed video clip's linked sibling audio. ripple omitted
  // = resolved from the destination track (base → true, overlay/extra → false).
  'edit.paste': { clip?: string; asset?: string; to_track: string; at_ms: number; src_range_ms?: [number, number]; ripple?: boolean; link_audio?: boolean; rationale?: string }
  'edit.gain': { clip?: string; track?: string; db: number; rationale?: string }
  // Video-layer geometry + opacity, normalized 0..1; base transforms use a black
  // canvas, overlays composite over lower tracks; identity (0,0,1,1) clears.
  'edit.transform': { clip: string; x?: number; y?: number; scale?: number; opacity?: number; rationale?: string }
  // Move a track to a new stacking index (z-order); index clamps to [0, count-1].
  // index is relative to tracks of the same kind, never project.tracks absolute.
  'edit.reorder_track': { track: string; index: number; rationale?: string }
  // Layer blend mode for an overlay video track (multiply/screen/…); 'normal' clears.
  'edit.blend': { track: string; mode?: 'normal' | 'multiply' | 'screen' | 'overlay' | 'darken' | 'lighten' | 'difference' | 'addition' | 'subtract' | 'softlight' | 'hardlight'; rationale?: string }
  // Visual output visibility for video/caption tracks. Audio uses edit.mute.
  'edit.track_visible': { track: string; on: boolean; rationale?: string }
  // Persisted timeline edit lock. The UI blocks drag/trim/drop gestures on locked rows.
  'edit.track_lock': { track: string; on: boolean; rationale?: string }
  // NON-DESTRUCTIVE per-track mute/solo FLAGS — gain is never touched (the dialed
  // level survives a reload). Audibility: a track plays iff !muted && (no track
  // soloed || this track soloed); explicit mute wins over solo. Audio/video tracks.
  'edit.mute': { track: string; on: boolean; rationale?: string }
  'edit.solo': { track: string; on: boolean; rationale?: string }
  // Non-destructive per-track stereo balance, -1..1, 0 = center.
  'edit.pan': { track: string; pan: number; rationale?: string }
  // Paste a source clip's attributes onto N targets (orchestrator).
  'edit.paste_attributes': {
    from_clip: string
    to_clips: string[]
    which: Array<'grade' | 'transform' | 'speed' | 'volume' | 'effects'>
    rationale?: string
  }
  // Pro trim trio: slip = shift a clip's source window
  // (source ms); roll = move the cut between two adjacent clips (timeline ms);
  // slide_edit = move a clip between its media neighbors (timeline ms).
  'edit.slip': { clip: string; by_ms: number; rationale?: string }
  'edit.roll': { track: string; at_ms: number; by_ms: number; rationale?: string }
  'edit.slide_edit': { clip: string; by_ms: number; rationale?: string }
  // Linear fades, clip-local ms (0 clears a side). Exactly one of clip|track;
  // track form resolves NOW to its first (in) / last (out) media clip.
  'edit.fade': { clip?: string; track?: string; in_ms?: number; out_ms?: number; kind?: 'audio' | 'video' | 'both'; rationale?: string }
  // Non-destructive mute range (SOURCE-asset ms — the src_in/src_out clock).
  // Exactly one of range_ms (add) | remove_ms (surgical unmute) | clear:true.
  // Audio-track clips only.
  'edit.mute_range': { clip: string; range_ms?: [number, number]; remove_ms?: [number, number]; clear?: boolean; rationale?: string }
  // Source crop rectangle in SOURCE PIXELS (the rect to KEEP); identity
  // (origin + full source size) clears. Video media clips only.
  'edit.crop': { clip: string; x: number; y: number; w: number; h: number; rationale?: string }
  // Crossfade (dissolve) the cut at at_ms between two ADJACENT media clips.
  // duration_ms is the overlap (0 clears back to a hard cut); the timeline
  // SHORTENS by duration_ms across the crossfade (NLE centred dissolve).
  // transition = the VIDEO seam style (ffmpeg xfade name; omit/"fade" = dissolve).
  // Audio is always a smooth crossfade. See TRANSITIONS in cut-core for the set.
  'edit.crossfade': { track: string; at_ms: number; duration_ms: number; transition?: string; rationale?: string }
  // Windowed-gain ducking: music_track gain reduced by db inside speech
  // windows computed from against_track's perception (db negative).
  'edit.duck': { music_track: string; against_track: string; db: number; attack_ms?: number; rationale?: string }
  // New empty compositing track; ids deterministic (v{N} / a{N}t) unless given.
  'edit.add_track': { kind: 'video' | 'audio'; id?: string; rationale?: string }
  'edit.remove_track': { track: string; force?: boolean; rationale?: string }
  // the two-segment verb-name contract: two-segment names — edit.add_marker / edit.remove_marker.
  'edit.add_marker': { at_ms: number; label: string; note?: string; rationale?: string }
  'edit.remove_marker': { id: string; rationale?: string }
  // Reposition a marker, id PRESERVED (remove+re-add would mint a new id).
  'edit.move_marker': { id: string; at_ms: number; rationale?: string }
  // Relabel/recolor/edit a marker note in ONE op (id + position kept).
  'edit.update_marker': { id: string; label?: string; color?: MarkerColor | 'none'; note?: string; rationale?: string }
  'edit.seek_marker': { from_ms?: number; direction?: 'next' | 'prev' | 'first' | 'last'; id?: string }
  // mode (op-rebase, engine commits 4408dbd/211f810): "tip" (DEFAULT) undoes
  // the LATEST timeline op via its full-timeline snapshot inverse — a deeper
  // target returns a guardrail error naming the later ops. "rebase" SELECTIVELY
  // undoes THIS op while KEEPING later ops (id-pinned skip-replay), refused with
  // a guardrail error naming the dependents if any later op references an id the
  // target created. Both modes APPEND (the log is never rewritten, the append-only operation-log contract).
  'edit.restore': { op_id: string; mode?: 'tip' | 'rebase'; rationale?: string }
  // Auto shot-detection: split the clip at every scene boundary, or mark them
  // (navigate vs cut). trim_edges = top-and-tail dead air (speech-anchored).
  'edit.split_at_scenes': { asset: string; track?: string; min_shot_ms?: number; rationale?: string }
  'edit.mark_scenes': { asset: string; track?: string; label_prefix?: string; rationale?: string }
  'edit.trim_edges': { keep_pad_ms?: number; min_trim_ms?: number; rationale?: string }

  // Audio domain — place an imported music asset as a BED + (default)
  // auto-duck under speech + surface beat:N markers. duck:false skips ducking;
  // duck object tunes against_track/db/attack_ms.
  'audio.add_music': {
    asset: string
    track?: string
    at_ms?: number
    src_range_ms?: [number, number]
    fit_to_timeline?: boolean
    bed_gain_db?: number
    duck?: boolean | { against_track?: string; db?: number; attack_ms?: number }
    beat_markers?: boolean
    rationale?: string
  }
  // Native AI DUBBING: re-voice an asset's speech into target_lang in a cloned
  // voice (default 'rebeka'), time-fit to the original, added as a NEW audio
  // track at the original segment timings (original audio kept = a mutable mix).
  // Reuses transcript.translate for translation; synthesizes each segment via the
  // OmniVoice TTS service (CUT_DUB_ENDPOINT). asset optional when exactly one is
  // transcribed. backend/model/timeout_ms tune the TRANSLATION step.
  'audio.dub': { target_lang: string; asset?: string; voice?: string; source_lang?: string; backend?: 'auto' | 'cli' | 'local'; model?: string; timeout_ms?: number; rationale?: string }

  'transcript.get': { asset: string }
  // EDL-aware transcript: words mapped to their timeline positions. `clip` →
  // SELECTED-CLIP view; `track` → one track; both omitted → PROGRAM transcript.
  'transcript.timeline': { clip?: string; track?: string }
  // Find a phrase → word ranges (feed straight into cut_words/assemble).
  'transcript.search': { asset: string; query: string; case_sensitive?: boolean }
  // Auto-chapter a transcript into topic chapters (TextTiling, no model).
  // NON-MUTATING: returns the chapter list to drop markers / export.chapters.
  'transcript.chapters': { asset: string; max_chapters?: number; min_gap_ms?: number }
  // `clip` scopes the cut to one clip (selected-clip view); omit to cut everywhere.
  'transcript.cut_words': { asset: string; word_range: [number, number]; clip?: string; rationale?: string }
  // Non-destructive transcript ignore — captions/assemble skip it; source text remains.
  'transcript.ignore_words': { asset: string; word_range: [number, number]; remove?: boolean; rationale?: string }
  // Non-destructive word mute — keeps timing/AV sync (the mute sibling of cut_words).
  'transcript.mute_words': { asset: string; word_range: [number, number]; clip?: string; rationale?: string }
  // Highlight reel: assemble the given word ranges (in order) into clips. The
  // Transcript reel tray dispatches this via dispatchVerb; typed here for parity.
  // Single-source (asset+word_ranges) OR multi-source (sources:[{asset,word_ranges}]).
  'transcript.assemble': { asset?: string; word_ranges?: [number, number][]; sources?: { asset: string; word_ranges: [number, number][] }[]; track?: string; audio_track?: string; at_ms?: number; pad_ms?: number; rationale?: string }
  // the required-argument contract: aggressiveness REQUIRED. the scope contract: timeline-wide by default,
  // optional asset/track narrows scope. allow_extreme overrides the totality
  // guard (refuses when the pass would remove >80% of the timeline).
  'transcript.remove_silences': { aggressiveness: 'calm' | 'natural' | 'jumpy'; min_ms?: number; padding_ms?: number; asset?: string; track?: string; allow_extreme?: boolean; rationale?: string }
  'transcript.remove_fillers': { lexicon?: string[]; asset?: string; track?: string; rationale?: string }
  // Auto-remove repeated line ATTEMPTS (retakes), keeping the best take.
  // Mirrors remove_fillers' scope + ripple-cut; detection = utterance
  // segmentation + token-sequence similarity. keep: last|first|longest.
  'transcript.remove_retakes': { asset?: string; track?: string; similarity?: number; pause_ms?: number; keep?: 'last' | 'first' | 'longest'; min_words?: number; rationale?: string }
  // i18n (TEXT translation; no dubbing). Translate an asset's transcript into
  // target_lang → a sibling receipts/<asset>.<lang>.words.json (source kept).
  // backend: auto (CLI agent claude/codex/grok if available, else the local
  // Opus-MT/MADLAD sidecar) | cli | local. source_lang required for local
  // (Opus-MT is per-pair). asset optional when exactly one is transcribed.
  'transcript.translate': { target_lang: string; asset?: string; source_lang?: string; backend?: 'auto' | 'cli' | 'local'; model?: string; timeout_ms?: number; rationale?: string }

  'captions.generate': { style_ref?: string; rationale?: string }
  // i18n (TEXT translation; no dubbing). Translate the caption cues into
  // target_lang, preserving each cue's EXACT range_ms (one source cue → one
  // target cue). backend auto/cli/local (CLI = the same subscription agent
  // agent.chat uses; local = Opus-MT/MADLAD sidecar). mode:track (default) adds
  // a target-language track (bilingual, top position); mode:replace overwrites
  // the source cues in place. reflow:true retimes the translated cues only in
  // mode:track; replace mode preserves existing cue timing/count.
  'captions.translate': { target_lang: string; source_lang?: string; backend?: 'auto' | 'cli' | 'local'; mode?: 'track' | 'replace'; source_track?: string; position?: 'bottom' | 'top' | 'center'; reflow?: boolean; model?: string; timeout_ms?: number; rationale?: string }
  'captions.import': { path: string; replace?: boolean; style_ref?: string; rationale?: string }
  // Timed text card on the dedicated txt1 track (separate from cap1 so
  // regeneration never wipes cards). style_ref XOR position.
  'captions.add_text': { text: string; range_ms: [number, number]; style_ref?: string; position?: 'bottom' | 'top' | 'center'; rationale?: string }
  // the two-segment verb-name contract: captions.set_style. the canonical-export contract: captions.export_srt is DROPPED —
  // export.srt is the only SRT exporter.
  'captions.set_style': { ref: string; style: CaptionStyle; rationale?: string }
  // Caption style gallery: save/apply/list presets.
  'captions.save_style': { name: string; ref?: string; style?: CaptionStyle; rationale?: string }
  'captions.apply_style': { name: string; ref?: string; rationale?: string }
  'captions.list_styles': Record<string, never>
  // Set a caption clip's absolute timeline range (retime = shift both edges,
  // trim = move one). edit.move/edit.trim REFUSE caption clips, so this is the
  // ONLY way to reposition a caption clip via direct manipulation.
  'captions.set_range': { clip: string; range_ms: [number, number]; rationale?: string }
  // Edit an EXISTING caption's words (+ optional style switch) in place, by clip id —
  // the companion to set_range (retime). add_text only ADDS; this EDITS, so a placed
  // caption is fully editable from the Inspector. caption-editing regression fix.
  'captions.set_text': { clip: string; text: string; style_ref?: string; rationale?: string }
  // The fix that satisfies verify.captions (measure→remedy): split over-length
  // cues + extend too-fast cues into gaps. captions.shift = bulk sync offset.
  'captions.reflow': { max_cps?: number; max_chars?: number; max_duration_ms?: number; min_gap_ms?: number; rationale?: string }
  'captions.shift': { offset_ms: number; rationale?: string }
  // Animate the cap1 cues as a kinetic overlay. replace_static
  // removes the animated static cues so the overlay shows alone (range-aware).
  'captions.kinetic': { range_ms?: [number, number]; position?: 'bottom' | 'center' | 'top'; color?: string; font_px?: number; replace_static?: boolean; per_word?: boolean; rationale?: string }

  // Native motion-graphics title over a timed span (preset =
  // lower_third | title_card). Distinct from captions.add_text's static card.
  'title.add': { text: string; range_ms: [number, number]; preset?: 'lower_third' | 'title_card' | 'top_bar' | 'subtitle' | 'headline'; animation?: 'fade' | 'slide_up' | 'slide_down' | 'slide_left' | 'slide_right' | 'pop' | 'none'; template?: 'typewriter' | 'word_pop' | 'slide_stack' | 'kinetic_emphasis' | 'lower_third_reveal' | 'caption_karaoke'; accent?: string; emphasis?: string; font_px?: number; color?: string; bg?: boolean; x?: number; y?: number; align?: 'left' | 'center' | 'right'; group_id?: string; rationale?: string }
  // Edit a PLACED title's TEXT (and/or style) in place, by clip id — the edit
  // companion to title.add (which only ADDS). Recovers the originating spec from
  // the op-log, merges overrides (text = common case), re-renders the overlay at
  // the same duration, and swaps the clip's asset. title-editing regression fix.
  'title.update': { clip: string; text?: string; preset?: 'lower_third' | 'title_card' | 'top_bar' | 'subtitle' | 'headline'; animation?: 'fade' | 'slide_up' | 'slide_down' | 'slide_left' | 'slide_right' | 'pop' | 'none'; template?: 'typewriter' | 'word_pop' | 'slide_stack' | 'kinetic_emphasis' | 'lower_third_reveal' | 'caption_karaoke'; accent?: string; emphasis?: string; font_px?: number; color?: string; bg?: boolean; x?: number; y?: number; align?: 'left' | 'center' | 'right'; group_id?: string; rationale?: string }
  'title.templates': Record<string, never>
  'edit.add_shape': { shape: 'rect' | 'ellipse' | 'line' | 'arrow'; range_ms: [number, number]; x?: number; y?: number; w?: number; h?: number; x2?: number; y2?: number; fill?: string; stroke?: string; stroke_px?: number; opacity?: number; radius_px?: number; head_px?: number; text?: string; color?: string; font_px?: number; animation?: 'fade' | 'slide_up' | 'slide_down' | 'slide_left' | 'slide_right' | 'pop' | 'none'; group_id?: string; rationale?: string }
  // Edit a PLACED shape overlay clip's props (kind, label text, color, geometry) in
  // place, by clip id — the edit companion to edit.add_shape (which only ADDS).
  // Recovers the originating edit.add_shape spec from the op-log, merges overrides
  // (label/color = common cases), re-renders the overlay at the same duration, and
  // swaps the clip's asset. `label` is the public name for the centered label text
  // (edit.add_shape calls it `text`). shape-editing regression fix.
  'shape.update': { clip: string; shape?: 'rect' | 'ellipse' | 'line' | 'arrow'; label?: string; color?: string; fill?: string; stroke?: string; stroke_px?: number; opacity?: number; radius_px?: number; head_px?: number; font_px?: number; animation?: 'fade' | 'slide_up' | 'slide_down' | 'slide_left' | 'slide_right' | 'pop' | 'none'; x?: number; y?: number; w?: number; h?: number; x2?: number; y2?: number; group_id?: string; rationale?: string }
  'assets.providers': Record<string, never>
  'assets.search': { provider: 'local_folder' | 'openverse' | 'archive_org' | 'wikimedia' | 'nasa' | 'stickers'; q: string; kind?: 'audio' | 'image' | 'video'; limit?: number; dir?: string; rationale?: string }
  'assets.fetch': { provider: 'local_folder' | 'openverse' | 'archive_org' | 'wikimedia' | 'nasa' | 'stickers'; id: string; kind?: 'audio' | 'image' | 'video'; dir?: string; rationale?: string }
  'assemble.broll': { slots: { query: string; at_ms: number; duration_ms: number }[]; source?: 'search' | 'generate'; provider?: string; dir?: string; kind?: 'video' | 'image' | 'audio'; track?: string; rationale?: string }
  'assemble.repurpose': { asset: string; count?: number; target_ms?: number; prompt?: string }
  'assemble.shorts': { asset: string; count?: number; target_ms?: number; aspect?: '9:16' | '1:1' | '4:5' | '16:9'; prompt?: string }
  'assemble.from_script': { asset: string; script: string; min_score?: number }
  'score.clip': { clip?: string; asset?: string; range_ms?: [number, number] }
  // Integrated Cut recorder (doctor + autoedit + the polish orchestrator
  // + a fenced file export). config (autoedit) is accepted but currently ignored.
  'screen_record.doctor': { warm_mic?: boolean }
  // Live duration-bounded capture. `start` launches an in-process
  // recorder thread and returns a capture_id; `stop` polls for the
  // finalized project.json then surfaces the events track (+ optional autoedit).
  // `monitor` is accepted-but-ignored in v1 (the record CLI has no --monitor flag).
  'screen_record.start': { duration_ms?: number; fps?: number; audio?: boolean; system_audio?: boolean; studio?: unknown; keys?: boolean; monitor?: number; window?: string; rationale?: string }
  'screen_record.stop': { capture_id: string; autoedit?: boolean; mux_raw?: boolean; raw_path?: string; rationale?: string }
  'screen_record.studio_event': {
    capture_id: string
    event: {
      t_ms: number
      source: 'camera' | 'recording' | 'background'
      kind: 'visibility' | 'transform' | 'marker' | 'style'
      visible?: boolean
      x?: number
      y?: number
      size?: number
      shape?: 'circle' | 'rounded_rect'
      radius?: number
      label?: string
      background?: 'none' | 'blur_screen' | 'solid' | 'gradient'
    }
  }
  'screen_record.autoedit': { track: string; config?: Record<string, unknown>; webcam?: string; studio_events?: string }
  'screen_record.polish': { source: string; plan: string; track?: string; at_ms?: number; rationale?: string; raw?: boolean }
  'screen_record.export': { source: string; plan: string; path?: string; format?: 'mp4' | 'gif'; gif_fps?: number; gif_width?: number }
  'assets.generate': {
    prompt: string
    provider: 'codex' | 'grok'
    kind?: 'image' | 'video'
    model?: string
    references?: string[]
    variation?: string
    placement?:
      | { mode: 'insert'; track: string; at_ms: number; duration_ms: number }
      | { mode: 'replace'; target_clip: string }
    timeout_ms?: number
    rationale?: string
  }
  'assets.generated_list': { kind?: 'image' | 'video'; limit?: number }
  'agent.chat': { message: string; attachments?: string[]; agent?: 'claude' | 'codex' | 'grok'; model?: string; timeout_ms?: number }
  'plugins.list': Record<string, never>
  'plugins.enable': { name: string; enabled?: boolean; rationale?: string }
  'plugins.call': { plugin: string; verb: string; args?: Record<string, unknown>; rationale?: string }

  // at_ms optional: draft mode previews the WHOLE timeline (ignores at_ms).
  'render.preview': { at_ms?: number; duration_ms?: number; draft?: boolean }
  // h = scale height; compose = exact composed frame (captions/overlays) vs fast proxy seek.
  'render.frame': { at_ms: number; h?: number; compose?: boolean; inline?: boolean }
  // Contact-sheet "see the whole edit at a glance" view: N evenly-spaced frames
  // of the COMPOSED timeline tiled into a grid (the agent/judge overview).
  // count = frames to sample (engine default applies when omitted); h = per-frame
  // height in px; compose = composed timeline (default) vs raw source; inline =
  // return the JPEG as base64 in the envelope (no extra /api fetch). Pure view —
  // creates no op, mutates no timeline (zero-local-mutation contract display-only).
  'render.storyboard': { count?: number; h?: number; compose?: boolean; inline?: boolean }
  // preset = quality tier; profile = footage profile for the auto-run check
  // battery (silent_screen_demo waives lufs/captions/edge-silence — recorded
  // as waived_by_profile, measured outcome kept).
  // Fit/resolution + reframe (aspect|width+height → explicit output geometry
  // for THIS render only, project untouched: multi-format publish) + dry_run
  // (return the plan, no encode) + normalize_loudness (LUFS target).
  // format = output FILE FORMAT (codec/container): h264 universal mp4 (default,
  // byte-identical to omitting it), hevc smaller H.265 mp4, vp9 web .webm, prores
  // pro .mov, av1 highest-quality mp4. hardware = encoder tier: 'auto' uses the
  // GPU encoder (NVENC/QSV/AMF/VideoToolbox) when present (much faster, probe-
  // verified with safe software fallback), 'off' forces byte-deterministic software.
  'render.final': { path?: string; preset?: 'draft' | 'standard' | 'high'; format?: 'h264' | 'hevc' | 'vp9' | 'prores' | 'av1'; hardware?: 'auto' | 'off'; bitrate?: string; rate_control?: 'vbr' | 'cbr'; audio_bitrate?: string; target_size_mb?: number; profile?: 'talking_head' | 'silent_screen_demo'; fit?: 'contain' | 'cover'; resolution?: 'project' | 'match_source'; aspect?: string; width?: number; height?: number; normalize_loudness?: number; dry_run?: boolean; rationale?: string }
  // SUBJECT-AWARE auto-reframe (perception contract): render the edit → subject instrument →
  // moving-crop post-pass to `aspect`. The HONEST alt to render.final{aspect,fit:cover}
  // (a naive static centre-crop). Returns {reframe_id, job_id}; receipt in the job result.
  'render.reframe': { aspect: string; preset?: 'talking_head' | 'sports' | 'pets' | 'cars' | 'general'; path?: string; direction?: Record<string, { cx?: number; mode?: 'widen' }>; rationale?: string }
  // Director model: render.direct builds the per-scene
  // contact sheet the foundation model (or the human) reads; render.qc reviews a
  // reframe output. Both are jobs — result lands in jobs.status.result.result.
  'render.direct': { preset?: 'talking_head' | 'sports' | 'pets' | 'cars' | 'general'; rationale?: string }
  'render.qc': { reframe_id: string; preset?: 'talking_head' | 'sports' | 'pets' | 'cars' | 'general'; rationale?: string }

  'verify.checks': { render_id?: string }
  // backend selects a rung of the judge access ladder. "auto" (default; "cli"
  // is a backward-compatible alias) walks claude→codex→antigravity→grok and
  // runs the first detected subscription CLI; a named rung forces it (honest
  // not_run if its CLI is absent). The rung set mirrors shellX's providers.
  'verify.judge': { render_id?: string; backend?: 'auto' | 'cli' | 'claude' | 'codex' | 'antigravity' | 'grok' }
  // Read-only QC receipts (no render needed): visual pacing, caption QC vs
  // timed-text standards, verbal pacing (WPM/fillers), brand conformance.
  'verify.pacing': Record<string, never>
  // PRE-render predictive gate: flags likely render problems (empty_tail/
  // black_or_frozen HIGH, slideshow/silent/tiny MED, uniform_border LOW) from the
  // current EDL + cached perception facts WITHOUT spending a render. No args.
  'verify.pregate': Record<string, never>
  'verify.captions': { max_cps?: number; min_duration_ms?: number; max_duration_ms?: number; min_gap_ms?: number; max_chars?: number }
  'verify.delivery': { asset?: string; lexicon?: string[]; min_wpm?: number; max_wpm?: number; max_fillers_per_min?: number; pause_gap_ms?: number }
  // Integrated-loudness (LUFS) measurement receipt for one asset — the MEASURE half
  // of the loudness loop (NORMALIZE = render.final normalize_loudness).
  'verify.loudness': { asset: string; target_lufs?: number }
  // Video-scopes receipt for one frame (signalstats luma/clipping/legality/cast);
  // scope_images:true also renders vectorscope/waveform/histogram PNGs.
  'verify.scopes': {
    at_ms?: number
    asset?: string
    scope_images?: boolean
    kinds?: ('vectorscope' | 'waveform' | 'histogram')[]
    rationale?: string
  }
  'verify.brand': { fonts?: string[]; colors?: string[]; position?: 'bottom' | 'top' | 'center'; min_size?: number; max_size?: number; aspect?: string }

  // Receipted Autopilot: render→verify→self-fix→re-verify under one checkpoint.
  'autopilot.run': {
    goal?: string
    comment_id?: string
    policy?: 'preview' | 'auto_low_risk'
    max_fix_iters?: number
    rationale?: string
  }
  // Generate module: pure catalog reads, non-mutating PNG previews, native
  // insert, prompt planning, and storyboard planning/preview/insert.
  'generate.list': {
    kind?: 'title' | 'caption' | 'shape' | 'motion' | 'social' | 'batch' | 'all'
    source?: 'builtin' | 'project' | 'user' | 'all'
    query?: string
  }
  'generate.describe': { id: string }
  'generate.preview': {
    id: string
    params?: Record<string, unknown>
    width?: number
    height?: number
    frame_ms?: number
  }
  'generate.insert': {
    id: string
    params?: Record<string, unknown>
    at_ms?: number
    track?: string
    rationale?: string
  }
  'generate.from_prompt': {
    prompt: string
    policy?: 'plan' | 'preview' | 'insert'
    agent?: 'auto' | 'claude' | 'codex' | 'grok'
    template_id?: string
    at_ms?: number
    width?: number
    height?: number
    timeout_ms?: number
    context?: Record<string, unknown>
    rationale?: string
  }
  'generate.storyboard': {
    input: string
    mode?: 'auto' | 'quick_prompt' | 'director_brief' | 'script' | 'existing_media'
    policy?: 'plan' | 'preview' | 'insert'
    answers?: Record<string, unknown>
    context?: Record<string, unknown>
    agent?: 'auto' | 'claude' | 'codex' | 'grok'
    timeout_ms?: number
    rationale?: string
  }
  'motion.template_to_cut': {
    template?: string
    params?: Record<string, unknown>
    policy?: 'preview' | 'insert'
    out_dir?: string
    at_ms?: number
    track?: string
    duration_ms?: number
    dry_run_render?: boolean
    checkpoint?: boolean
    job_id?: string
    rationale?: string
  }
  'motion.script_to_cut': {
    script?: Record<string, unknown>
    script_path?: string
    policy?: 'preview' | 'insert'
    out_dir?: string
    at_ms?: number
    track?: string
    duration_ms?: number
    dry_run_render?: boolean
    checkpoint?: boolean
    job_id?: string
    rationale?: string
  }
  'motion.job.get': { job_id: string }
  'motion.job.list': { limit?: number }
  'motion.map_import': {
    path: string
    packageDir?: string
  }
  'motion.apply_import': {
    path: string
    packageDir?: string
    dryRun?: boolean
    background?: boolean
  }
  'motion.link.refresh': {
    clip: string
    preset?: 'mp4-h264' | 'mp4-h265'
    job_id?: string
    rationale?: string
  }
  'motion.link.relink': {
    clip: string
    package_dir: string
    rationale?: string
  }
  'motion.link.edit': { clip: string }
  'motion.link.tracking.inventory': { clip: string }
  'motion.link.tracking.request': {
    clip: string
    analysis_id: string
    asset_id: string
    mode: 'point' | 'planar'
    model: 'translation' | 'similarity' | 'homography'
    region: { x: number; y: number; width: number; height: number }
    reference_ms?: number
    start_ms?: number
    end_ms?: number
    every_ms?: number
    search_radius_px?: number
    confidence_floor?: number
    rationale?: string
  }
  'motion.link.tracking.inspect': { clip: string; analysis_id: string }
  'motion.link.tracking.apply': {
    clip: string
    analysis_id: string
    layer_id: string
    segment_index?: number
    include_low_confidence?: boolean
    rationale?: string
  }
  'motion.link.tracking.verify': { clip: string; layer_id: string; analysis_id?: string }
  'motion.link.tracking.detach': { clip: string; layer_id: string; rationale?: string }
  // Recipe layer: declarative pipeline manifests (named, gated workflows).
  'recipe.list': {}
  'recipe.describe': { name: string }
  'recipe.run': {
    name: string
    args?: Record<string, unknown>
    policy?: 'run' | 'dry_run'
    rationale?: string
  }
  // social repurposing: rank shareable windows, then bundle one per platform.
  'clip.candidates': { asset?: string; count?: number; min_ms?: number; max_ms?: number }
  'render.bundle': {
    range_ms?: [number, number]
    candidate?: { at_ms: number; dur_ms: number }
    platforms?: string[]
    preset?: 'draft' | 'standard' | 'high'
    normalize_loudness?: number
    brand_ref?: BrandKit
    rationale?: string
  }
  // BATCH DELIVERY: a batch render queue. Fan the ONE
  // current timeline out into N renders (each a render.final arg subset; `output`
  // aliases render.final's `path`), run SEQUENTIALLY through the same render.final
  // path. Returns {queue_id, count, jobs:[{idx, output}]}; per-entry job_ids +
  // receipts land in the queue job result (jobs.status{queue_id}) as each completes.
  'render.queue': {
    jobs: Array<Partial<VerbArgs['render.final']> & { output?: string }>
    rationale?: string
  }

  'export.frame': { at_ms: number; to_asset?: boolean; path?: string }
  'export.range': { range_ms: [number, number]; to_asset?: boolean; preset?: string; path?: string }
  'export.xml': { format: 'fcpxml' | 'premiere' | 'resolve'; path?: string }
  'export.otio': { path?: string; rationale?: string }
  'export.edl': { path?: string; title?: string; rationale?: string }
  'import.otio': { path: string; mode?: 'preview' | 'replace'; expected_hash?: string; rationale?: string }
  'export.audio': { format?: 'mp3' | 'm4a' | 'aac' | 'wav' | 'flac' | 'opus'; path?: string; track?: string; to_asset?: boolean; rationale?: string }
  'export.publish': { platform: 'youtube' | 'youtube_4k' | 'tiktok' | 'reels' | 'instagram_feed' | 'x' | 'square'; preset?: 'draft' | 'standard' | 'high'; hardware?: 'auto' | 'off'; path?: string; dry_run?: boolean; rationale?: string }
  'export.gif': { range_ms?: [number, number]; fps?: number; width?: number; dither?: 'floyd' | 'bayer' | 'none'; to_asset?: boolean; rationale?: string; path?: string }
  'export.srt': { path?: string }
  'export.vtt': { path?: string }
  // ASS/SSA styled captions; karaoke:true = word-level \k fill.
  'export.ass': { path?: string; karaoke?: boolean }
  'export.chapters': { path?: string }
  'export.transcript': { format?: 'txt' | 'md'; timestamps?: boolean; path?: string }

  'ui.state': Record<string, never>
  'ui.screenshot': { inline?: boolean }
  // debug.screenshot: server-side OS screenshot (works headless, unlike ui.screenshot).
  'debug.screenshot': { monitor?: number; window?: string; inline?: boolean }
  // wizard|environment open the env doctor surfaces (relay-drivable per
  // invariant 1 — an agent can pop the wizard the same way it drives any panel).
  // scopes opens the Review rail's Scopes tab; matte/shape open right-side
  // drawers; stock/find-media/search/find-moment/sequence-index are Find tabs.
  // Full set = UI_OPEN_PANELS (single source).
  'ui.open': { panel: UiOpenPanel }
  'ui.playhead': { at_ms: number }
  'ui.select': { clip_ids: string[] }
  // Agent-driven element highlight overlay (guided demos / debugging). Resolve by
  // ONE of selector|clip|panel; label/description show a chip; duration_ms=0 stays
  // until cleared by clear:true, the close button, or Escape.
  'ui.highlight': { selector?: string; clip?: string; panel?: string; label?: string; description?: string; duration_ms?: number; scroll?: boolean; clear?: boolean }

  // System domain — environment doctor + consented tool fetch.
  'system.mcp_test': Record<string, never>
  'system.doctor': { refresh?: boolean }
  // Manual ffmpeg override (Change-ffmpeg control); path:null clears → automatic.
  'system.set_ffmpeg': { path?: string | null }
  'system.set_stt_model': { model?: string; language?: string; clear?: boolean; rationale?: string }
  'system.fetch_tool': { tool: 'ffmpeg'; rationale?: string }
  // Provision the Python perception sidecar (uv + runtime + torch + model).
  // warm_model pulls the model in the same job so first transcription is warm.
  // Long-running (several minutes) — returns {job_id}, polled via jobs.status.
  'system.setup_perception': { warm_model?: boolean; rationale?: string }
  // AI background-removal install (ffmpeg pattern): no path = one-click download
  // of the 14 MB RVM model (returns {job_id}); path = browse-to-existing (sync).
  'system.setup_matte': { model?: 'rvm' | 'matanyone'; path?: string; accept_noncommercial?: boolean; rationale?: string }
}

export type VerbName = keyof VerbArgs
type VerbResultPayload<N extends VerbName> = N extends keyof VerbResults ? VerbResults[N] : unknown

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

/**
 * Call one verb. Never throws on verb-level failure — inspect `ok`/`error`
 * (the envelope is the contract). Throws only on transport failure
 * (network down / non-JSON response), which callers surface as disconnect.
 */
export async function callVerb<N extends VerbName>(
  name: N,
  args: VerbArgs[N],
): Promise<VerbResult<VerbResultPayload<N>>> {
  const res = await fetch(`${API_BASE}/api/verb/${name}`, {
    method: 'POST',
    // x-cut-actor: this client is the human's working surface — ops from UI
    // gestures must be attributed HUMAN in the op log.
    // Agents calling REST directly omit the header → agent/rest default.
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:ui' },
    body: JSON.stringify(args ?? {}),
  })
  return (await res.json()) as VerbResult<VerbResultPayload<N>>
}

/** Convenience: fetch full project state (the panels' refresh primitive). */
export async function fetchProject(): Promise<Project | null> {
  const r = await callVerb('project.state', {})
  return r.ok ? r.result ?? null : null
}
