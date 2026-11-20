import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { measureTimelineScrollPerformance } from '../src/panels/Timeline/timelinePerformance'
import type { LaidItem } from '../src/panels/Timeline/layout'
import { buildTimelineVisibleRangeIndex } from '../src/panels/Timeline/timelineVisibleRangeIndex'

const uiRoot = resolve(import.meta.dirname, '..')

function longFormItems(trackCount: number, clipsPerTrack: number): LaidItem[] {
  const items: LaidItem[] = []
  for (let track = 0; track < trackCount; track += 1) {
    for (let clip = 0; clip < clipsPerTrack; clip += 1) {
      const startMs = clip * 4_000
      items.push({
        id: `t${track}-c${clip}`,
        kind: 'video',
        contentClass: 'video',
        trackId: `v${track}`,
        startMs,
        editorialStartMs: startMs,
        durMs: 4_000,
        label: `clip ${track}:${clip}`,
        asset: `asset-${track}`,
        srcInMs: startMs,
        srcOutMs: startMs + 4_000,
      })
    }
  }
  return items
}

const trackCount = 6
const probe = measureTimelineScrollPerformance({
  items: longFormItems(trackCount, 600),
  trackCount,
  // 250px/s enables the windowed-thumbnail request planner. A 1000px lane
  // shows four seconds, or one clip on each of the six tracks.
  zoom: 5,
  laneWidthPx: 1_000,
  // A two-second 240Hz scroll input coalesced into 120 rAF state updates.
  scrollOffsetsPx: Array.from({ length: 120 }, (_, index) => index * 1_000),
  sourceScrollEvents: 480,
  intervalMs: 2_000,
})

assert.deepEqual(probe.eventRate, {
  sourceEvents: 480,
  sourceEventsPerSecond: 240,
  rafStateUpdates: 120,
  rafStateUpdatesPerSecond: 60,
})
assert.equal(probe.dirtyRanges.length, 120)
assert.deepEqual(probe.dirtyRanges[0], { startMs: 0, endMs: 4_000, visibleItems: 6, offscreenItems: 3_594 })
assert.deepEqual(probe.dirtyRanges.at(-1), { startMs: 476_000, endMs: 480_000, visibleItems: 6, offscreenItems: 3_594 })
assert.deepEqual(probe.renderCost, {
  legacyTrackRowExecutions: 720,
  legacyClipElementDescriptions: 432_000,
  memoizedTrackRowExecutions: 0,
  memoizedClipElementDescriptions: 0,
  avoidedTrackRowExecutions: 720,
  avoidedClipElementDescriptions: 432_000,
})
assert.deepEqual(probe.thumbnailCandidateChecks, {
  legacy: 432_000,
  indexBuildEntries: 3_600,
  indexed: 3_191,
  avoided: 428_809,
})

const rangeItems = [
  longFormItems(1, 1)[0],
  { ...longFormItems(1, 1)[0], id: 'long', startMs: 1_000, durMs: 12_000 },
  { ...longFormItems(1, 1)[0], id: 'before', startMs: 0, durMs: 500 },
  { ...longFormItems(1, 1)[0], id: 'after', startMs: 14_000, durMs: 4_000 },
]
const rangeIndex = buildTimelineVisibleRangeIndex(rangeItems)
for (const [startMs, endMs] of [[0, 1_000], [4_000, 5_000], [12_999, 14_000], [14_000, 15_000]]) {
  const expected = rangeItems.filter((item) => item.startMs + item.durMs > startMs && item.startMs < endMs).map((item) => item.id)
  assert.deepEqual(rangeIndex.query(startMs, endMs).items.map((item) => item.id), expected, `range ${startMs}-${endMs} preserves exact source-order intersections`)
}
assert.deepEqual(rangeIndex.query(4_000, 4_000), { items: [], examinedEntries: 0 })

assert.throws(
  () => measureTimelineScrollPerformance({
    items: [], trackCount: 1, zoom: 0, laneWidthPx: 100, scrollOffsetsPx: [], sourceScrollEvents: 0, intervalMs: 1,
  }),
  /zoom must be a finite positive number/,
)

const trackRow = readFileSync(resolve(uiRoot, 'src/panels/Timeline/TimelineTrackRow.tsx'), 'utf8')
const timeline = readFileSync(resolve(uiRoot, 'src/panels/Timeline/index.tsx'), 'utf8')
const windowedThumbnails = readFileSync(resolve(uiRoot, 'src/panels/Timeline/useWindowedThumbnails.ts'), 'utf8')
assert.match(trackRow, /const TimelineTrackRow = memo\(function TimelineTrackRow/, 'memo must own the track-row render')
assert.match(timeline, /const openTrackMenu = useCallback/, 'track-menu handler must retain identity across scroll state updates')
assert.match(timeline, /onOpenTrackMenu=\{openTrackMenu\}/, 'memoized rows must receive the stable track-menu handler')
assert.match(windowedThumbnails, /buildTimelineVisibleRangeIndex\(allItems\)/, 'thumbnail planning must index stable timeline geometry')
assert.match(windowedThumbnails, /visibleRangeIndex\.query\(viewLeftMs, viewRightMs\)\.items/, 'thumbnail planning must only inspect viewport intersections')

console.log('timeline performance probe passed')
