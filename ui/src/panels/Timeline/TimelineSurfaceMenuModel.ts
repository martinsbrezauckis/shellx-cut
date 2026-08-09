import type { Track } from '../../lib/client'
import type { ContextMenuActionState } from './ClipContextMenuModel'
import type { LaidItem } from './layout'
import { SPEED_FACTOR_MAX, SPEED_FACTOR_MIN } from './speedFactor'

export type TimelineSurfaceMenuState =
  | { kind: 'empty'; x: number; y: number; atMs: number; trackId: string | null }
  | { kind: 'gap'; x: number; y: number; itemId: string }
  | { kind: 'locked'; x: number; y: number; trackId: string; itemId: string | null; atMs: number }

export type TimelineContextTarget =
  | { kind: 'clip'; itemId: string }
  | TimelineSurfaceMenuState
  | { kind: 'none' }

const hidden = (reason: string): ContextMenuActionState => ({ visibility: 'hidden', enabled: false, reason })
const disabled = (reason: string): ContextMenuActionState => ({ visibility: 'visible', enabled: false, reason })
const enabled = (reason: string): ContextMenuActionState => ({ visibility: 'visible', enabled: true, reason })

/** Resolve the DOM ids into one authoritative target. A locked track wins over
 * a contained gap/clip so this menu can never route an edit while locked. */
export function resolveTimelineContextTarget(args: {
  itemId: string | null
  gapId: string | null
  trackId: string | null
  x: number
  y: number
  atMs: number
  items: LaidItem[]
  tracks: Track[]
}): TimelineContextTarget {
  // A DOM surface may identify at most one item. If an element claims a stale
  // or contradictory id, refuse rather than retargeting the nearest track.
  if (args.itemId && args.gapId) return { kind: 'none' }
  const itemId = args.gapId ?? args.itemId
  const item = itemId ? args.items.find((candidate) => candidate.id === itemId) ?? null : null
  if (itemId && !item) return { kind: 'none' }
  if (item && args.trackId && item.trackId !== args.trackId) return { kind: 'none' }
  if (!item && args.trackId && !args.tracks.some((candidate) => candidate.id === args.trackId)) return { kind: 'none' }
  const trackId = item?.trackId ?? args.trackId
  const track = trackId ? args.tracks.find((candidate) => candidate.id === trackId) ?? null : null
  if (track?.locked) return { kind: 'locked', x: args.x, y: args.y, trackId: track.id, itemId: item?.kind === 'gap' ? null : item?.id ?? null, atMs: args.atMs }
  if (item?.kind === 'gap') return { kind: 'gap', x: args.x, y: args.y, itemId: item.id }
  if (item) return { kind: 'clip', itemId: item.id }
  return { kind: 'empty', x: args.x, y: args.y, atMs: args.atMs, trackId: track?.id ?? null }
}

/** Gap fill can only use the exact clipboard source still present on a
 * same-kind unlocked track. The engine computes the final fit; this state makes
 * its published 0.25–4× prerequisite visible before dispatch. */
export function gapFillState(gap: LaidItem, track: Track | null, source: LaidItem | null): ContextMenuActionState {
  if (!track || track.locked) return disabled('Unlock this track before filling its gap')
  if (!source) return disabled('Copy a media clip that is still on the timeline first')
  if ((source.kind !== 'video' && source.kind !== 'audio') || source.kind !== track.kind) {
    return disabled(`A copied ${track.kind} clip is required for this ${track.kind} gap`)
  }
  const sourceSpan = (source.srcOutMs ?? 0) - (source.srcInMs ?? 0)
  const factor = sourceSpan / gap.durMs
  if (!Number.isFinite(factor) || factor < SPEED_FACTOR_MIN || factor > SPEED_FACTOR_MAX) {
    return disabled(`This source would need ${factor.toFixed(2)}×; Fit to fill supports ${SPEED_FACTOR_MIN}×–${SPEED_FACTOR_MAX}×`)
  }
  return enabled(`Fit this copied clip into the ${Math.round(gap.durMs)}ms gap at ${factor.toFixed(2)}×`)
}

export function removeTrackState(track: Track | null, tracks: Track[]): ContextMenuActionState {
  if (!track) return hidden('Track is no longer available')
  const baseVideo = tracks.find((candidate) => candidate.kind === 'video')?.id
  const baseAudio = tracks.find((candidate) => candidate.kind === 'audio')?.id
  return track.id === baseVideo || track.id === baseAudio
    ? disabled('The base video and audio tracks cannot be removed')
    : enabled(`Remove track ${track.id} after confirmation`)
}
