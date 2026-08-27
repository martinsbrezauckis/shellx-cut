import { pxToMs, type LaidItem } from './layout'
import { buildTimelineVisibleRangeIndex } from './timelineVisibleRangeIndex'

/**
 * Deterministic cost model for a scroll-only Timeline update.
 *
 * This deliberately counts work the source must schedule instead of elapsed
 * milliseconds: wall-clock timings vary with browser, GPU, and host load. The
 * model is for a stable project/selection/thumbnail snapshot; project edits,
 * selection changes, and thumbnail arrivals are separate invalidations.
 */
export interface TimelineScrollPerformanceInput {
  items: readonly LaidItem[]
  trackCount: number
  zoom: number
  laneWidthPx: number
  /** One offset for every rAF-batched scroll state update under test. */
  scrollOffsetsPx: readonly number[]
  /** Raw browser scroll events received during the measured interval. */
  sourceScrollEvents: number
  intervalMs: number
}

export interface TimelineDirtyRange {
  startMs: number
  endMs: number
  visibleItems: number
  offscreenItems: number
}

export interface TimelineScrollPerformance {
  eventRate: {
    sourceEvents: number
    sourceEventsPerSecond: number
    rafStateUpdates: number
    rafStateUpdatesPerSecond: number
  }
  dirtyRanges: TimelineDirtyRange[]
  renderCost: {
    /** TimelineTrackRow executions before row memoization. */
    legacyTrackRowExecutions: number
    /** ClipView element descriptions constructed inside those row executions. */
    legacyClipElementDescriptions: number
    /** Scroll alone does not change a lane's content or geometry. */
    memoizedTrackRowExecutions: number
    memoizedClipElementDescriptions: number
    avoidedTrackRowExecutions: number
    avoidedClipElementDescriptions: number
  }
  thumbnailCandidateChecks: {
    /** Pre-index linear scan, once per scroll state update. */
    legacy: number
    /** Stable index records created once for the supplied timeline geometry. */
    indexBuildEntries: number
    /** Interval-index entries examined across the supplied scroll updates. */
    indexed: number
    avoided: number
  }
}

function finiteNonNegative(value: number, name: string): void {
  if (!Number.isFinite(value) || value < 0) throw new Error(`${name} must be a finite non-negative number`)
}

function ratePerSecond(count: number, intervalMs: number): number {
  return Number(((count * 1000) / intervalMs).toFixed(3))
}

/**
 * Measure the exact scroll-only render fan-out for a supplied timeline shape.
 *
 * `legacy*` describes the TimelineTrackRow ownership before it was memoized:
 * each rAF state update ran every row, which mapped every item into a ClipView
 * element. `memoized*` is the expected work after this module's narrow fix,
 * when all row props other than scroll position retain identity. Thumbnail
 * request planning resolves a stable visible-range index rather than scanning
 * every item on each activated-zoom scroll update.
 */
export function measureTimelineScrollPerformance(input: TimelineScrollPerformanceInput): TimelineScrollPerformance {
  finiteNonNegative(input.trackCount, 'trackCount')
  finiteNonNegative(input.laneWidthPx, 'laneWidthPx')
  finiteNonNegative(input.sourceScrollEvents, 'sourceScrollEvents')
  if (!Number.isFinite(input.zoom) || input.zoom <= 0) throw new Error('zoom must be a finite positive number')
  if (!Number.isFinite(input.intervalMs) || input.intervalMs <= 0) throw new Error('intervalMs must be a finite positive number')

  const rafStateUpdates = input.scrollOffsetsPx.length
  const itemCount = input.items.length
  const visibleRangeIndex = buildTimelineVisibleRangeIndex(input.items)
  let indexedThumbnailCandidateChecks = 0
  const dirtyRanges = input.scrollOffsetsPx.map((scrollLeftPx) => {
    finiteNonNegative(scrollLeftPx, 'scrollOffsetsPx entry')
    const startMs = pxToMs(scrollLeftPx, input.zoom)
    const endMs = pxToMs(scrollLeftPx + input.laneWidthPx, input.zoom)
    const visible = visibleRangeIndex.query(startMs, endMs)
    indexedThumbnailCandidateChecks += visible.examinedEntries
    const visibleItems = visible.items.length
    return { startMs, endMs, visibleItems, offscreenItems: itemCount - visibleItems }
  })

  const legacyTrackRowExecutions = input.trackCount * rafStateUpdates
  const legacyClipElementDescriptions = itemCount * rafStateUpdates
  return {
    eventRate: {
      sourceEvents: input.sourceScrollEvents,
      sourceEventsPerSecond: ratePerSecond(input.sourceScrollEvents, input.intervalMs),
      rafStateUpdates,
      rafStateUpdatesPerSecond: ratePerSecond(rafStateUpdates, input.intervalMs),
    },
    dirtyRanges,
    renderCost: {
      legacyTrackRowExecutions,
      legacyClipElementDescriptions,
      memoizedTrackRowExecutions: 0,
      memoizedClipElementDescriptions: 0,
      avoidedTrackRowExecutions: legacyTrackRowExecutions,
      avoidedClipElementDescriptions: legacyClipElementDescriptions,
    },
    thumbnailCandidateChecks: {
      legacy: itemCount * rafStateUpdates,
      indexBuildEntries: itemCount,
      indexed: indexedThumbnailCandidateChecks,
      avoided: itemCount * rafStateUpdates - indexedThumbnailCandidateChecks,
    },
  }
}
