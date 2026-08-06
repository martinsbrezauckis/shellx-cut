// panels/Timeline/layout.ts — pure timeline math (no React, no DOM).
// Role: the single time↔pixel transform + clip layout + ruler tick ladder +
// snapping resolution used by the Timeline panel. Timeline layout uses integer-ms time
// base, BASE_PPS=50 transform, zoom-invariant 10-screen-px snap threshold,
// two-ladder ruler intervals with divisibility repair.
// Callers: panels/Timeline/index.tsx. Dependencies: lib/client types only.

import { isIdentityTransform, type Clip, type ClipFade, type ClipGrade, type ClipTransform, type Marker, type MotionClipLink, type Project, type Track, type TrackKind } from '../../lib/client'

// ---------------------------------------------------------------------------
// Constants (adjust only with a reason)
// ---------------------------------------------------------------------------

/** Pixels per second at zoom 1.0 (timeline behavior contract). */
export const BASE_PPS = 50
/** Max zoom (timeline behavior contract: zoom ∈ [computed min … 100]). */
export const MAX_ZOOM = 100
/** Drag-vs-click threshold, px in x OR y (timeline behavior contract). */
export const DRAG_THRESHOLD_PX = 5
/** Click-to-seek max press duration, ms (timeline behavior contract). */
export const CLICK_MAX_MS = 500
/** Snap engages within this many SCREEN px at every zoom (timeline behavior contract). */
export const SNAP_THRESHOLD_PX = 10
/** Wheel: per-event delta cap before exp factor (timeline behavior contract). */
export const WHEEL_DELTA_CAP = 30
/** Wheel: horizontal pan step clamp px/event (timeline behavior contract). */
export const WHEEL_PAN_CLAMP_PX = 40
/** Keyboard/button zoom step factor (timeline behavior contract). */
export const ZOOM_KEY_FACTOR = 1.7
/** Edge auto-scroll: activation distance / max speed (timeline behavior contract). */
export const EDGE_SCROLL_ZONE_PX = 100
export const EDGE_SCROLL_MAX_PX = 15

/** Track lane heights by kind. */
export const TRACK_HEIGHT: Record<string, number> = { video: 56, audio: 40, caption: 32 }
/** Ruler height + left track-header rail width. */
export const RULER_H = 28
export const RAIL_W = 176

// ---------------------------------------------------------------------------
// Time ↔ pixel transform (timeline behavior contract — ONE transform, used everywhere)
// ---------------------------------------------------------------------------

/** ms → px at a zoom level. */
export function msToPx(ms: number, zoom: number): number {
  return (ms / 1000) * BASE_PPS * zoom
}

/** px → ms at a zoom level (rounded — integer-ms time base, no float drift). */
export function pxToMs(px: number, zoom: number): number {
  return Math.round((px / (BASE_PPS * zoom)) * 1000)
}

/** Snap a px value to the device-pixel grid for crisp 1-2px lines. */
export function snapToDeviceGrid(px: number): number {
  const dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1
  return Math.round(px * dpr) / dpr
}

/** Left for a centered vertical line of `width` at time-px `px`. */
export function centeredLineLeft(px: number, width: number): number {
  return snapToDeviceGrid(px) - width / 2
}

/** Min zoom: whole project fits with margin — project ≥ ~25% of viewport. */
export function minZoomFor(durationMs: number, viewportW: number): number {
  if (durationMs <= 0 || viewportW <= 0) return 0.05
  const fit = (viewportW * 0.25) / ((durationMs / 1000) * BASE_PPS)
  return Math.min(1, Math.max(0.002, fit))
}

// ---------------------------------------------------------------------------
// Clip layout — schema clips → absolutely-positioned lane items
// ---------------------------------------------------------------------------

/** One laid-out item on a lane (clip OR rendered gap), in timeline ms.
 *
 * TWO TIME BASES (the coordinate contract — getting this wrong dispatches
 * verbs at positions the engine rejects):
 *  - LAID / render time (`startMs`): where content draws and plays. A
 *    crossfade pulls the right clip back into the left clip's tail, so laid
 *    positions REWIND by each upstream overlap. The playhead, pointer math,
 *    px transforms, and `edit.split_edit` (EDL-keyed) live here.
 *  - EDITORIAL time (`editorialStartMs`): the engine's cumulative-track
 *    cursor — the plain sum of clip durations, crossfade-INDEPENDENT. Every
 *    cumulative-track verb keys on it: edit.split / edit.crossfade /
 *    edit.roll / edit.ripple_delete range_ms / edit.move / edit.insert /
 *    edit.fit_to_fill (verified in app/core/src/edit.rs + trim_edit.rs).
 * The two are equal until the first crossfade upstream on the track; after
 * that, dispatching a laid coordinate targets a nonexistent boundary
 * (engine not_found — the dual-surface harness caught exactly this). */
export interface LaidItem {
  /** Clip id; gaps get a synthetic stable id `gap:<track>:<index>`. */
  id: string
  kind: 'video' | 'audio' | 'caption' | 'gap'
  trackId: string
  /** LAID/render start — drawing + pointer space (see time-base note above). */
  startMs: number
  durMs: number
  /** EDITORIAL start — the engine's clip-duration-sum cursor (see note above).
   * Equals startMs plus the total crossfade overlap upstream on this track.
   * Captions carry an absolute range, so theirs equals startMs. Dispatch
   * cumulative-track verb positions/ranges from THIS, never from startMs. */
  editorialStartMs: number
  /** Display label: asset id for AV, text for captions. */
  label: string
  /** AV clips only — source range (for trim math). */
  srcInMs?: number
  srcOutMs?: number
  /** AV clips only — asset reference. */
  asset?: string
  /** Video-track clip backed by a STILL IMAGE asset (probe kind=image) —
   * renders with the photo tint/icon, distinct from motion video. */
  isImage?: boolean
  /** Non-identity overlay geometry (edit.transform) — drives the PiP badge. */
  transform?: ClipTransform
  /** Active linear fade (edit.fade) — drives the corner triangles. */
  fade?: ClipFade
  /** Crossfade-IN overlap on the RIGHT clip of a seam (edit.crossfade,
   * xfade_in_ms > 0). Drives the overlap wedge drawn at the clip's start —
   * the LEFT neighbour's tail and this head overlap by this many ms, so the
   * realized timeline is SHORTER than the sum of clip durations. */
  xfadeInMs?: number
  /** Playback speed factor (edit.speed), carried only when != 1.0. Drives the
   * speed badge AND has already been folded into durMs (timeline duration =
   * source span / speed), so the clip is drawn at its real timeline width. */
  speed?: number
  /** Reverse playback changes which source edge a timeline trim adjusts. */
  reverse?: boolean
  /** True when the clip carries a non-identity color grade (edit.grade). The
   * engine serde-skips an identity grade, so presence == a real grade. Drives
   * the timeline grade badge so users can see which clips are graded. */
  graded?: boolean
  /** The full color grade (edit.grade) when present — the live preview's
   * composite stage reads it to apply an APPROXIMATE CSS filter to the layer
   * (the timeline only needs `graded` for its badge). serde-skipped when
   * identity, so presence == a real grade. */
  grade?: ClipGrade | null
  /** Durable ShellX Motion source/render identity projected by project.state. */
  motionLink?: MotionClipLink
}

type GapClip = Extract<Clip, { kind: 'gap' }>
type CaptionClip = Extract<Clip, { text: string }>
type MediaClip = Extract<Clip, { asset: string }>

function gapClipFrom(c: Clip): GapClip | null {
  return 'duration_ms' in c && c.kind === 'gap' ? c : null
}

function captionClipFrom(c: Clip): CaptionClip | null {
  return 'text' in c ? c : null
}

function mediaClipFrom(c: Clip): MediaClip | null {
  return 'asset' in c ? c : null
}

/** Asset ids whose probe reported kind=image (still images). The probe is
 * stored verbatim on the asset (media.probe result) — kind lives at its root. */
export function imageAssetIds(project: Project | null): Set<string> {
  const out = new Set<string>()
  if (!project) return out
  for (const [id, asset] of Object.entries(project.assets ?? {})) {
    const kind = (asset.probe as { kind?: unknown } | undefined)?.kind
    if (kind === 'image') out.add(id)
  }
  return out
}

/**
 * Lay out one track. AV tracks: cumulative position (timeline/op-log contract — clips ordered,
 * non-overlapping, gap clips are content). Caption tracks: absolute range_ms.
 * `imageAssets` (optional, from imageAssetIds) flags still-image clips for
 * distinct rendering; omitting it only loses the photo tint, never geometry.
 */
export function layoutTrack(track: Track, imageAssets?: Set<string>): LaidItem[] {
  const items: LaidItem[] = []
  // LAID cursor (rewinds by crossfade overlaps) + EDITORIAL cursor (plain
  // clip-duration sum, the engine's cumulative-track clock) — see LaidItem.
  let cursor = 0
  let edCursor = 0
  track.clips.forEach((c, i) => {
    const gap = gapClipFrom(c)
    const caption = captionClipFrom(c)
    const clip = mediaClipFrom(c)
    if (gap) {
      items.push({
        id: `gap:${track.id}:${i}`,
        kind: 'gap',
        trackId: track.id,
        startMs: cursor,
        editorialStartMs: edCursor,
        durMs: gap.duration_ms,
        label: '',
      })
      cursor += gap.duration_ms
      edCursor += gap.duration_ms
    } else if (caption) {
      items.push({
        id: caption.id,
        kind: 'caption',
        trackId: track.id,
        startMs: caption.range_ms[0],
        // Captions are ABSOLUTE-range clips (no cumulative cursor): their
        // editorial position IS their laid position.
        editorialStartMs: caption.range_ms[0],
        durMs: caption.range_ms[1] - caption.range_ms[0],
        label: caption.text,
      })
    } else if (clip) {
      // Speed retime (edit.speed): the clip occupies source_span / speed ms of
      // timeline — mirror the engine's Clip::timeline_duration_ms (round-divide)
      // EXACTLY so the UI width + every later clip's position match the render.
      const rawDur = clip.src_out_ms - clip.src_in_ms
      const speed = clip.speed && clip.speed > 0 ? clip.speed : 1
      const dur = speed !== 1 ? Math.round(rawDur / speed) : rawDur
      // Crossfade overlap (edit.crossfade): the engine takes the overlap from
      // the LEFT neighbour's tail + this clip's head, so the realized timeline
      // SHORTENS by xfade_in_ms across the seam. Mirror that here — back the
      // cursor up by the overlap so this clip starts INSIDE its predecessor's
      // tail, exactly as the EDL composes it (clamped to the predecessor's
      // already-laid duration so a too-long overlap never rewinds past 0).
      const rawX = clip.xfade_in_ms ?? 0
      const prev = items[items.length - 1]
      const prevDur = prev && (prev.kind === 'video' || prev.kind === 'audio') ? prev.durMs : 0
      const xfade = rawX > 0 ? Math.min(rawX, prevDur, dur) : 0
      const startMs = cursor - xfade
      items.push({
        id: clip.id,
        kind: track.kind === 'audio' ? 'audio' : 'video',
        trackId: track.id,
        startMs,
        // The editorial cursor never rewinds — a crossfade shortens the LAID
        // timeline only; the engine's clip-duration-sum clock is untouched.
        editorialStartMs: edCursor,
        durMs: dur,
        label: clip.asset,
        srcInMs: clip.src_in_ms,
        srcOutMs: clip.src_out_ms,
        asset: clip.asset,
        isImage: imageAssets?.has(clip.asset) || undefined,
        // Identity transforms are noise — only carry geometry that does work.
        transform: isIdentityTransform(clip.transform) ? undefined : clip.transform,
        // 0/0 fades are cleared fades — carry only ramps that render.
        fade: clip.fade && (clip.fade.in_ms > 0 || clip.fade.out_ms > 0) ? clip.fade : undefined,
        // Carry the overlap so the panel can draw the seam wedge.
        xfadeInMs: xfade > 0 ? xfade : undefined,
        // Carry speed (only when retimed) for the badge; durMs already reflects it.
        speed: speed !== 1 ? speed : undefined,
        reverse: clip.reverse || undefined,
        // A non-identity grade is serde-present on the clip → flag for the badge.
        graded: clip.grade ? true : undefined,
        // Carry the full grade so the live composite stage can CSS-approximate it.
        grade: clip.grade ?? undefined,
        motionLink: clip.motion_link,
      })
      cursor = startMs + dur
      edCursor += dur
    }
  })
  return items
}

/** A timeline position resolved back to an asset's SOURCE time. */
export interface SourceAt {
  asset: string
  /** Source-media ms inside that asset (NOT timeline ms). */
  srcMs: number
}

export interface SourceTimelineOccurrence {
  clipId: string
  trackId: string
  atMs: number
}

/**
 * Resolve a source-media instant to every video-timeline occurrence that shows
 * it. Visual-search hits are source-relative; sending `peak_ms` straight to
 * ui.playhead is wrong after a trim, delay, reuse, speed change, or reverse.
 * Variable-speed ramps are deliberately omitted here because the UI model does
 * not yet carry the engine's ramp segments; callers keep Source as the exact
 * fallback instead of presenting an approximate timeline jump.
 */
export function sourceTimelineOccurrences(
  project: Project | null,
  assetId: string,
  sourceMs: number,
): SourceTimelineOccurrence[] {
  if (!project || !Number.isFinite(sourceMs)) return []
  const found: SourceTimelineOccurrence[] = []
  for (const track of project.tracks) {
    if (track.kind !== 'video') continue
    for (const item of layoutTrack(track)) {
      if (item.asset !== assetId || item.srcInMs === undefined || item.srcOutMs === undefined) continue
      if (sourceMs < item.srcInMs || sourceMs >= item.srcOutMs) continue
      const raw = track.clips.find((clip) => 'id' in clip && clip.id === item.id)
      if (!raw || !('asset' in raw)) continue
      if ((raw as { speed_ramp?: unknown }).speed_ramp != null) continue
      const speed = raw.speed && raw.speed > 0 ? raw.speed : 1
      const sourceOffset = raw.reverse
        ? item.srcOutMs - sourceMs
        : sourceMs - item.srcInMs
      const timelineOffset = Math.round(sourceOffset / speed)
      found.push({
        clipId: item.id,
        trackId: track.id,
        atMs: Math.max(item.startMs, Math.min(item.startMs + item.durMs - 1, item.startMs + timelineOffset)),
      })
    }
  }
  return found.sort((a, b) => a.atMs - b.atMs || a.trackId.localeCompare(b.trackId))
}

/**
 * Map a TIMELINE ms back to the source-media ms of the asset playing there, by
 * walking the EDL. After any cut the timeline and source clocks diverge —
 * a clip at timeline `startMs` plays source `[src_in_ms, src_out_ms)`, so the
 * source time at the playhead is `src_in_ms + (timelineMs − startMs)`. Without
 * this walk a transcript/word lookup that treats timelineMs AS source ms drifts
 * by the total removed duration before the playhead.
 *
 * Resolution: prefer the covering VIDEO clip (the speech reference); fall back
 * to the covering AUDIO clip when no video track covers the position (audio-only
 * sections). Returns null over a gap / past the end / with no project.
 */
export function sourceAtPlayhead(project: Project | null, timelineMs: number): SourceAt | null {
  if (!project) return null
  const find = (kind: TrackKind): SourceAt | null => {
    for (const track of project.tracks) {
      if (track.kind !== kind) continue
      for (const it of layoutTrack(track)) {
        if (it.kind === 'gap' || it.kind === 'caption' || !it.asset || it.srcInMs === undefined) continue
        if (timelineMs >= it.startMs && timelineMs < it.startMs + it.durMs) {
          return { asset: it.asset, srcMs: it.srcInMs + (timelineMs - it.startMs) }
        }
      }
    }
    return null
  }
  return find('video') ?? find('audio')
}

/** Timeline content duration = max end across tracks + markers; min 60s. */
export function projectDurationMs(project: Project | null): number {
  if (!project) return 60_000
  let max = 0
  for (const t of project.tracks) {
    for (const it of layoutTrack(t)) max = Math.max(max, it.startMs + it.durMs)
  }
  for (const m of project.markers ?? []) max = Math.max(max, m.at_ms)
  // 60s FLOOR: the timeline ruler always shows at least a 60s editing canvas so there's
  // room to scroll / drop clips past short content. This is the CANVAS width — NOT the
  // playback duration. PLAYBACK must use contentExtentMs() (below), else the preview runs
  // into BLACK past short content up to this floor.
  return Math.max(max, 60_000)
}

/** The PLAYABLE content extent (ms): the end of the last clip / marker, with NO canvas
 * floor. PLAYBACK + the preview scrubber/clock use THIS so they stop at the real end of
 * the content (a short recording, a clip) instead of crawling through black up to the 60s
 * editing floor. Returns 0 for an empty timeline (nothing to play — callers guard
 * with Math.min/comparisons, never divide by it). */
export function contentExtentMs(project: Project | null): number {
  if (!project) return 0
  let max = 0
  for (const t of project.tracks) {
    for (const it of layoutTrack(t)) max = Math.max(max, it.startMs + it.durMs)
  }
  for (const m of project.markers ?? []) max = Math.max(max, m.at_ms)
  return max
}

// ---------------------------------------------------------------------------
// Track geometry + ripple-vs-lift matrix
// ---------------------------------------------------------------------------

/** One laid-out track row with its vertical extent inside the track area
 * (offset BELOW the ruler — top is RULER_H-relative). Drives drop-target
 * resolution for vertical drag-move and the ripple-vs-lift feedback. */
export interface TrackRow {
  id: string
  kind: TrackKind
  /** Top offset from the start of the track area (px, ruler excluded). */
  top: number
  height: number
  /** Stacking index within its kind (0 = base; ≥1 = overlay/extra). */
  kindIndex: number
  /** Visual output flag (video/caption); defaults true for old projects. */
  visible: boolean
  /** Timeline edit guard; locked tracks reject drag/trim/drop gestures. */
  locked: boolean
}

/** Lay out track rows top→bottom with per-kind heights + kind-stacking index.
 * kindIndex mirrors the engine's "first video/audio track = base, later = overlay"
 * rule (edit.insert ripple matrix; edit.move kind-match) used for both the
 * cross-track drop guard and the ripple-vs-lift UI prediction. */
export function trackRows(project: Project | null): TrackRow[] {
  if (!project) return []
  const rows: TrackRow[] = []
  let top = 0
  const kindSeen: Record<string, number> = {}
  for (const t of project.tracks) {
    const height = TRACK_HEIGHT[t.kind] ?? 40
    const kindIndex = kindSeen[t.kind] ?? 0
    kindSeen[t.kind] = kindIndex + 1
    rows.push({ id: t.id, kind: t.kind, top, height, kindIndex, visible: t.visible !== false, locked: !!t.locked })
    top += height + 1 // +1 = the 1px hairline between lanes
  }
  return rows
}

/** Resolve which track a cursor Y (relative to the track-area top, ruler
 * already subtracted) falls on. Returns null above/below the lanes. */
export function trackRowAtY(rows: TrackRow[], yInTrackArea: number): TrackRow | null {
  for (const r of rows) {
    if (yInTrackArea >= r.top && yInTrackArea < r.top + r.height + 1) return r
  }
  return null
}

/**
 * Predict the engine's ripple-vs-lift behavior for a clip dropped onto a track,
 * mirroring the edit.move / insert-ripple matrix WITHOUT contradicting it.
 *
 * edit.move gap-fills the source slot (source never ripples) and splices into
 * the destination. The destination's downstream content shifts right by the
 * clip duration. Whether that shift propagates to SIBLING tracks is the
 * ripple-vs-lift distinction the engine encodes for inserts:
 *  - BASE track (first of its kind, kindIndex 0) → siblings keep AV sync
 *    ("ripple" — the timeline expectation for a base-canvas edit).
 *  - OVERLAY / EXTRA track (kindIndex ≥ 1) → it floats; nothing else moves
 *    ("lift").
 * This is FEEDBACK ONLY — edit.move itself never takes a ripple flag; the UI
 * states the expectation the user should hold, it does not change the verb.
 */
export type RippleMode = 'ripple' | 'lift'
export function rippleModeForTrack(row: TrackRow): RippleMode {
  return row.kindIndex === 0 ? 'ripple' : 'lift'
}

/** Format an honest, verb-shaped rationale for a human timeline gesture, e.g.
 * `user drag: clip c3 +1.20s on v1` or `user drag: clip c6 −0.50s → v2 (lift)`.
 * Signed seconds with 2 decimals; track-change and ripple mode annotated. */
export function dragRationale(
  clipId: string,
  deltaMs: number,
  toTrack: string,
  trackChanged: boolean,
  mode?: RippleMode,
): string {
  const secs = deltaMs / 1000
  const sign = secs >= 0 ? '+' : '−'
  const mag = `${sign}${Math.abs(secs).toFixed(2)}s`
  const dest = trackChanged ? ` → ${toTrack}${mode ? ` (${mode})` : ''}` : ` on ${toTrack}`
  return `user drag: clip ${clipId} ${mag}${dest}`
}

// ---------------------------------------------------------------------------
// Marker classification (audio + capture manifest) — visual distinction
// ---------------------------------------------------------------------------

/** Marker visual class. 'beat' = audio.add_music beat grid (label "beat"),
 * 'capture' = capture-manifest event markers (label "capture:<type>"),
 * 'plain' = a user/agent edit.add_marker. Drives subtle per-class styling
 * (orange markers, with beats/captures visually distinct). */
export type MarkerClass = 'beat' | 'capture' | 'plain'

export function markerClass(m: Marker): MarkerClass {
  if (m.label === 'beat') return 'beat'
  if (m.label.startsWith('capture:')) return 'capture'
  return 'plain'
}

// ---------------------------------------------------------------------------
// Crossfade seam detection — which clip boundaries are an exact clip-to-clip
// cut (edit.crossfade targets). A seam is the END of clip i == START of clip
// i+1 where BOTH are media clips (gaps/captions excluded — the verb refuses
// them). Returns the timeline at_ms (= the boundary) + the two clip ids.
// ---------------------------------------------------------------------------

export interface Seam {
  /** EDITORIAL boundary time of the cut (clip-duration sum) — what the
   * engine's cumulative-track verbs (edit.crossfade, edit.roll) key on.
   * DISPATCH this; never a laid coordinate. After any upstream crossfade the
   * two diverge, and the laid value targets a nonexistent boundary (proven
   * live: UI sent laid 3242, the engine's cut was editorial 3642). */
  atMs: number
  /** LAID/render-time coordinate of the visible boundary (left clip's drawn
   * end) — for DRAWING the handle and playhead-distance math only. */
  laidMs: number
  leftId: string
  rightId: string
  trackId: string
  /** Current crossfade overlap on the right clip (0 = a hard cut). */
  xfadeMs: number
}

/** Adjacent media-clip seams on one track's laid items (media↔media only). */
export function trackSeams(items: LaidItem[]): Seam[] {
  const seams: Seam[] = []
  for (let i = 0; i < items.length - 1; i++) {
    const a = items[i]
    const b = items[i + 1]
    const aMedia = a.kind === 'video' || a.kind === 'audio'
    const bMedia = b.kind === 'video' || b.kind === 'audio'
    if (!aMedia || !bMedia) continue
    // The cut the engine keys on is the LEFT clip's EDITORIAL end
    // (editorialStartMs + durMs — crossfade-independent), NOT its laid end:
    // laid positions rewind by every upstream overlap, so after one crossfade
    // the laid end of any later clip understates the editorial boundary and
    // edit.crossfade there is not_found (the dual-surface harness's live
    // catch). A crossfade ON this seam itself keeps the same editorial at_ms
    // (verified against the live engine: a crossfaded seam re-targets at the
    // ORIGINAL cut). The handle is DRAWN at the laid end (`laidMs`) so it
    // sits on the visible boundary between the two clip bodies.
    seams.push({
      atMs: a.editorialStartMs + a.durMs,
      laidMs: a.startMs + a.durMs,
      leftId: a.id,
      rightId: b.id,
      trackId: b.trackId,
      xfadeMs: b.xfadeInMs ?? 0,
    })
  }
  return seams
}

/**
 * Convert a LAID/render-time position on ONE track's laid items to the
 * EDITORIAL position the engine's cumulative-track verbs key on.
 * - Inside an item: editorial start + the same within-item offset (within-item
 *   deltas are identical in both bases).
 * - Inside a crossfade overlap (two items cover the position): resolves into
 *   the LEFT clip's tail (first covering item in track order).
 * - Past the last item: editorial end + the overshoot.
 * - Empty track / before the first item: identity (the clocks start aligned).
 */
export function laidToEditorialMs(items: LaidItem[], laidMs: number): number {
  for (const it of items) {
    if (laidMs >= it.startMs && laidMs < it.startMs + it.durMs) {
      return it.editorialStartMs + (laidMs - it.startMs)
    }
  }
  const last = items[items.length - 1]
  if (last && laidMs >= last.startMs + last.durMs) {
    return last.editorialStartMs + last.durMs + (laidMs - (last.startMs + last.durMs))
  }
  return laidMs
}

// ---------------------------------------------------------------------------
// Linked A/V resolution + linked split planning
//
// The engine has no stored clip-linkage identity: `edit.insert`/import place a
// video clip and its audio counterpart as two clips, and the server's linked
// ops (edit.trim/edit.move {linked:true} → resolve_linked_media) re-infer the
// pair from the auto-placement shape. edit.split and edit.ripple_delete carry
// NO linked arg, so the UI must dispatch for both halves itself — these
// helpers mirror the engine's linkage criteria exactly so UI-side propagation
// and engine-side linked ops agree on what "linked" means.
// ---------------------------------------------------------------------------

/**
 * Exact linked A/V counterparts of a media item, mirroring the engine's
 * resolve_linked_media (app/server/src/dispatch/edit_tools/linked_move.rs):
 * opposite AV kind, same asset, same source window, same LAID timeline span
 * (the engine matches spans via the EDL, which is laid/render time).
 * Returns ALL matches — callers apply the engine's ambiguity policy
 * (exactly one = linked; several = refuse rather than guess).
 */
export function linkedSiblings(item: LaidItem, allItems: LaidItem[]): LaidItem[] {
  if (item.kind !== 'video' && item.kind !== 'audio') return []
  const opposite = item.kind === 'video' ? 'audio' : 'video'
  return allItems.filter((c) =>
    c.kind === opposite
    && c.asset === item.asset
    && c.srcInMs === item.srcInMs
    && c.srcOutMs === item.srcOutMs
    && c.startMs === item.startMs
    && c.durMs === item.durMs,
  )
}

/** One edit.split dispatch target planned by planLinkedSplit. */
export interface LinkedSplitTarget {
  track: string
  /** EDITORIAL split position on that track (the engine's split cursor). */
  atMs: number
  /** The clip being split there (for rationale/error text). */
  clipId: string
}

export type LinkedSplitPlan =
  | { kind: 'ok'; targets: LinkedSplitTarget[] }
  /** >1 exact counterpart — splitting one guessed half would desync; refuse
   * (the engine's own linked-op ambiguity policy). */
  | { kind: 'ambiguous'; candidates: number }
  /** The counterpart sits on a locked track — splitting only one half would
   * desync the pair; refuse until unlocked (engine guardrail parity). */
  | { kind: 'locked'; trackId: string }

/**
 * Plan a razor/split at LAID position `laidCutMs` inside `anchor` so the cut
 * lands on the anchor AND its exact linked A/V counterpart (NLE convention:
 * linked clips cut together). Positions are converted to EDITORIAL time
 * per-track — the linked pair shares the within-clip offset (equal laid
 * spans), but each track carries its own editorial cursor.
 * The caller guards that laidCutMs is strictly inside the anchor's laid body
 * and dispatches one edit.split per target (shared group_id when 2).
 */
export function planLinkedSplit(
  anchor: LaidItem,
  allItems: LaidItem[],
  laidCutMs: number,
  isTrackLocked: (trackId: string) => boolean,
): LinkedSplitPlan {
  const offset = laidCutMs - anchor.startMs
  const targets: LinkedSplitTarget[] = [
    { track: anchor.trackId, atMs: Math.round(anchor.editorialStartMs + offset), clipId: anchor.id },
  ]
  const sibs = linkedSiblings(anchor, allItems)
  if (sibs.length > 1) return { kind: 'ambiguous', candidates: sibs.length }
  const sib = sibs[0]
  if (sib) {
    if (isTrackLocked(sib.trackId)) return { kind: 'locked', trackId: sib.trackId }
    targets.push({ track: sib.trackId, atMs: Math.round(sib.editorialStartMs + offset), clipId: sib.id })
  }
  return { kind: 'ok', targets }
}

// ---------------------------------------------------------------------------
// Snapping (timeline behavior contract — pixel-constant threshold, self-exclusion)
// ---------------------------------------------------------------------------

export interface SnapResult {
  ms: number
  /** Snap point that won (for the guide line), or null if no snap. */
  snappedTo: number | null
}

/** Build snap candidates: other clips' edges + playhead + markers. */
export function snapCandidates(
  items: LaidItem[],
  markers: Marker[],
  playheadMs: number,
  excludeIds: Set<string>,
): number[] {
  const pts: number[] = [0, playheadMs]
  for (const it of items) {
    if (it.kind === 'gap' || excludeIds.has(it.id)) continue
    pts.push(it.startMs, it.startMs + it.durMs)
  }
  for (const m of markers) pts.push(m.at_ms)
  return pts
}

/**
 * Resolve snapping for a proposed [start,end] range: both edges are
 * candidates, globally nearest within 10 SCREEN px wins, anchor back-computed
 * through the edge offset. `bypass` = Shift held (checked per mousemove).
 */
export function resolveSnap(
  startMs: number,
  durMs: number,
  candidates: number[],
  zoom: number,
  bypass: boolean,
): SnapResult {
  if (bypass) return { ms: startMs, snappedTo: null }
  const thresholdMs = (SNAP_THRESHOLD_PX / (BASE_PPS * zoom)) * 1000
  let best: { dist: number; ms: number; point: number } | null = null
  for (const p of candidates) {
    const dStart = Math.abs(p - startMs)
    if (dStart <= thresholdMs && (!best || dStart < best.dist)) best = { dist: dStart, ms: p, point: p }
    const dEnd = Math.abs(p - (startMs + durMs))
    if (dEnd <= thresholdMs && (!best || dEnd < best.dist)) best = { dist: dEnd, ms: p - durMs, point: p }
  }
  return best ? { ms: Math.max(0, Math.round(best.ms)), snappedTo: best.point } : { ms: startMs, snappedTo: null }
}

// ---------------------------------------------------------------------------
// Ruler ticks: ladder + divisibility repair, windowed
// ---------------------------------------------------------------------------

/** Candidate intervals in ms: sub-second (for deep zoom) then seconds ladder. */
const INTERVALS_MS = [
  100, 200, 250, 500,
  1000, 2000, 3000, 5000, 10_000, 15_000, 30_000, 60_000,
  120_000, 300_000, 600_000, 900_000, 1_800_000, 3_600_000,
]

/** How a clock/ruler value is read out. */
export type TimeDisplayMode = 'ms' | 'frames' | 'smpte'

/**
 * Frame-aware ruler ladder. The base ms ladder bottoms out at 100ms = 3 frames
 * at 30fps, so the ruler can never show per-frame ticks however far you zoom.
 * Prepending whole-frame multiples (1/2/5/10/15 frames) lets ticks land on
 * frame boundaries at deep zoom, matching conventional NLE behavior.
 * 24fps → 41/83/208/417ms; 30fps → 33/66/165/330/495ms; deduped + sorted with
 * the second-scale ladder above 500ms.
 */
function frameAwareLadder(fps: number): number[] {
  const f = Math.max(1, Math.round(1000 / Math.max(1, fps)))
  const sub = [f, f * 2, f * 5, f * 10, f * 15].filter((v) => v > 0 && v < 500)
  return Array.from(new Set([...sub, ...INTERVALS_MS])).sort((a, b) => a - b)
}

export interface RulerTick {
  ms: number
  major: boolean
  label?: string
}

/** First ladder interval whose px spacing clears `minPx` at this zoom. */
function pickInterval(minPx: number, zoom: number, ladder: number[]): number {
  for (const iv of ladder) if (msToPx(iv, zoom) >= minPx) return iv
  return ladder[ladder.length - 1]
}

/**
 * Windowed tick list for [fromMs, toMs]. Labels need ≥120px, ticks ≥18px;
 * tick interval repaired to divide the label interval so labels land ON ticks.
 * The ladder is fps-aware so per-frame ticks appear at deep zoom regardless of
 * `mode`; `mode` selects how the major-tick LABEL reads (seconds / frames / SMPTE).
 */
export function rulerTicks(
  fromMs: number,
  toMs: number,
  zoom: number,
  fps = 30,
  mode: TimeDisplayMode = 'ms',
): RulerTick[] {
  const ladder = frameAwareLadder(fps)
  const labelIv = pickInterval(120, zoom, ladder)
  let tickIv = pickInterval(18, zoom, ladder)
  // Divisibility repair: walk the ladder up until tickIv divides labelIv.
  while (labelIv % tickIv !== 0) {
    const next = ladder[ladder.indexOf(tickIv) + 1]
    if (!next || next > labelIv) { tickIv = labelIv; break }
    tickIv = next
  }
  const ticks: RulerTick[] = []
  // FRAME REGIME: when the chosen interval is sub-second it came from the
  // frame-multiple part of the ladder — step in WHOLE FRAMES at EXACT frame
  // boundaries (Math.round(f/fps)), not integer-ms multiples, so ticks land on
  // real frames with no drift and labels read the exact frame index (kills the
  // "two ticks inside frame 0" artifact at deep zoom). Above 1s, step in ms on
  // the seconds ladder where whole-second alignment is what matters.
  const frameMs = 1000 / Math.max(1, fps)
  if (tickIv < 1000) {
    const tickF = Math.max(1, Math.round(tickIv / frameMs))
    const labelF = Math.max(tickF, Math.round(labelIv / frameMs))
    let f = Math.max(0, Math.ceil(fromMs / frameMs / tickF) * tickF)
    for (; ; f += tickF) {
      const ms = Math.round(f * frameMs)
      if (ms > toMs) break
      const major = f % labelF === 0
      ticks.push({ ms, major, label: major ? rulerLabelFrame(f, mode, fps) : undefined })
    }
  } else {
    const first = Math.max(0, Math.floor(fromMs / tickIv) * tickIv)
    for (let ms = first; ms <= toMs; ms += tickIv) {
      const major = ms % labelIv === 0
      ticks.push({ ms, major, label: major ? rulerLabel(ms, mode, fps) : undefined })
    }
  }
  return ticks
}

/**
 * Label a ruler tick that sits on an EXACT frame index `f` (frame regime).
 * Labels straight from the index so they never drift (vs recomputing from a
 * rounded ms, which would mislabel the frame at 29.97/24 boundaries).
 */
function rulerLabelFrame(f: number, mode: TimeDisplayMode, fps: number): string {
  if (mode === 'frames') return `${f}f`
  if (mode === 'smpte') {
    const fpsInt = Math.max(1, Math.round(fps))
    const p = (n: number) => String(n).padStart(2, '0')
    return `${p(Math.floor(f / (fpsInt * 3600)))}:${p(Math.floor(f / (fpsInt * 60)) % 60)}:${p(Math.floor(f / fpsInt) % 60)}:${p(f % fpsInt)}`
  }
  return rulerLabel(Math.round((f * 1000) / Math.max(1, fps)), 'ms', fps)
}

/**
 * Ruler label. ms mode: M:SS (H:MM:SS past an hour, S.d below 1s intervals).
 * frames mode: absolute frame number (`120f`). smpte mode: HH:MM:SS:FF.
 */
export function rulerLabel(ms: number, mode: TimeDisplayMode = 'ms', fps = 30): string {
  if (mode === 'frames') return `${framesOf(ms, fps)}f`
  if (mode === 'smpte') return timecodeSmpte(ms, fps)
  const totalS = ms / 1000
  const h = Math.floor(totalS / 3600)
  const m = Math.floor((totalS % 3600) / 60)
  const s = Math.floor(totalS % 60)
  const frac = ms % 1000
  const base = h > 0
    ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
    : `${m}:${String(s).padStart(2, '0')}`
  return frac > 0 ? `${base}.${String(Math.round(frac / 100))}` : base
}

/** Whole frames CONTAINING this ms (floor — frame N covers [N/fps,(N+1)/fps)). */
export function framesOf(ms: number, fps: number): number {
  return Math.floor((Math.max(0, ms) * Math.max(1, fps)) / 1000)
}

/**
 * SMPTE-style non-drop timecode HH:MM:SS:FF at the project fps. Floor-based so
 * the FF field names the frame currently shown (not the next). fps is rounded to
 * an integer frame count per labelled second (29.97 → 30); drop-frame notation
 * (the ;FF / 10-minute cadence) is deferred — broadcast-delivery only.
 */
export function timecodeSmpte(ms: number, fps: number): string {
  const fpsInt = Math.max(1, Math.round(fps))
  const total = framesOf(ms, fps)
  const ff = total % fpsInt
  const ss = Math.floor(total / fpsInt) % 60
  const mm = Math.floor(total / (fpsInt * 60)) % 60
  const hh = Math.floor(total / (fpsInt * 3600))
  const p = (n: number) => String(n).padStart(2, '0')
  return `${p(hh)}:${p(mm)}:${p(ss)}:${p(ff)}`
}

/** Clock readout (transport / chip / statusbar) in the chosen display mode. */
export function formatClock(ms: number, fps: number, mode: TimeDisplayMode): string {
  if (mode === 'frames') return `${framesOf(ms, fps)}f`
  if (mode === 'smpte') return timecodeSmpte(ms, fps)
  return timecode(ms)
}

/** Transport/playhead readout: HH:MM:SS.mmm, tabular-mono. */
export function timecode(ms: number): string {
  const t = Math.max(0, Math.round(ms))
  const h = Math.floor(t / 3_600_000)
  const m = Math.floor((t % 3_600_000) / 60_000)
  const s = Math.floor((t % 60_000) / 1000)
  const f = t % 1000
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}.${String(f).padStart(3, '0')}`
}

/** Short human duration for clip badges: `4.2s` / `1:03`. */
export function shortDur(ms: number): string {
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`
  const m = Math.floor(ms / 60_000)
  const s = Math.round((ms % 60_000) / 1000)
  return `${m}:${String(s).padStart(2, '0')}`
}
