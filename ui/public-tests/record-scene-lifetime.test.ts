import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { RecordingSceneControlLifetime } from '../src/panels/Record/recordingScenes'

const lifetime = new RecordingSceneControlLifetime()
lifetime.replaceCapture('capture-a')
const delayedSceneA = lifetime.begin('capture-a', 'scene')
assert.ok(delayedSceneA, 'scene activation A begins with capture A ownership')

// A new live recording starts before the response for A returns. The old
// response must not alter selection, preview composition, scene status, or a
// timer state owned by B.
lifetime.replaceCapture('capture-b')
const timerB = lifetime.begin('capture-b', 'timer')
assert.ok(timerB, 'timer control B begins with its own capture generation')
assert.equal(lifetime.isCurrent(delayedSceneA), false, 'delayed scene A is stale after B starts')
assert.equal(lifetime.isCurrent(timerB), true, 'B remains eligible to write its own acknowledged timer state')

lifetime.clearCapture()
assert.equal(lifetime.isCurrent(timerB), false, 'stopping the capture invalidates a delayed timer B response')
lifetime.replaceCapture('capture-c')
const sceneC = lifetime.begin('capture-c', 'scene')
const timerC = lifetime.begin('capture-c', 'timer')
assert.ok(sceneC)
assert.ok(timerC)
assert.equal(lifetime.isCurrent(sceneC), true, 'a timer request does not invalidate an independent scene acknowledgement')
assert.equal(lifetime.isCurrent(timerC), true, 'a scene request does not invalidate an independent timer acknowledgement')
lifetime.unmount()
assert.equal(lifetime.isCurrent(sceneC), false, 'unmount invalidates delayed live-scene responses')

const hook = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/useRecordingScenes.ts', import.meta.url)),
  'utf8',
)
assert.match(hook, /useLayoutEffect\(\(\) => \{[\s\S]*?if \(recording && captureId\) controlLifetimeRef\.current\.replaceCapture\(captureId\)[\s\S]*?else controlLifetimeRef\.current\.clearCapture\(\)/,
  'commit-time capture replacement and stop invalidate the scene-control generation before a queued response can write')
assert.match(hook, /screen_record\.scene_activate[\s\S]*?isCurrent\(controlLease\)[\s\S]*?setSelectedSceneId\(scene\.id\)[\s\S]*?onPreviewScene\(scene\.id\)/,
  'scene activation proves its exact live lease before selection and preview updates')
assert.match(hook, /screen_record\.scene_timer[\s\S]*?isCurrent\(controlLease\)[\s\S]*?setTimerStatus/,
  'timer acknowledgements prove the same lease before state writes')
assert.match(hook, /return \(\) => lifetime\.unmount\(\)/,
  'unmount revokes all live scene and timer requests')

console.log('PASS recording scene/timer capture lifetime rejects delayed A after capture B')
