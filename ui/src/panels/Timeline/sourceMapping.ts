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
