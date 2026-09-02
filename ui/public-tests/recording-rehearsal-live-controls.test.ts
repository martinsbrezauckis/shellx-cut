import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { recordingRehearsalConfigKey } from '../src/panels/Record/RecordingRehearsal'

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const read = (path: string) => readFileSync(resolve(uiRoot, path), 'utf8')

const record = read('src/panels/Record/index.tsx')
const rehearsal = read('src/panels/Record/RecordingRehearsal.tsx')
const live = read('src/panels/Record/RecordingLiveControls.tsx')
const captureSafety = read('src/panels/Record/RecordingCaptureSafetyStatus.tsx')
const pause = read('src/panels/Record/RecordingPauseControl.tsx')
const scenes = read('src/panels/Record/RecordingScenesControl.tsx')
const css = read('src/panels/Record/record.css')

const baseRehearsal = {
  sourceKind: 'display' as const,
  fps: 30,
  monitor: 1,
  monitorId: 'opaque-display-a',
  windowId: null,
}
assert.notEqual(
  recordingRehearsalConfigKey(baseRehearsal),
  recordingRehearsalConfigKey({ ...baseRehearsal, fps: 60 }),
  'a different fps cannot reuse a prior rehearsal take',
)
assert.notEqual(
  recordingRehearsalConfigKey(baseRehearsal),
  recordingRehearsalConfigKey({ ...baseRehearsal, monitorId: 'opaque-display-b' }),
  'a changed selected opaque monitor cannot reuse a prior rehearsal take',
)
assert.notEqual(
  recordingRehearsalConfigKey(baseRehearsal),
  recordingRehearsalConfigKey({ ...baseRehearsal, sourceKind: 'window', windowId: 'opaque-window' }),
  'a changed source kind/window identity cannot reuse a prior rehearsal take',
)

assert.match(rehearsal, /screen_record\.rehearsal_start/,
  'rehearsal uses its dedicated bounded native take rather than normal recording')
assert.match(rehearsal, /duration_ms: 3_000/,
  'the beginner-facing rehearsal asks for the three-second bounded default')
assert.match(rehearsal, /screen_record\.rehearsal_discard/,
  'the UI explicitly discards temporary playback on replacement and unmount')
assert.match(rehearsal, /data-cut-action="record-rehearse"/,
  'the disposable native rehearsal action has a stable id')
assert.match(rehearsal, /data-cut-action="record-rehearsal-discard"/,
  'the temporary take discard action has a stable id')
assert.match(rehearsal, /requestId\.current \+= 1[\s\S]*void discard\(\)/,
  'unmount invalidates a late request and clears the temporary take')
assert.match(rehearsal, /recordingRehearsalConfigKey\(/,
  'every take is fingerprinted to its exact source and fps configuration')
assert.match(rehearsal, /configKeyRef\.current === configKey[\s\S]*requestId\.current \+= 1[\s\S]*setState\(\{ kind: 'idle' \}\)/,
  'source/fps replacement invalidates both running and ready visible take state')
assert.match(rehearsal, /if \(id !== requestId\.current\) return[\s\S]*setState\(\{ kind: 'running' \}\)/,
  'a config change during cleanup cannot launch a stale native rehearsal request')
assert.match(rehearsal, /void discard\(take\.playback_handle\)/,
  'a late handle is discarded through the same ownership-aware cleanup path')
assert.match(rehearsal, /data-cut-rec-rehearsal-disposable/,
  'the visible contract says the test creates no project recording or media')
assert.match(rehearsal, /data-cut-rec-rehearsal-video/,
  'a completed native take has an immediate playable video surface')
assert.match(rehearsal, /never claims audio that it did not capture/,
  'the video-only rehearsal has an explicit audio boundary')

assert.match(record, /<RecordingRehearsal[\s\S]*startError=\{preflightStartError\(\)\}/,
  'rehearsal receives the same visible start preflight rather than a parallel readiness rule')
assert.match(record, /<RecordingLiveControls[\s\S]*onMarker=\{addRecordingMarker\}[\s\S]*onStop=\{stop\}/,
  'the active HUD owns the same real marker and stop mutations')
assert.match(record, /screen_record\.studio_event/,
  'the visible marker appends to the existing Studio journal')
assert.match(record, /recordingMarkerAcknowledged\(response\.result, label\)/,
  'a marker is announced only after the journal echoes its accepted event')
assert.match(record, /matchesFixedAction\(e, 'recording\.marker'\)/,
  'F12 remains routed through the same marker action as the HUD')
assert.match(record, /sceneControl=\{phase === 'recording' \? undefined : recordingSceneControl\}/,
  'live scene controls move from setup to the HUD instead of duplicating')

assert.match(live, /data-cut-rec-live-controls/,
  'the compact active surface has a stable inspection root')
assert.match(live, /data-cut-action="record-marker"/,
  'the marker button has a stable actionable identity')
assert.match(live, /<RecordingPauseControl[\s\S]*mode="live"/,
  'pause uses the existing acknowledged control in its live-only form')
assert.match(live, /data-cut-rec-live-pause-unavailable/,
  'unadmitted pause has an explicit reason instead of a dead action')
assert.match(live, /data-cut-action="record-stop"/,
  'stop remains a visible live control')
assert.match(live, /captureSafety/, 'the compact live HUD has one capture-owned safety status slot')
assert.match(captureSafety, /data-cut-rec-controller-placement=/,
  'controller placement remains a bounded, inspectable observed status')
assert.match(scenes, /compact\?: boolean/,
  'the same scene component can be compact in the active HUD')
assert.match(scenes, /!compact &&/, 'recovery controls are not duplicated in the active recording HUD')
assert.match(pause, /mode === 'live'/, 'the paused/resume action has one dedicated live rendering mode')
assert.match(css, /\.rec-live-controls/, 'live controls have a compact dedicated layout')
assert.match(css, /\.rec-scenes--compact/, 'scene controls compact in the active HUD')
assert.match(css, /\.rec-rehearsal__playback video/, 'temporary playback is constrained in the Record layout')

console.log('PASS recorder rehearsal truth boundary and live-controls wiring')
