import type { LaidItem } from './layout'

interface TimelineRangeEntry {
  item: LaidItem
  sourceIndex: number
  startMs: number
  endMs: number
}

interface TimelineRangeNode {
  entry: TimelineRangeEntry
  minStartMs: number
  maxEndMs: number
  left: TimelineRangeNode | null
  right: TimelineRangeNode | null
}

export interface TimelineVisibleRangeQuery {
  /** Items that can intersect the requested timeline range, in source order. */
  items: readonly LaidItem[]
  /** Interval-index entries examined while resolving this range. */
  examinedEntries: number
}

export interface TimelineVisibleRangeIndex {
  query(startMs: number, endMs: number): TimelineVisibleRangeQuery
}

function buildNode(entries: readonly TimelineRangeEntry[], from: number, to: number): TimelineRangeNode | null {
  if (from >= to) return null
  const middle = from + Math.floor((to - from) / 2)
  const left = buildNode(entries, from, middle)
  const right = buildNode(entries, middle + 1, to)
  const entry = entries[middle]
  return {
    entry,
    minStartMs: Math.min(entry.startMs, left?.minStartMs ?? Infinity, right?.minStartMs ?? Infinity),
    maxEndMs: Math.max(entry.endMs, left?.maxEndMs ?? -Infinity, right?.maxEndMs ?? -Infinity),
    left,
    right,
  }
}

/**
 * Index stable timeline geometry so scroll-driven thumbnail planning only
 * examines clips that can intersect the viewport. The result is restored to
 * the caller's source order, which preserves request scheduling semantics.
 */
export function buildTimelineVisibleRangeIndex(items: readonly LaidItem[]): TimelineVisibleRangeIndex {
  const entries = items.map((item, sourceIndex) => ({
    item,
    sourceIndex,
    startMs: item.startMs,
    endMs: item.startMs + item.durMs,
  })).sort((a, b) => a.startMs - b.startMs || a.sourceIndex - b.sourceIndex)
  const root = buildNode(entries, 0, entries.length)

  return {
    query(startMs, endMs) {
      if (!Number.isFinite(startMs) || !Number.isFinite(endMs) || endMs <= startMs) {
        return { items: [], examinedEntries: 0 }
      }

      const matches: TimelineRangeEntry[] = []
      let examinedEntries = 0
      const visit = (node: TimelineRangeNode | null) => {
        if (!node) return
        examinedEntries += 1
        if (node.maxEndMs <= startMs || node.minStartMs >= endMs) return
        visit(node.left)
        if (node.entry.endMs > startMs && node.entry.startMs < endMs) matches.push(node.entry)
        visit(node.right)
      }
      visit(root)
      matches.sort((a, b) => a.sourceIndex - b.sourceIndex)
      return { items: matches.map(({ item }) => item), examinedEntries }
    },
  }
}
