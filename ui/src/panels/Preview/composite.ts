// panels/Preview/composite.ts — pure resolver for the LIVE composite stage.
//
// Role: given the project + a timeline position, resolve the layer stack the
// monitor renders for a real-time, GPU-composited preview:
//   - OVERLAY video layers: the clip under the playhead on every video track
//     ABOVE the primary/base track, with normalized PiP geometry + opacity +
//     (approximate) grade. Rendered as stacked <video> elements in a stage that
//     carries the project aspect ratio, so normalized coords map 1:1.
//   - CAPTION layers: caption clips covering the playhead, as DOM text.
//   - GRADE: a CSS `filter` string that APPROXIMATES the engine's per-clip grade
//     (contrast/brightness/saturation accurate-ish; temperature a light tint;
//     gamma + 3D LUT are NOT CSS-expressible → omitted in the live view). The
//     exact grade stays the COMPOSED toggle (/api/frame?compose=1) + render.final.
//
// This is DERIVED, deliberately-approximate feedback (mirrors preview.rs's
// doctrine): smooth + instant for editing; exactness lives in the render path.
// No React/DOM here — index.tsx consumes these and owns the elements + clock.
//
// Callers: panels/Preview/index.tsx. Deps: lib/client types, Timeline/layout.

import type { CSSProperties } from 'react'
import { scopeMediaUrl } from '../../lib/mediaScope'
import { sourceUrl, type CaptionStyle, type ClipGrade, type ClipTransform, type Project } from '../../lib/client'
import { layoutTrack, type LaidItem } from '../Timeline/layout'

/**
 * Max simultaneous overlay <video> elements decoded at once. Each is a 960×540
 * proxy; 4 covers realistic PiP / B-roll / lower-third stacking without
 * straining the WebView decoder. Past this we keep the FRONT-most (most visible)
 * overlays and report the dropped count so the UI can note the cap honestly.
 */
export const MAX_OVERLAYS = 4

/** How far ahead an overlay clip is loaded and parked (matches the base deck). */
export const OVERLAY_PREROLL_MS = 3000

/** One resolved overlay video layer (a clip on a track above the base). */
export interface OverlayLayer {
  clipId: string
  trackId: string
  /** Proxy URL when built, else the streamed source. */
  src: string
  kind: 'proxy' | 'source'
  assetId: string
  /** Timeline placement of the overlay clip (for master-clock sync). */
  startMs: number
  srcInMs: number
  srcOutMs: number
  durMs: number
  speed: number
  reverse: boolean
  freezeAtMs?: number
  /** Normalized geometry + opacity (defaults to full-frame opaque). */
  transform: Required<ClipTransform>
  /** Approximate grade for the CSS filter, when the clip is graded. */
  grade?: ClipGrade | null
  /** Still image (probe kind=image) — rendered from source, no seek/loop sync. */
  isImage: boolean
}

/** One resolved caption layer (a caption clip covering the playhead). */
export interface CaptionLayer {
  id: string
  text: string
  style: CaptionStyle
}

/** What resolveOverlays returns: the (front-most-capped) overlays + how many
 * were dropped by the decode cap (0 normally). */
export interface OverlayResolution {
  overlays: OverlayLayer[]
  dropped: number
}

/**
 * Whether the live stage (playing proxy <video>s) is mounted: whenever there is
 * something playable, paused or playing, in either mode. It is never unmounted
 * to show the exact frame while paused; the composed frame is drawn ON TOP of it
 * (see composedFrameOnTop) so starting play never re-creates media elements,
 * which on some webviews (WebKitGTK) shows the wrong picture for seconds.
 */
export function shouldUseLivePreviewSurface(hasVideo: boolean): boolean {
  return hasVideo
}

/** The exact composed frame covers the live stage while the playhead rests:
 * COMPOSED on (the default) and paused. Playing, the live stage shows. */
export function composedFrameOnTop(composed: boolean, rate: number, liveStage: boolean): boolean {
  return composed && rate === 0 && liveStage
}

/** A base-track gap (no base clip under the playhead) shown on the live
 * stage: during forward play always (overlays play as video over black, the
 * base deck parks the next clip), and paused when overlay clips are visible
 * there, like a paused base clip, so starting or stopping play does not swap
 * surfaces. An empty gap keeps the engine frame (and its "no clip" note). */
export function liveBaseGap(inGap: boolean, rate: number, overlayCount: number): boolean {
  return inGap && (rate === 1 || (rate === 0 && overlayCount > 0))
}

/** Project geometry aspect ratio (w/h) for the stage box; 16:9 fallback. */
export function stageAspect(project: Project | null): number {
  const w = project?.settings.width ?? 16
  const h = project?.settings.height ?? 9
  return h > 0 ? w / h : 16 / 9
}

/** Ordered ids of the project's VIDEO tracks (compositing order = array order;
 * first with clips is the base canvas, each later one stacks above). */
export function videoTrackIds(project: Project | null): string[] {
  if (!project) return []
  return project.tracks.filter((t) => t.kind === 'video').map((t) => t.id)
}

/** Fill a (possibly-partial / missing) transform to a full opaque identity. */
function fullTransform(t: ClipTransform | null | undefined): Required<ClipTransform> {
  return {
    x: t?.x ?? 0,
    y: t?.y ?? 0,
    scale: t?.scale ?? 1,
    opacity: t?.opacity ?? 1,
  }
}

/** The video tracks stacked above the primary track, base to front (callers
 * skip hidden ones). */
function overlayTracks(project: Project | null, primaryTrackId: string | null) {
  if (!project || !primaryTrackId) return null
  const vids = project.tracks.filter((t) => t.kind === 'video')
  const primaryIdx = vids.findIndex((t) => t.id === primaryTrackId)
  if (primaryIdx < 0) return null
  return vids.slice(primaryIdx + 1)
}

/** One overlay layer for a laid clip, or null when its asset is unknown. */
function overlayLayer(project: Project, trackId: string, it: LaidItem, imageAssets: Set<string>): OverlayLayer | null {
  const asset = it.asset ? project.assets[it.asset] : undefined
  if (!it.asset || !asset) return null
  // Prefer the proxy; fall back to streaming the source (also when the
  // media-address guard refuses the proxy address and gives '').
  const proxied = asset.proxy ? scopeMediaUrl(asset.proxy) : ''
  const src = proxied || sourceUrl(it.asset)
  const kind: 'proxy' | 'source' = proxied ? 'proxy' : 'source'
  return {
    clipId: it.id,
    trackId,
    src,
    kind,
    assetId: it.asset,
    startMs: it.startMs,
    srcInMs: it.srcInMs ?? 0,
    srcOutMs: it.srcOutMs ?? (it.srcInMs ?? 0) + it.durMs,
    durMs: it.durMs,
    speed: it.speed && it.speed > 0 ? it.speed : 1,
    reverse: !!it.reverse,
    freezeAtMs: it.freezeAtMs,
    transform: fullTransform(it.transform),
    grade: it.grade ?? null,
    isImage: imageAssets.has(it.asset),
  }
}

/** Overlay VIDEO clips that start within `horizonMs` after `atMs` on a track
 * with room under the decode cap (see MAX_OVERLAYS), so the monitor can load
 * and park each one on its first frame before it appears (the same reason as
 * the base deck: an element created at the cut shows the wrong picture on some
 * webviews until its first seek completes). Stills need no parking. */
export function resolveUpcomingOverlays(
  project: Project | null,
  atMs: number,
  primaryTrackId: string | null,
  imageAssets: Set<string>,
  active: readonly OverlayLayer[],
  horizonMs = OVERLAY_PREROLL_MS,
): OverlayLayer[] {
  const tracks = overlayTracks(project, primaryTrackId)
  if (!project || !tracks) return []
  const out: OverlayLayer[] = []
  for (const track of tracks) {
    if (active.length + out.length >= MAX_OVERLAYS) break
    if (track.visible === false) continue
    for (const it of layoutTrack(track, imageAssets)) {
      if (it.kind !== 'video' || !it.asset || imageAssets.has(it.asset)) continue
      if (it.startMs <= atMs) continue
      if (it.startMs > atMs + horizonMs) break
      const layer = overlayLayer(project, track.id, it, imageAssets)
      if (layer) out.push(layer)
      break
    }
  }
  return out
}

/**
 * Resolve the overlay layers stacked ABOVE `primaryTrackId` at timeline `atMs`.
 * Walks each video track AFTER the primary (in project/compositing order),
 * takes the clip covering the playhead (if any), and builds an OverlayLayer with
 * its PiP transform + grade. Layers are returned base→front (stable DOM order =
 * stacking). If more than MAX_OVERLAYS are active, the FRONT-most are kept and
 * the rest reported as `dropped`.
 */
export function resolveOverlays(
  project: Project | null,
  atMs: number,
  primaryTrackId: string | null,
  imageAssets: Set<string>,
): OverlayResolution {
  const tracks = overlayTracks(project, primaryTrackId)
  if (!project || !tracks) return { overlays: [], dropped: 0 }

  const out: OverlayLayer[] = []
  for (const track of tracks) {
    if (track.visible === false) continue
    for (const it of layoutTrack(track, imageAssets)) {
      if (it.kind !== 'video' || !it.asset) continue
      if (atMs < it.startMs || atMs >= it.startMs + it.durMs) continue
      const layer = overlayLayer(project, track.id, it, imageAssets)
      if (!layer) continue
      out.push(layer)
      break // one clip per track covers a given playhead position
    }
  }
  // Decode cap: keep the FRONT-most (last = most-visible) MAX_OVERLAYS.
  if (out.length > MAX_OVERLAYS) {
    const dropped = out.length - MAX_OVERLAYS
    return { overlays: out.slice(out.length - MAX_OVERLAYS), dropped }
  }
  return { overlays: out, dropped: 0 }
}

/** The base/primary clip's grade at `atMs` (to filter the base <video>), or
 * null. `primaryTrackId` is whatever the base layer resolved to. */
export function baseGradeAt(
  project: Project | null,
  atMs: number,
  primaryTrackId: string | null,
  imageAssets: Set<string>,
): ClipGrade | null {
  if (!project || !primaryTrackId) return null
  const track = project.tracks.find((t) => t.id === primaryTrackId)
  if (!track) return null
  for (const it of layoutTrack(track, imageAssets)) {
    if (it.kind !== 'video') continue
    if (atMs >= it.startMs && atMs < it.startMs + it.durMs) return it.grade ?? null
  }
  return null
}

/** Caption clips whose range covers `atMs`, with their resolved style. */
export function resolveCaptions(project: Project | null, atMs: number): CaptionLayer[] {
  if (!project) return []
  const out: CaptionLayer[] = []
  for (const track of project.tracks) {
    if (track.kind !== 'caption' || track.visible === false) continue
    for (const it of layoutTrack(track)) {
      if (it.kind !== 'caption') continue
      if (atMs < it.startMs || atMs >= it.startMs + it.durMs) continue
      // style_ref → caption_styles; fall back to a readable default.
      const clip = track.clips.find((c) => 'id' in c && c.id === it.id) as
        | { style_ref?: string }
        | undefined
      const ref = clip?.style_ref
      const style = (ref && project.caption_styles[ref]) || DEFAULT_CAPTION_STYLE
      out.push({ id: it.id, text: it.label, style })
    }
  }
  return out
}

/** A readable default when a caption clip has no resolvable style. */
export const DEFAULT_CAPTION_STYLE: CaptionStyle = {
  font: 'Inter, system-ui, sans-serif',
  size: 48,
  color: '#ffffff',
  bg: 'rgba(0,0,0,0.55)',
  pos: 'bottom',
}

/**
 * APPROXIMATE the engine's per-clip grade as a CSS `filter` string.
 * - contrast / saturation: CSS contrast()/saturate() are multiplicative with
 *   1.0 = identity — same convention as ffmpeg `eq` → passed through.
 * - brightness: ffmpeg eq brightness is ADDITIVE (0 = identity, ~[-1,1]); CSS
 *   brightness() is MULTIPLICATIVE (1 = identity) → mapped as 1 + brightness.
 * - temperature_k: a LIGHT warm/cool tint via sepia()+hue-rotate(), scaled by
 *   the distance from ~6500K neutral and clamped — a hint, not a match.
 * - gamma + LUT: NOT expressible in CSS → omitted (exact stays render path).
 * Returns '' (no filter) for a missing/identity grade.
 */
export function gradeFilter(grade: ClipGrade | null | undefined): string {
  if (!grade) return ''
  const parts: string[] = []
  const contrast = grade.contrast ?? 1
  const brightness = grade.brightness ?? 0
  const saturation = grade.saturation ?? 1
  if (contrast !== 1) parts.push(`contrast(${clamp(contrast, 0, 4).toFixed(3)})`)
  if (brightness !== 0) parts.push(`brightness(${clamp(1 + brightness, 0, 4).toFixed(3)})`)
  if (saturation !== 1) parts.push(`saturate(${clamp(saturation, 0, 4).toFixed(3)})`)
  const tk = grade.temperature_k
  if (typeof tk === 'number' && tk > 0) {
    // Warmer (lower K) → small sepia + slight negative hue; cooler → slight
    // positive hue. Distance from 6500K, capped so the tint stays subtle.
    const d = clamp((6500 - tk) / 6500, -0.6, 0.6) // + = warmer
    const sep = Math.min(0.4, Math.abs(d) * 0.5)
    if (sep > 0.01) parts.push(`sepia(${sep.toFixed(3)})`)
    const hue = -d * 18 // warm tilts hue down, cool up; degrees
    if (Math.abs(hue) > 0.5) parts.push(`hue-rotate(${hue.toFixed(1)}deg)`)
  }
  return parts.join(' ')
}

/** Inline style box for an overlay clip from its normalized transform. The
 * stage carries the project aspect, so normalized maps 1:1: left=x, top=y,
 * width=scale, height=scale (the overlay is the conformed full frame scaled —
 * same aspect as the stage). opacity from the transform. */
export function overlayBoxStyle(t: Required<ClipTransform>): CSSProperties {
  return {
    left: `${(t.x * 100).toFixed(4)}%`,
    top: `${(t.y * 100).toFixed(4)}%`,
    width: `${(t.scale * 100).toFixed(4)}%`,
    height: `${(t.scale * 100).toFixed(4)}%`,
    opacity: t.opacity,
  }
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v))
}
