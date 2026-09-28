import assert from 'node:assert/strict'
import type { ScreenRecordLiveFrameResult } from '../src/lib/clientResults'
import { acceptRecordingLiveFrame, awaitingRecordingLiveFrame, recordingLiveFramePresentation } from '../src/panels/Record/recordingLiveFramePresentation'

const ready: ScreenRecordLiveFrameResult = {
  capture_id: 'capture-A', state: 'ready', reason: null, generation: 4,
  frame_age_ms: 20, last_sample_cost_ms: 3, max_sample_cost_ms: 30,
  recursion: 'none', controller_exclusion: 'not_applicable',
  frame: { mime: 'image/bmp', bytes: 14, generation: 4, captured_at_ms: 100, base64: 'Qk0A' },
}

const shown = recordingLiveFramePresentation('capture-A', ready)
assert.equal(shown.frameUrl, 'data:image/bmp;base64,Qk0A')
assert.equal(recordingLiveFramePresentation('capture-B', ready).frameUrl, null, 'a different capture cannot supply pixels')
assert.equal(recordingLiveFramePresentation('capture-A', { ...ready, state: 'stale' }).frameUrl, null, 'retained stale bytes stay hidden')
assert.equal(recordingLiveFramePresentation('capture-A', { ...ready, frame_age_ms: 5_001 }).frameUrl, null, 'old ready bytes stay hidden')
assert.equal(recordingLiveFramePresentation('capture-A', { ...ready, frame: { ...ready.frame!, generation: 3 } }).frameUrl, null, 'wrong generation stays hidden')
assert.equal(recordingLiveFramePresentation('capture-A', { ...ready, frame: null }).frameUrl, null)
assert.equal(recordingLiveFramePresentation('capture-A', { ...ready, state: 'terminal', reason: 'Capture ended.' }).detail, 'Capture ended.')

const olderGeneration = recordingLiveFramePresentation('capture-A', { ...ready, generation: 3, frame: { ...ready.frame!, generation: 3 } })
assert.equal(acceptRecordingLiveFrame(shown, olderGeneration), shown, 'late generation cannot replace current pixels')
const olderFrame = recordingLiveFramePresentation('capture-A', { ...ready, frame: { ...ready.frame!, captured_at_ms: 99 } })
assert.equal(acceptRecordingLiveFrame(shown, olderFrame), shown, 'older frame in the same generation cannot replace current pixels')
const rollover = recordingLiveFramePresentation('capture-A', { ...ready, state: 'awaiting_frame', generation: 5, frame: null })
assert.equal(acceptRecordingLiveFrame(shown, rollover).frameUrl, null, 'physical rollover clears prior pixels')
assert.equal(acceptRecordingLiveFrame(shown, awaitingRecordingLiveFrame('capture-B')).frameUrl, null, 'new capture starts blank')

console.log('record live frame presentation: passed')
