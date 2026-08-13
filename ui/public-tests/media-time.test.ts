import assert from 'node:assert/strict'
import { sourceMsAtTimelinePosition, timelineMsAtSourcePosition } from '../src/lib/mediaTime'
import { sourceAtPlayhead, sourceTimelineOccurrences } from '../src/panels/Timeline/layout'

const project = {
  tracks: [{
    id: 'v1',
    kind: 'video',
    clips: [
      { id: 'fast', asset: 'fast-source', src_in_ms: 1000, src_out_ms: 5000, speed: 2 },
      { id: 'reverse', asset: 'reverse-source', src_in_ms: 10_000, src_out_ms: 14_000, speed: 2, reverse: true },
      { id: 'freeze', asset: 'freeze-source', src_in_ms: 20_000, src_out_ms: 24_000, freeze: { at_ms: 750 } },
      { id: 'ramp', asset: 'ramp-source', src_in_ms: 30_000, src_out_ms: 35_000, speed_ramp: { points: [] } },
    ],
  }],
} as any

const ordinaryProject = {
  tracks: [{
    id: 'v1',
    kind: 'video',
    clips: [
      { id: 'ordinary-a', asset: 'ordinary-source-a', src_in_ms: 1200, src_out_ms: 2200 },
      { id: 'ordinary-b', asset: 'ordinary-source-b', src_in_ms: 5000, src_out_ms: 6000 },
    ],
  }],
} as any

assert.deepEqual(
  sourceAtPlayhead(ordinaryProject, 0),
  { asset: 'ordinary-source-a', srcMs: 1200 },
  'ordinary playback maps the first timeline instant to the inclusive source in-point',
)
assert.deepEqual(
  sourceAtPlayhead(ordinaryProject, 999),
  { asset: 'ordinary-source-a', srcMs: 2199 },
  'ordinary playback maps the last covered timeline instant inside the first clip',
)
assert.deepEqual(
  sourceAtPlayhead(ordinaryProject, 1000),
  { asset: 'ordinary-source-b', srcMs: 5000 },
  'clip coverage is half-open, so the shared boundary belongs to the next clip',
)

assert.equal(
  sourceMsAtTimelinePosition({ startMs: 0, srcInMs: 20_000, srcOutMs: 24_000, freezeAtMs: 750 }, 3999),
  20_750,
  'a freeze holds its source frame across the whole timeline slot',
)

const fastAtPlayhead = sourceAtPlayhead(project, 500)
assert.deepEqual(fastAtPlayhead, { asset: 'fast-source', srcMs: 2000 }, 'constant speed maps playhead time to source time')
assert.equal(
  timelineMsAtSourcePosition({ startMs: 0, srcInMs: 1000, srcOutMs: 5000, speed: 2 }, fastAtPlayhead?.srcMs ?? -1),
  500,
  'constant-speed source mapping round-trips through the shared media clock',
)

const reverseAtPlayhead = sourceAtPlayhead(project, 2500)
assert.deepEqual(reverseAtPlayhead, { asset: 'reverse-source', srcMs: 13_000 }, 'reverse playback maps the playhead from the high source edge')
assert.equal(
  timelineMsAtSourcePosition({ startMs: 2000, srcInMs: 10_000, srcOutMs: 14_000, speed: 2, reverse: true }, reverseAtPlayhead?.srcMs ?? -1),
  2500,
  'reverse source mapping round-trips through the shared media clock',
)

assert.deepEqual(sourceAtPlayhead(project, 4100), { asset: 'freeze-source', srcMs: 20_750 }, 'freeze maps to its selected source frame')
assert.deepEqual(sourceAtPlayhead(project, 7900), { asset: 'freeze-source', srcMs: 20_750 }, 'freeze remains at its selected source frame at the slot end')
assert.equal(sourceAtPlayhead(project, 8500), null, 'speed ramps fail closed without the engine-expanded segment model')
assert.equal(
  sourceAtPlayhead({
    ...project,
    tracks: [...project.tracks, {
      id: 'a1', kind: 'audio', clips: [{ id: 'audio-under-ramp', asset: 'audio-source', src_in_ms: 0, src_out_ms: 10_000 }],
    }],
  }, 8500),
  null,
  'an unsupported ramped video does not silently fall back to unrelated audio source time',
)

assert.deepEqual(
  sourceTimelineOccurrences(project, 'freeze-source', 20_750),
  [{ clipId: 'freeze', trackId: 'v1', atMs: 4000 }],
  'visual search navigates a frozen source frame to a deterministic occurrence',
)
assert.deepEqual(sourceTimelineOccurrences(project, 'freeze-source', 20_751), [], 'visual search does not invent unfrozen source frames')

console.log('PASS media-time source mapping')
