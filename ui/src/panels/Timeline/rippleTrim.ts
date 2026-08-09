import { sourceMsAtTimelinePosition } from '../../lib/mediaTime'
import type { LaidItem } from './layout'

export type RippleTrimSide = 'start' | 'end'

interface RippleTrimPlanBase {
  clipId: string
  trackId: string
  side: RippleTrimSide
  rangeMs: [number, number]
  seekMs: number
}

export type RippleTrimPlan = RippleTrimPlanBase & (
  | { operation: 'trim'; trim: { src_in_ms?: number; src_out_ms?: number } }
  | { operation: 'delete' }
)

export function sourceTrimAtTimelinePosition(
  item: LaidItem,
  edge: RippleTrimSide,
  atMs: number,
): { src_in_ms?: number; src_out_ms?: number } | null {
  if (item.srcInMs === undefined || item.srcOutMs === undefined) return null
  const sourceCutMs = sourceMsAtTimelinePosition({
    startMs: item.startMs,
    srcInMs: item.srcInMs,
    srcOutMs: item.srcOutMs,
    speed: item.speed,
    reverse: item.reverse,
  }, atMs)
  if (edge === 'start') {
    return item.reverse ? { src_out_ms: sourceCutMs } : { src_in_ms: sourceCutMs }
  }
  return item.reverse ? { src_in_ms: sourceCutMs } : { src_out_ms: sourceCutMs }
}

function mediaAtPlayhead(item: LaidItem, playheadMs: number, side: RippleTrimSide): boolean {
  if (item.kind !== 'video' && item.kind !== 'audio') return false
  const endMs = item.startMs + item.durMs
  return side === 'start'
    ? item.startMs < playheadMs && playheadMs <= endMs
    : item.startMs <= playheadMs && playheadMs < endMs
}

/**
 * Plan the NLE Q/W edit without mutating timeline state. Selection wins when it
 * is under the playhead; otherwise the first video lane is the program target.
 * Boundary rules deliberately choose the clip before the playhead for Q and the
 * clip after it for W.
 */
export function planRippleTrimAtPlayhead(
  items: LaidItem[],
  selectedIds: string[],
  playheadMs: number,
  side: RippleTrimSide,
): RippleTrimPlan | null {
  if (!Number.isFinite(playheadMs)) return null
  const atPlayhead = items.filter((item) => mediaAtPlayhead(item, playheadMs, side))
  const selected = new Set(selectedIds)
  const candidates = atPlayhead.some((item) => selected.has(item.id))
    ? atPlayhead.filter((item) => selected.has(item.id))
    : atPlayhead
  const item = candidates.find((candidate) => candidate.kind === 'video') ?? candidates[0]
  if (!item || item.srcInMs === undefined || item.srcOutMs === undefined) return null

  const trim = sourceTrimAtTimelinePosition(item, side, playheadMs)
  if (!trim) return null

  const rangeMs: [number, number] = side === 'start'
    ? [item.startMs, Math.round(playheadMs)]
    : [Math.round(playheadMs), item.startMs + item.durMs]
  if (rangeMs[0] >= rangeMs[1]) return null

  const removesWholeClip = rangeMs[0] === item.startMs
    && rangeMs[1] === item.startMs + item.durMs

  return {
    clipId: item.id,
    trackId: item.trackId,
    side,
    rangeMs,
    seekMs: side === 'start' ? item.startMs : Math.round(playheadMs),
    ...(removesWholeClip ? { operation: 'delete' as const } : { operation: 'trim' as const, trim }),
  }
}
