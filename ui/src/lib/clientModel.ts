// clientModel.ts — shared project/media model types for the typed verb client.
//
// Role: pure TypeScript model surface mirrored from cut-core and verb results.
// Keep this free of transport/fetch code so UI helpers can import model types
// without depending on the verb client runtime.

import type { MotionClipLink } from './motionLinkModel'
export type { MotionClipLink } from './motionLinkModel'

// ---------------------------------------------------------------------------
// Shared model types (mirror cut-core; ms everywhere)
// ---------------------------------------------------------------------------

/** The supported color-management spaces (project.color working/output +
 *  edit.color_space clip input). Mirror of cut-core ColorSpace (serialized
 *  lowercase). rec709 is the default working+output space. */
export type ColorSpace = 'rec709' | 'rec2020' | 'srgb' | 'linear'

/** Durable project-level constraints used by verify.brand and render.bundle. */
export interface BrandKit {
  fonts?: string[]
  colors?: string[]
  position?: 'bottom' | 'top' | 'center'
  min_size?: number
  max_size?: number
  aspect?: string
}

export interface ProjectSettings {
  width: number
  height: number
  fps: number
  audio_rate: number
  /** Color management (project.color): the WORKING space the renderer
   *  composites/grades in + the OUTPUT space the delivered file is tagged in.
   *  Engine serde-skips this whole field when it is the default rec709/rec709,
   *  so it is OPTIONAL in JSON — a missing `color` means rec709/rec709. */
  color?: { working: ColorSpace; output: ColorSpace }
}

export interface Asset {
  path: string
  hash: string
  probe?: unknown
  transcript?: string
  perception?: string
  proxy?: string
  /** Relative path to the timeline thumbnail strip (filmstrip/<id>.jpg), served
   *  at /filmstrip/<id>.jpg. The timeline clip slices it by source range. */
  filmstrip?: string
}

export interface TranscriptIgnore {
  asset: string
  word_range: [number, number]
}

export type TrackKind = 'video' | 'audio' | 'caption'

/** Overlay geometry + opacity on a clip (edit.transform) — normalized to the
 * project frame: x/y = top-left as a fraction of frame width/height, scale =
 * overlay width as a fraction of frame width, opacity = 0..1 alpha. (0,0,1,1) =
 * identity/full-frame, fully opaque. Read by the renderer for clips on OVERLAY
 * video tracks (track order = stacking). opacity optional for back-compat with
 * projects saved before the field (a missing opacity = fully opaque). */
export interface ClipTransform {
  x: number
  y: number
  scale: number
  opacity?: number
}

/** True when a transform changes nothing (mirror of cut-core is_identity). */
export function isIdentityTransform(t: ClipTransform | null | undefined): boolean {
  return !t || (t.x === 0 && t.y === 0 && t.scale === 1 && (t.opacity ?? 1) === 1)
}

/** Per-clip SOURCE crop rectangle (edit.crop storage) — mirror of
 *  cut-core::ClipCrop. SOURCE PIXELS (the rect to KEEP), not normalized; crop
 *  happens before conform/transform. Engine serde-skips it when identity (None),
 *  so it is OPTIONAL in JSON; the Layer drawer reads it to seed the crop sliders. */
export interface ClipCrop {
  x: number
  y: number
  w: number
  h: number
}

/** A timed gain reduction on an audio track (edit.duck storage). range_ms is
 * ABSOLUTE timeline time of the full-depth plateau; db is negative; attack_ms
 * is the linear ramp on EACH side. Windowed gain, not a live sidechain. */
export interface GainWindow {
  range_ms: [number, number]
  db: number
  attack_ms: number
}

/** Per-clip linear fade (edit.fade storage). CLIP-LOCAL durations — they
 * travel with the clip through ripples. in_ms ramps from silence/black
 * (alpha on overlay tracks) at the clip start, out_ms to it at the end. */
export interface ClipFade {
  in_ms: number
  out_ms: number
  kind: 'audio' | 'video' | 'both'
}

/** Per-clip color grade (edit.grade storage) — mirror of cut-core::ClipGrade.
 * ffmpeg eq params: 1.0/0.0 are identity. temperature_k/lut are optional.
 * Engine serde-skips the whole field when the grade is identity (None), so it
 * is OPTIONAL in JSON; the Grade drawer reads it to seed its sliders. */
export interface ClipGrade {
  contrast: number
  brightness: number
  saturation: number
  gamma: number
  temperature_k?: number | null
  lut?: string | null
}

/** One geometric POWER WINDOW on a clip (edit.grade_window storage) — mirror of
 *  cut-core::GradeWindow. A region of the frame (`window`) plus the ClipGrade
 *  applied ONLY inside it (`grade`). Windows STACK on a clip; the engine
 *  serde-skips the whole grade_windows vec when empty, so it is OPTIONAL in JSON.
 *  Read by the Inspector to list a clip's windows. */
export interface ClipGradeWindow {
  window: {
    shape: 'rect' | 'ellipse' | 'polygon'
    /** Normalized points (fractions of frame W/H, 0..1). rect: two opposite
     *  corners [[x0,y0],[x1,y1]]; ellipse: [[cx,cy],[rx,ry]]; polygon: vertices. */
    points: [number, number][]
    feather?: number
    invert?: boolean
  }
  grade: ClipGrade
}

/** One per-clip visual effect (edit.effect storage) — mirror of cut-core
 *  ClipEffect, tagged by `type`. vignette/sharpen/grain use `amount`, blur uses
 *  `radius`; chroma_key (overlay clips only) uses color/similarity/blend. */
export type ClipEffect =
  | { type: 'vignette'; amount?: number }
  | { type: 'sharpen'; amount?: number }
  | { type: 'blur'; radius?: number }
  | { type: 'grain'; amount?: number }
  | { type: 'chroma_key'; color: string; similarity?: number; blend?: number }
  | { type: 'denoise'; amount?: number } // AUDIO effect (audio-track clips only)
  | { type: 'compressor'; amount?: number } // AUDIO dynamics compressor (audio-track clips)
  | { type: 'gate'; amount?: number } // AUDIO noise gate — kills room tone between phrases (audio-track clips)
  | { type: 'mirror' } // horizontal flip (un-mirror webcam)
  | { type: 'flip' } // vertical flip
  | { type: 'hue_shift'; degrees?: number }
  | { type: 'rgb_split'; amount?: number } // chromatic aberration / glitch
  | { type: 'pixelize'; size?: number } // mosaic / retro
  | { type: 'sepia' } // vintage tone
  | { type: 'auto_color'; amount?: number } // one-click auto contrast + white balance
  | { type: 'vhs'; amount?: number } // retro-tape preset chain (chroma shift + grain + blur)
  | { type: 'posterize'; levels?: number } // banded / poster look
  | { type: 'invert' } // negative / inverted colours
  | { type: 'emboss' } // relief / engraved look

/** Freeze-frame storage (edit.freeze): hold the source frame at `at_ms` (offset
 *  into the clip's visible range) for the clip's whole slot. */
export interface ClipFreeze {
  at_ms: number
}

/** One end-state of a Ken Burns animation (edit.animate): zoom >= 1 (1 = whole
 *  frame), x/y = normalized focal centre (0..1; 0.5,0.5 = centre). */
export interface AnimState {
  zoom: number
  x: number
  y: number
}

/** Ken Burns pan/zoom animation (edit.animate storage): linear from→to. */
export interface ClipAnimation {
  from: AnimState
  to: AnimState
}

/** One peaking (bell) band of a parametric EQ (edit.eq) — mirror of
 *  cut-core::EqBand. Constant-Q `equalizer` at `freq_hz`, `gain_db` boost/cut,
 *  bandwidth from `q` (higher = narrower; default 1.0). */
export interface EqBand {
  freq_hz: number
  gain_db: number
  q?: number
}

/** Parametric audio EQ (edit.eq storage) — mirror of cut-core::ClipEq. High-pass
 *  (low-cut) + peaking bands + low-pass (high-cut) on the clip audio. */
export interface ClipEq {
  high_pass_hz?: number | null
  low_pass_hz?: number | null
  bands?: EqBand[]
}

/** Animatable parameter names (edit.keyframe). scale = animated zoom (multiplier,
 *  1=native, clamped [1,10]) — the eased multi-point generalization of edit.animate. */
export type KfParam = 'opacity' | 'volume' | 'pos_x' | 'pos_y' | 'scale'

/** Keyframe interpolation (edit.keyframe). linear/hold OR a Penner ease_* curve
 *  (quad/cubic/expo/back/elastic/bounce × in/out/in_out) — eased motion reads
 *  professional rather than mechanical. Mirror of cut-core::KfInterp. */
export type KfInterp =
  | 'linear'
  | 'hold'
  | 'ease_in_quad'
  | 'ease_out_quad'
  | 'ease_in_out_quad'
  | 'ease_in_cubic'
  | 'ease_out_cubic'
  | 'ease_in_out_cubic'
  | 'ease_in_expo'
  | 'ease_out_expo'
  | 'ease_in_out_expo'
  | 'ease_in_back'
  | 'ease_out_back'
  | 'ease_in_out_back'
  | 'ease_in_elastic'
  | 'ease_out_elastic'
  | 'ease_in_out_elastic'
  | 'ease_in_bounce'
  | 'ease_out_bounce'
  | 'ease_in_out_bounce'

/** One keyframe track (edit.keyframe storage) — one parameter animated over the
 *  clip via control points. Mirror of cut-core::Keyframe. */
export interface Keyframe {
  param: KfParam
  points: { t_ms: number; value: number }[]
  interp?: KfInterp
}

/** Untagged clip union — same shapes as cut-core::Clip (timeline/op-log contract).
 * xfade_in_ms (`edit.crossfade`): crossfade-IN length on a MEDIA clip — the
 * dissolve overlap taken from the PREVIOUS clip's tail + this clip's head when
 * > 0. Stored on the RIGHT clip of the pair (the clip whose start is the cut).
 * Engine serde-skips it when 0 (older logs round-trip), so it is OPTIONAL in
 * JSON; the timeline renders an overlap wedge at the seam when present.
 * grade (edit.grade): per-clip color grade; serde-skipped when identity. */
export type Clip =
  | { kind: 'gap'; duration_ms: number }
  | { id: string; asset: string; src_in_ms: number; src_out_ms: number; effects?: unknown[]; gain_db?: number; transform?: ClipTransform; crop?: ClipCrop | null; fade?: ClipFade; xfade_in_ms?: number; speed?: number; grade?: ClipGrade | null; reverse?: boolean; freeze?: ClipFreeze | null; animation?: ClipAnimation | null; eq?: ClipEq | null; keyframes?: Keyframe[];
      /** Non-destructive mute ranges (edit.mute_range / transcript.mute_words),
       *  SOURCE-asset ms [start,end) — engine serde-skips when empty. Drives the
       *  Transcript panel's muted-word styling. */
      mute_ranges?: Array<[number, number]>;
      /** Color management (edit.color_space): the clip's INPUT color-space tag —
       *  the source footage's space, converted INTO the project working space
       *  before grade/effects. Engine serde-skips when None (untagged → assumed
       *  already in the working space). Read by the Inspector input-space selector. */
      input_color_space?: ColorSpace;
      /** Layered grade (edit.grade_stack): an ordered node-stack of grade layers
       *  (serial grading nodes). Authoritative over the single `grade` when
       *  non-empty. Engine serde-skips when empty. Read by the Inspector stack editor. */
      grade_stack?: ClipGrade[];
      /** Power windows (edit.grade_window): region-scoped grades on this clip.
       *  Engine serde-skips when empty. Read by the Inspector window list. */
      grade_windows?: ClipGradeWindow[];
      /** TRANSIENT (project.state only, not persisted): the current editable text
       *  of a title overlay clip on a `title*` track — recovered from the op-log
       *  so the Inspector can seed the in-place title editor (title.update). Only
       *  present on title.add-created clips; kinetic-caption titles omit it. */
      title_text?: string;
      /** TRANSIENT (project.state only, not persisted): the current editable props
       *  of a SHAPE overlay clip on a `title*` track (shapes share the title tracks)
       *  — recovered from the op-log so the Inspector can seed the in-place shape
       *  editor (shape.update). Only present on edit.add_shape-created clips; this is
       *  the marker that routes a shape clip to the shape editor and a title clip
       *  (which carries `title_text` instead) to the title editor. */
      shape_kind?: string; shape_label?: string; shape_color?: string;
      /** TRANSIENT (project.state): replay-backed Motion source/render identity. */
      motion_link?: MotionClipLink }
  | { id: string; text: string; style_ref?: string; range_ms: [number, number] }

/** Duration a constant-speed media clip occupies on the timeline. Mirrors
 * cut-core Clip::timeline_duration_ms for the non-ramped clips the UI can
 * keyframe. */
export function mediaClipTimelineDurationMs(clip: {
  src_in_ms: number
  src_out_ms: number
  speed?: number
}): number {
  const sourceMs = Math.max(0, clip.src_out_ms - clip.src_in_ms)
  const speed = typeof clip.speed === 'number' && Number.isFinite(clip.speed) && clip.speed > 0
    ? clip.speed
    : 1
  return speed === 1 ? sourceMs : Math.round(sourceMs / speed)
}

export interface Track {
  id: string
  kind: TrackKind
  clips: Clip[]
  gain_db?: number
  /** Timed duck windows (edit.duck); omitted in JSON when empty. */
  gain_windows?: GainWindow[]
  /** LAYER blend mode (edit.blend) for an OVERLAY video track — how this whole
   *  track composites onto everything below it. Engine stores Option<String>:
   *  absent/null = 'normal' (default alpha-over). Mirrors core Track.blend_mode. */
  blend_mode?: string
  /** VISUAL track visibility (edit.track_visible). False hides video/caption
   *  output from preview/export while preserving clips. Audio uses edit.mute.
   *  Absent = true. Mirrors core Track.visible. */
  visible?: boolean
  /** Persisted timeline edit lock (edit.track_lock). True blocks drag/trim/drop
   *  gestures in the human timeline UI. Absent = false. Mirrors core Track.locked. */
  locked?: boolean
  /** NON-DESTRUCTIVE AUDIO-track MUTE flag (edit.mute). True = silenced in the
   *  audio mix. The track's gain_db is independent (never overwritten by mute). Server
   *  truth — the Mixer/Timeline derive their Mute button state from this. Absent =
   *  false (omitted in JSON when false). Mirrors core Track.muted. */
  muted?: boolean
  /** NON-DESTRUCTIVE AUDIO-track SOLO flag (edit.solo). When any audio track has
   *  solo=true, only soloed audio tracks are audible. Absent = false. */
  solo?: boolean
  /** NON-DESTRUCTIVE AUDIO-track stereo pan/balance (edit.pan), −1 left …
   *  0 center … +1 right. Absent = center. Mirrors core Track.pan. */
  pan?: number
}

export interface Marker {
  id: string
  at_ms: number
  label: string
  note?: string
  /** Display color — one of MARKER_COLOR_SWATCH's keys; absent = default. */
  color?: MarkerColor
}

/** The closed marker-color set (mirrors core MARKER_COLORS). */
export type MarkerColor = 'red' | 'orange' | 'yellow' | 'green' | 'teal' | 'blue' | 'purple' | 'pink'

/** Marker color name → CSS swatch. One shared map so the ruler triangle and
 * the context-menu swatches can never disagree. */
export const MARKER_COLOR_SWATCH: Record<MarkerColor, string> = {
  red: '#e5484d',
  orange: '#f76b15',
  yellow: '#ffc53d',
  green: '#46a758',
  teal: '#12a594',
  blue: '#3e63dd',
  purple: '#8e4ec6',
  pink: '#d6409f',
}

export interface CaptionStyle {
  font: string
  size: number
  color: string
  bg?: string
  pos?: 'bottom' | 'top' | 'center'
  [extra: string]: unknown
}

export interface Checkpoint {
  id: string
  name: string
  sequence_id?: string
  at_op: string
  ts: string
}

/** A drafted change set the agent proposed for a comment (comment.draft). */
export interface CommentDraft {
  verbs: { verb: string; args: Record<string, unknown> }[]
  rationale?: string | null
  confidence?: number | null
  validation?: { ok: boolean; verb_count: number; invalid: { verb: string; why: string }[] }
  backend?: { provider?: string; model?: string } | null
  ts?: string
}

export interface CommentAnchor {
  track_id: string
  clip_id: string
  offset_ms: number
}

export interface CommentReviewSource {
  source_op_id: string
  render_id: string
  render_hash: string
}

/** A timecoded review comment. Lives in project.comments. */
export interface Comment {
  id: string
  at_ms: number
  end_ms?: number
  anchor?: CommentAnchor | null
  text: string
  author: string
  status: 'open' | 'addressed' | 'dismissed'
  ts: string
  review_source?: CommentReviewSource | null
  draft?: CommentDraft | null
}

export interface Sequence {
  id: string
  name: string
  settings: ProjectSettings
  tracks: Track[]
  markers: Marker[]
  caption_styles: Record<string, CaptionStyle>
  comments?: Comment[]
  adjustments?: unknown[]
  nests?: unknown[]
  transcript_ignores?: TranscriptIgnore[]
}

export interface SequenceSummary {
  id: string
  name: string
  active: boolean
  duration_ms: number
  clip_count: number
  settings: ProjectSettings
}

export interface Project {
  schema: string
  name: string
  settings: ProjectSettings
  assets: Record<string, Asset>
  tracks: Track[]
  markers: Marker[]
  caption_styles: Record<string, CaptionStyle>
  brand?: BrandKit
  checkpoints: Checkpoint[]
  active_sequence?: string
  sequences?: Sequence[]
  comments?: Comment[]
  transcript_ignores?: TranscriptIgnore[]
}

export interface OpRecord {
  op_id: string
  ts: string
  actor: { kind: 'agent' | 'human' | 'system'; name: string; via: string }
  verb: string
  args: unknown
  rationale?: string
  effects?: Array<{ track?: string; [k: string]: unknown }>
  inverse?: { verb: string; args: unknown }
  status: 'applied' | 'rejected'
}

export interface CheckResult {
  name: string
  pass: boolean
  details: unknown
  evidence: unknown
}

/** One finding from the perceptual judge (verify.judge). */
export interface JudgeIssue {
  at_ms?: number
  end_ms?: number
  kind?: string
  severity?: string
  evidence?: string
  suggested_fix?: string
  [extra: string]: unknown
}

/** The judge's structured review (judge.review — null when not_run/error). */
export interface JudgeReview {
  verdict: 'pass' | 'fail' | 'needs_review'
  confidence?: number
  issues?: JudgeIssue[]
  summary?: string
  cannot_assess?: string[]
  [extra: string]: unknown
}

/** RenderReceipt.judge — schema shellx-cut/judge-review/1 (verify.judge).
 * Honest statuses: completed | not_run (no adapter/CLI — never a fake pass)
 * | error (adapter attempted and failed). Loosely typed: the envelope grows
 * adapter-specific fields (post_filter, bundle_dir, …) we render generically. */
export interface JudgeEnvelope {
  schema?: string
  status?: 'completed' | 'not_run' | 'error' | string
  review?: JudgeReview | null
  not_run_reason?: string | null
  reason?: string
  backend?: {
    name?: string
    provider?: string
    model?: string
    frames_sent?: number
    watched?: boolean
    listened?: boolean
    [extra: string]: unknown
  }
  cli?: {
    model?: string
    accounting_cost_usd?: number
    duration_ms?: number
    [extra: string]: unknown
  }
  [extra: string]: unknown
}

/** A machine-actionable repair derived from one failing check (the agent-receipt
 *  contract at schema/receipts.schema.json#/$defs/FixAction). The Receipted-Autopilot
 *  consumes these; the UI uses them for the clean "N to fix" summary. */
export interface FixAction {
  check: string
  fix_verb: string
  fix_args: Record<string, unknown>
  targets: { clip_id?: string; at_ms?: number; op_id?: string }[]
  measured: unknown
  rationale: string
  auto_fixable: boolean
}

export interface RenderReceipt {
  render_id: string
  ts: string
  output_path: string
  output_hash: string
  duration_ms: number
  preset: string
  at_op: string
  checks: CheckResult[]
  pass: boolean
  judge?: JudgeEnvelope | null
  /** One entry per recognised FAILING check (empty when all pass). Drives the
   *  status-bar "what changed" summary + the autopilot self-fix loop. */
  fix_actions?: FixAction[]
}

export interface WordSpan {
  idx: number
  word: string
  start_ms: number
  end_ms: number
  confidence?: number
  /** Canonical speaker label ('S1'..'Sn', arrival order) — present only after
   *  media.diarize has labeled this asset (omitted otherwise). */
  speaker?: string
}

export interface Transcript {
  asset: string
  model: string
  language?: string
  words: WordSpan[]
}

/** One word of the EDL-aware (timeline-mapped) transcript — transcript.timeline.
 *  Each word carries the owning clip + track and its position on the timeline. */
export interface TimelineWord {
  clip_id: string | null
  track: string
  track_kind: 'video' | 'audio'
  asset: string
  word_index: number
  word: string
  src_start_ms: number
  src_end_ms: number
  timeline_start_ms: number
  timeline_end_ms: number
  /** Canonical speaker label ('S1'..'Sn') — present only after media.diarize. */
  speaker?: string
}

/** transcript.timeline result: words mapped to the timeline through the EDL. */
export interface TimelineTranscript {
  clip: string | null
  track: string | null
  word_count: number
  entries: TimelineWord[]
}

export interface CutError {
  code: string
  message: string
  clip_id?: string
  at_ms?: number
  cause: string
  /** What the agent/user should do next, when a clear step exists. Mirrors
   * cut-core CutError.suggested_action (serde-skipped when None). The rebase
   * guardrail sets it; the UI renders it VERBATIM. */
  suggested_action?: string
}

export type { JobRecord } from './jobModel'

/** media.waveform result — per-bucket abs-max audio amplitude (0..1), left→
 * right in time across `[0, source_ms]` of the asset. `bucket_count` ==
 * `peaks.length`; `source_ms` is the audio span the peaks cover (from the
 * asset probe's duration). The timeline maps a clip's SOURCE range onto this
 * span to slice the peaks it draws. Display-only — never a timeline mutation. */
export interface Waveform {
  asset: string
  bucket_count: number
  peaks: number[]
  source_ms: number
  sample_rate: number
}
