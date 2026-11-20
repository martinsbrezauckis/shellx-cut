import assert from 'node:assert/strict'
import { createPlayheadSeeks } from '../src/app/playheadSeeks'

// A ui.playhead command (or a ruler click) and a playback clock tick can land
// in the same render batch; the last state write wins. Observed: an explicit
// seek to 20000 ms followed 1 ms later by a clock publication of 41901 ms left
// the playhead at 41901 ms and the seek was lost. The publication carries the
// seek revision its clock last saw and must lose to the newer explicit seek.
const writes: number[] = []
let revision = 0
const seeks = createPlayheadSeeks((ms) => writes.push(ms), (r) => { revision = r })

seeks.seek(40000)
assert.equal(revision, 1)
assert.equal(seeks.playback(40118.4, 1), true, 'a publication from the current revision applies')
assert.deepEqual(writes, [40000, 40118])

const observedByClock = revision
seeks.seek(20000)
assert.equal(seeks.playback(41901, observedByClock), false, 'a publication from before the seek is dropped')
assert.equal(writes[writes.length - 1], 20000, 'the explicit seek is the last write of the batch')
assert.equal(revision, 2)

assert.equal(seeks.playback(20113, 2), true, 'the clock continues once it has seen the seek')
assert.equal(writes[writes.length - 1], 20113)
seeks.seek(-5)
assert.equal(writes[writes.length - 1], 0, 'positions are normalized to whole non-negative ms')
console.log('PASS explicit seeks take precedence over stale playback publications')
