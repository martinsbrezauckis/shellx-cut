import assert from 'node:assert/strict'
import { shouldPublishPlaybackTick, playbackBindingKey } from '../src/panels/Preview/playbackClock'
import { activeVideo } from '../src/panels/Preview/model'
import { PlaybackHandoff, PlaybackSeekTracker, acceptPlaybackFrame, canStartPreviewMedia, normalizePlayheadMs, forwardPlaybackPosition, playbackVideoKey } from '../src/panels/Preview/playbackClock'
import { resolveOverlays } from '../src/panels/Preview/composite'
import { previewPlaybackClockPosition } from '../src/panels/Preview/useVoiceoverPlayback'
const clip = (id: string, a: number, b: number, speed = 1) => ({ id, asset: 'a', src_in_ms: a, src_out_ms: b, speed })
const project: any = { assets: { a: { proxy: '/a.mp4' } }, tracks: [
  { id: 'v1', kind: 'video', clips: [clip('first', 0, 30000), {kind:'gap', duration_ms:30000}, clip('last', 60000, 90000)] },
  { id: 'v2', kind: 'video', clips: [{kind:'gap', duration_ms:30000}, clip('middle', 30000, 60000)] },
] }
assert.equal(activeVideo(project, 30000, new Set()), null)
assert.equal(resolveOverlays(project, 45000, 'v1', new Set()).overlays[0]?.clipId, 'middle')
const first = activeVideo(project, 29999, new Set())!
const last = activeVideo(project, 60000, new Set())!
assert.equal(playbackBindingKey(first, 0), playbackBindingKey({ ...first, clipId: 'piece', startMs: first.startMs + first.durMs, srcInMs: first.srcOutMs }, 0), 'continuous pieces share a clock binding')
assert.notEqual(playbackBindingKey(first, 0), playbackBindingKey(first, 1), 'a deck swap rebinds the clock')
assert.notEqual(playbackBindingKey(first, 0), playbackBindingKey({ ...first, srcInMs: first.srcInMs + 200 }, 0), 'a source jump rebinds the clock')
assert.equal(playbackBindingKey(null, null), null, 'a gap clears the clock binding')
assert.notEqual(playbackBindingKey({ ...first, speed: 2 }, 0), playbackBindingKey({ ...first, speed: 2, clipId: 'piece' }, 0), 'other speeds preserve placement boundaries')
const key = playbackVideoKey(last)
const frame = (video: typeof last | null, media: number, element: number, ready = true, seeking = false) => acceptPlaybackFrame(key, video, media, element, ready, seeking, 1000/24)
assert.equal(frame(last, 0, 0, false), false, 'initial callback cannot run before seek readiness')
assert.equal(frame(last, 0, 60000), false, 'queued pre-seek callback rejected after seek')
assert.equal(frame(last, 60000, 60000, true, true), false, 'in-flight seek rejected')
assert.equal(frame(last, 60000, 60000), true)
assert.equal(frame(first, 60000, 60000), false, 'old callback cannot use new clip mapping')
assert.equal(frame(last, 90000, 60000), false, 'stale callback after backward user seek rejected')
assert.equal(frame(null, 60000, 60000), false)
const adjacent: any = {...project, tracks:[{id:'v1',kind:'video',clips:[clip('a',0,30000),clip('b',30000,60000)]}]}
const a = activeVideo(adjacent, 29999, new Set())!
const b = activeVideo(adjacent, 30000, new Set())!
assert.notEqual(playbackVideoKey(a), playbackVideoKey(b), 'same source cuts rearm epoch')
assert.equal(acceptPlaybackFrame(playbackVideoKey(a), b, 30000, 30000, true, false, 42), false)
const faster = {...b, speed: 2}
assert.equal(acceptPlaybackFrame(playbackVideoKey(faster), faster, 30200, 30000, true, false, 42), true)
assert.equal(forwardPlaybackPosition(59960,60000,60000),60000, 'decoded frame before cut cannot cross back into gap')
assert.equal(forwardPlaybackPosition(60200,60300,60000),60300, 'out-of-order frames cannot rewind')
assert.equal(forwardPlaybackPosition(60500,60300,60000),60500)
const tracker = new PlaybackSeekTracker(1000)
tracker.published(1100); tracker.published(1200)
assert.equal(tracker.external(1100), false)
assert.equal(tracker.external(1200), false)
assert.equal(tracker.external(7000), true, 'same-clip user seek starts a new epoch')
tracker.published(7100)
assert.equal(tracker.external(7100), false)
assert.equal(tracker.external(500), true, 'backward external seek detected')
assert.equal(shouldPublishPlaybackTick(199, 100, 2399, 1, 2400), false, 'inside a gap the usual cadence is sufficient')
assert.equal(shouldPublishPlaybackTick(199, 100, 2401, 1, 2400), true, 'crossing the next clip start must publish before the next 100 ms tick')
assert.equal(shouldPublishPlaybackTick(199, 100, 2401, -1, 2400), false, 'reverse playback keeps its cadence')
assert.equal(shouldPublishPlaybackTick(200, 100, 2399, 1, null), true, 'ordinary publication keeps its cadence')
console.log('PASS preview playback clock guards')

// Boundary update and a same-clip user seek can be batched before React renders
// the next clip; the handoff must release even when the element has ended.
const handoffTracker = new PlaybackSeekTracker(29500)
const handoff = new PlaybackHandoff(handoffTracker)
assert.equal(handoff.begin(), true)
handoffTracker.published(30000)
assert.equal(handoff.observe(29500), 'wait')
assert.equal(handoff.observe(28000), 'seek')
assert.equal(handoff.observe(28000), 'continue')
assert.equal(handoff.begin(), true, 'a later boundary can hand off again')
assert.equal(handoff.begin(), false, 'duplicate ended/frame cannot hand off twice')

const explicitTracker = new PlaybackSeekTracker(29500, 4)
const explicitHandoff = new PlaybackHandoff(explicitTracker)
explicitHandoff.begin()
explicitTracker.published(30000)
assert.equal(explicitHandoff.observe(29500, 5), 'seek', 'same-position user seek cancels handoff')
explicitTracker.published(29600)
assert.equal(explicitHandoff.observe(29600, 6), 'seek', 'seek matching an unacknowledged clock update is still explicit')

// The tracker must use the same representation as App, even if a caller
// publishes a raw fractional media clock value instead of its usual clamp.
const fractional = new PlaybackSeekTracker(900)
for (const value of [1000.4, 1001.6, 1002.2, 1003.9]) {
  const sent = fractional.published(value)
  assert.equal(sent, normalizePlayheadMs(value))
  assert.equal(fractional.external(normalizePlayheadMs(sent)), false)
}
const boundaryTracker = new PlaybackSeekTracker(100)
const boundaryHandoff = new PlaybackHandoff(boundaryTracker)
boundaryHandoff.begin()
const boundary = boundaryTracker.published(200.49)
assert.equal(boundaryHandoff.observe(normalizePlayheadMs(boundary)), 'wait')
assert.equal(boundaryHandoff.observe(0, 1), 'seek')
assert.equal(boundaryTracker.published(-2.8), 0)
assert.equal(boundaryTracker.external(0, 1), false)
assert.equal(canStartPreviewMedia(0), false, 'pre-metadata seek must wait for canplay')
assert.equal(canStartPreviewMedia(1), false)
assert.equal(canStartPreviewMedia(2), true, 'canplay can retry deferred startup')

const productionTracker = new PlaybackSeekTracker(900)
for (const mediaTime of [1000.4, 1001.6, 1002.2, 1003.9, -4, 10000.4]) {
  const bounded = previewPlaybackClockPosition(mediaTime, 9000, null).positionMs
  const published = productionTracker.published(bounded)
  assert.equal(productionTracker.external(normalizePlayheadMs(published)), false)
}
assert.equal(previewPlaybackClockPosition(-4.2, 9000, null).positionMs, 0)
assert.equal(previewPlaybackClockPosition(9001.2, 9000, null).positionMs, 9000)

// ui.playhead can be accepted against an older observable snapshot while a
// playback publication of the same value is still pending. The command's
// explicit revision must make that value a seek, not a clock acknowledgment.
const debugTracker = new PlaybackSeekTracker(29000, 7)
const debugHandoff = new PlaybackHandoff(debugTracker)
debugHandoff.begin()
debugTracker.published(29500)
assert.equal(debugHandoff.observe(29500, 8), 'seek', 'debug seek matching pending clock value cancels handoff')
