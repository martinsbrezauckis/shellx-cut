import type { Project, Track, TrackKind } from '../../lib/client'
import { sourceMsAtTimelinePosition, timelineMsAtSourcePosition, type TimelineSourceWindow } from '../../lib/mediaTime'

interface SourceLaidItem {
  id: string
  kind: 'video' | 'audio' | 'caption' | 'gap'
  startMs: number
  durMs: number
  asset?: string
  srcInMs?: number
  srcOutMs?: number
  speed?: number
  reverse?: boolean
}

type LayoutTrack = (track: Track) => SourceLaidItem[]

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

/** A source-frame resolution for a deliberate timeline target.  Unlike the
 * generic playhead helper below, this carries a concise refusal reason so a
 * visible Match Frame control can stay honest when its exact mapping is not
 * available. */
export type SourceFrameMatch =
  | { source: SourceAt; clipId: string; trackId: string }
  | { source: null; reason: string }

function sourceWindowForItem(item: SourceLaidItem, freezeAtMs?: number | null): TimelineSourceWindow | null {
  if (item.srcInMs === undefined || item.srcOutMs === undefined) return null
  return {
    startMs: item.startMs,
    srcInMs: item.srcInMs,
    srcOutMs: item.srcOutMs,
    speed: item.speed,
    reverse: item.reverse,
    freezeAtMs,
  }
}

/**
 * Resolve a source-media instant to every video-timeline occurrence that shows
 * it. Visual-search hits are source-relative; sending `peak_ms` straight to
 * ui.playhead is wrong after a trim, delay, reuse, speed change, or reverse.
 * Variable-speed ramps are deliberately omitted here because the UI model does
 * not yet carry the engine's ramp segments; callers keep Source as the exact
 * fallback instead of presenting an approximate timeline jump.
 */
export function sourceTimelineOccurrencesForLayout(
  project: Project | null,
  assetId: string,
  sourceMs: number,
  layoutTrack: LayoutTrack,
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
      const window = sourceWindowForItem(item, raw.freeze?.at_ms)
      if (!window) continue
      // A held frame appears throughout the slot. One deterministic occurrence
      // at its start is enough to navigate a visual-search result; no other
      // source timestamp occurs in that frozen image.
      if (raw.freeze) {
        if (sourceMs !== sourceMsAtTimelinePosition(window, item.startMs)) continue
        found.push({ clipId: item.id, trackId: track.id, atMs: item.startMs })
        continue
      }
      const timelineOffset = Math.round(timelineMsAtSourcePosition(window, sourceMs) - item.startMs)
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
 * a clip at timeline `startMs` plays source `[src_in_ms, src_out_ms)`. Without
 * this walk a transcript/word lookup that treats timelineMs AS source ms drifts
 * by the total removed duration before the playhead. Constant speed, reverse,
 * and freeze use the shared media clock; speed ramps return null rather than
 * inventing a mapping because the UI model does not carry the engine's sampled
 * ramp segments.
 *
 * Resolution: prefer the covering VIDEO clip (the speech reference); fall back
 * to the covering AUDIO clip when no video track covers the position (audio-only
 * sections). Returns null over a gap, past the end, an unsupported speed ramp,
 * or with no project.
 */
export function sourceAtPlayheadForLayout(
  project: Project | null,
  timelineMs: number,
  layoutTrack: LayoutTrack,
): SourceAt | null {
  if (!project) return null
  const find = (kind: TrackKind): { covered: boolean; source: SourceAt | null } => {
    for (const track of project.tracks) {
      if (track.kind !== kind) continue
      for (const item of layoutTrack(track)) {
        if (item.kind === 'gap' || item.kind === 'caption' || !item.asset || item.srcInMs === undefined) continue
        if (timelineMs >= item.startMs && timelineMs < item.startMs + item.durMs) {
          const raw = track.clips.find((clip) => 'id' in clip && clip.id === item.id)
          if (!raw || !('asset' in raw)) return { covered: true, source: null }
          // Ramps have a non-linear source clock. layoutTrack intentionally
          // does not model their engine-expanded segments, so fail closed until
          // that exact representation is available rather than mis-highlighting
          // a transcript word.
          if ((raw as { speed_ramp?: unknown }).speed_ramp != null) return { covered: true, source: null }
          const window = sourceWindowForItem(item, raw.freeze?.at_ms)
          return {
            covered: true,
            source: window ? { asset: item.asset, srcMs: sourceMsAtTimelinePosition(window, timelineMs) } : null,
          }
        }
      }
    }
    return { covered: false, source: null }
  }
  const video = find('video')
  return video.covered ? video.source : find('audio').source
}

/**
 * Resolve one explicitly targeted VIDEO clip or video track at the playhead to
 * its exact source frame. Match Frame is intentionally narrower than generic
 * source lookup: it never switches to another video track or falls back to
 * audio. Constant speed, reverse, and freeze use the shared source clock;
 * speed ramps have no authoritative UI mapping and fail closed.
 */
export function sourceFrameMatchForLayout(
  project: Project | null,
  timelineMs: number,
  target: { clipId?: string; trackId?: string },
  layoutTrack: LayoutTrack,
): SourceFrameMatch {
  if (!project) return { source: null, reason: 'Open a project first' }
  if (!Number.isFinite(timelineMs)) return { source: null, reason: 'Move the playhead to a video frame' }
  if (!target.clipId && !target.trackId) return { source: null, reason: 'Choose a video clip or track first' }

  const track = target.trackId
    ? project.tracks.find((candidate) => candidate.id === target.trackId) ?? null
    : null
  if (target.trackId && !track) return { source: null, reason: 'That video track is no longer available' }
  if (track && track.kind !== 'video') return { source: null, reason: 'Match Frame needs a video track' }

  const candidates = track ? [track] : project.tracks
  // A track header names a lane, not one of its clips. During a crossfade (or
  // any other laid overlap) two visible media clips cover the same playhead,
  // so choosing the first EDL item would only be deterministic by accident.
  // An explicit clip target remains authoritative: its identity disambiguates
  // the source frame even while another clip is visible underneath/over it.
  if (track && !target.clipId) {
    const coveringMedia = layoutTrack(track).filter((item) => (
      item.kind === 'video'
      && !!item.asset
      && timelineMs >= item.startMs
      && timelineMs < item.startMs + item.durMs
    ))
    if (coveringMedia.length > 1) {
      return {
        source: null,
        reason: 'More than one video clip covers this playhead; select one clip to match its exact frame',
      }
    }
  }
  for (const candidate of candidates) {
    if (candidate.kind !== 'video') continue
    for (const item of layoutTrack(candidate)) {
      if (target.clipId && item.id !== target.clipId) continue
      if (item.kind !== 'video' || !item.asset || item.srcInMs === undefined || item.srcOutMs === undefined) continue
      if (timelineMs < item.startMs || timelineMs >= item.startMs + item.durMs) {
        if (target.clipId) return { source: null, reason: 'Move the playhead onto this video clip' }
        continue
      }
      const asset = project.assets?.[item.asset]
      if (project.assets && !asset) return { source: null, reason: 'This source is no longer available' }
      const probeKind = asset?.probe && typeof asset.probe === 'object' && 'kind' in asset.probe
        ? (asset.probe as { kind?: unknown }).kind
        : undefined
      if (probeKind && probeKind !== 'video') {
        return { source: null, reason: 'Match Frame needs a video source' }
      }
      const raw = candidate.clips.find((clip) => 'id' in clip && clip.id === item.id)
      if (!raw || !('asset' in raw)) return { source: null, reason: 'This clip no longer has a source' }
      if ((raw as { speed_ramp?: unknown }).speed_ramp != null) {
        return { source: null, reason: 'Speed ramps cannot match an exact source frame' }
      }
      const window = sourceWindowForItem(item, raw.freeze?.at_ms)
      if (!window) return { source: null, reason: 'This clip has no source range' }
      return {
        source: { asset: item.asset, srcMs: sourceMsAtTimelinePosition(window, timelineMs) },
        clipId: item.id,
        trackId: candidate.id,
      }
    }
  }

  return { source: null, reason: target.clipId ? 'This clip is not on a video track' : 'No video clip covers this playhead' }
}
