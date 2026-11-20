import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RecordingVideoTimerControl } from '../src/panels/Record/RecordingVideoTimerControl'
import {
  RECORDING_SCENE_PRESETS,
  recordingSceneAcknowledged,
  recordingSceneTimerAcknowledged,
  recordingSceneCapability,
  recordingSceneStartConfig,
  recordingSceneTimerLabel,
  recordingTimerClock,
  recordingTimerDraft,
  parseRecordingTimerDuration,
  sceneUsesCamera,
  resetCameraLayoutForRecordingScene,
  studioStateForRecordingScene,
} from '../src/panels/Record/recordingScenes'
import { defaultStudioState } from '../src/panels/Record/studioTypes'

;(globalThis as typeof globalThis & { React?: { createElement: typeof createElement } }).React = { createElement }

assert.ok(RECORDING_SCENE_PRESETS.every((scene) => Number.isSafeInteger(scene.preset_revision) && scene.preset_revision > 0),
  'scene presets carry non-zero numeric PresetRevision values compatible with recorder-core')
assert.equal(RECORDING_SCENE_PRESETS[0].id, 'screen', 'screen-only remains the novice-safe default')
assert.deepEqual(RECORDING_SCENE_PRESETS[0].layout, { kind: 'screen' }, 'screen layout uses the public tagged DTO')
assert.deepEqual(RECORDING_SCENE_PRESETS.find((scene) => scene.id === 'presenter-corner')?.layout, {
  kind: 'presenter_pip', corner: 'bottom_right', size_percent: 22, shape: 'circle',
}, 'presenter layout uses only the engine-supported tagged PiP form')
assert.equal(RECORDING_SCENE_PRESETS.filter(sceneUsesCamera).length, 2,
  'only the two explicitly named presenter layouts require a camera')

const config = recordingSceneStartConfig(8, 'presenter-corner', { kind: 'countdown', duration_ms: 300_000 })
assert.equal(config.catalog_revision, 8, 'Start preserves Doctor’s numeric non-zero CatalogRevision')
assert.equal(config.initial_scene_id, 'presenter-corner')
assert.deepEqual(config.timer, { kind: 'countdown', duration_ms: 300_000 },
  'the timer is one capture-wide public DTO, not a per-scene claim')
assert.ok(config.presets.every((scene) => !('timer' in scene) && !('background' in scene)),
  'scene payloads omit unjournaled local preview background/timer metadata')
assert.deepEqual(recordingSceneCapability({ supported: true, catalog_revision: 8 }), {
  supported: true,
  catalog_revision: 8,
  detail: 'Live scene switches are available for this recording.',
}, 'only explicit numeric Doctor capability enables the proposed scene route')
assert.equal(recordingSceneCapability({ supported: true, catalog_revision: '8' }).supported, false,
  'string fingerprints never masquerade as recorder revisions')
assert.match(recordingSceneTimerLabel({ kind: 'elapsed' }), /begin when the recording starts/)
assert.match(recordingSceneTimerLabel({ kind: 'countdown', duration_ms: 300_000 }), /will not stop recording/,
  'a configured countdown never promises to stop a capture')
assert.deepEqual(parseRecordingTimerDuration('00:00:01'), { duration_ms: 1_000, error: null })
assert.deepEqual(parseRecordingTimerDuration('23:59:59'), { duration_ms: 86_399_000, error: null })
assert.deepEqual(parseRecordingTimerDuration('05:00'), { duration_ms: 300_000, error: null })
for (const invalid of ['00:00', '24:00:00', '00:60', '01:60', '5:00', 'abc', '00:00:00']) {
  assert.ok(parseRecordingTimerDuration(invalid).error, `${invalid} cannot replace a valid countdown`)
  assert.equal(recordingTimerDraft('countdown', invalid).timer, null,
    `an invalid ${invalid} draft cannot reach the capture configuration`)
}
assert.equal(recordingTimerClock(300_000), '05:00')
assert.equal(recordingTimerClock(3_600_000), '01:00:00')
assert.deepEqual(recordingTimerDraft('countdown', '10:00'), {
  timer: { kind: 'countdown', duration_ms: 600_000 }, error: null,
})
assert.deepEqual(recordingTimerDraft('off', 'bad input'), { timer: { kind: 'off' }, error: null },
  'the duration field is irrelevant when the selected mode is off')
const startAck = {
  saved: true,
  scene_id: 'screen',
  preset_revision: 1,
  logical_media_time_ms: 0,
  scene: { active_scene_id: 'screen', composition: { kind: 'screen' }, timer: { kind: 'elapsed' } },
}
assert.equal(recordingSceneAcknowledged(startAck, RECORDING_SCENE_PRESETS[0]), true,
  'only an exact durable start acknowledgement permits saved scene copy')
assert.equal(recordingSceneAcknowledged({ ...startAck, saved: false }, RECORDING_SCENE_PRESETS[0]), false,
  'a Start success envelope without saved acknowledgement remains unconfirmed')
assert.equal(recordingSceneTimerAcknowledged({
  action: 'pause', state: 'paused', logical_media_time_ms: 40,
  scene: { active_scene_id: 'screen', composition: { kind: 'screen' }, timer: { kind: 'elapsed' } },
}, 'pause'), true, 'a timer update needs the matching action, state, logical time, and projection')
for (const action of ['pause', 'resume', 'reset', 'restart', 'end'] as const) {
  for (const state of ['running', 'paused', 'ended'] as const) {
    const expectedState = action === 'pause' ? 'paused' : action === 'end' ? 'ended' : 'running'
    assert.equal(recordingSceneTimerAcknowledged({
      action, state, logical_media_time_ms: 40,
      scene: { active_scene_id: 'screen', composition: { kind: 'screen' }, timer: { kind: 'elapsed' } },
    }, action), state === expectedState, `${action} must reject a contradictory ${state} acknowledgement`)
  }
}
const customBackgroundStudio = { ...defaultStudioState(), background: 'solid' as const }
assert.equal(studioStateForRecordingScene(RECORDING_SCENE_PRESETS[0], customBackgroundStudio).background, 'solid',
  'selecting Screen only preserves independent Studio background choices')
const manualCameraStudio = {
  ...defaultStudioState(),
  camera: { ...defaultStudioState().camera, position: 'top_left' as const, x: 0.04, y: 0.04, size: 0.4, shape: 'rounded_rect' as const },
}
const demoScene = RECORDING_SCENE_PRESETS.find((scene) => scene.id === 'demo-corner')!
const switchedWithManualLayout = studioStateForRecordingScene(demoScene, manualCameraStudio, true)
assert.equal(switchedWithManualLayout.camera.position, 'top_left', 'a chosen corner survives a scene switch')
assert.equal(switchedWithManualLayout.camera.size, 0.4, 'a chosen size survives a scene switch')
assert.equal(switchedWithManualLayout.camera.shape, 'rounded_rect', 'a chosen shape survives a scene switch')
assert.equal(switchedWithManualLayout.camera.visible, true, 'the active scene still owns visibility')
const resetForDemo = resetCameraLayoutForRecordingScene(demoScene, switchedWithManualLayout)
assert.equal(resetForDemo.camera.position, 'top_right', 'Reset restores the active scene corner')
assert.equal(resetForDemo.camera.size, 0.18, 'Reset restores the active scene size')
assert.equal(resetForDemo.camera.visible, switchedWithManualLayout.camera.visible, 'Reset does not change visibility')

const timerControlProps = {
  status: { state: 'awaiting_ack' as const, message: 'Timer configured.' },
  recording: true, disabled: false, liveSupported: true,
  onTimerChange: () => {}, onTimerControl: () => {},
}
const offHtml = renderToStaticMarkup(createElement(RecordingVideoTimerControl, {
  ...timerControlProps, timer: { kind: 'off' },
}))
for (const action of ['pause', 'resume', 'reset', 'restart', 'end']) {
  assert.match(offHtml, new RegExp(`data-cut-rec-video-timer-action="${action}"[^>]*disabled=""`),
    `${action} cannot run while the video timer is off`)
}
assert.match(offHtml, /video timer is off/, 'the disabled timer explains why its actions are unavailable')
const idleHtml = renderToStaticMarkup(createElement(RecordingVideoTimerControl, {
  ...timerControlProps, recording: false, timer: { kind: 'elapsed' },
}))
assert.match(idleHtml, /data-cut-rec-video-timer-mode="countdown"/, 'countdown is selectable')
assert.match(idleHtml, /data-cut-rec-video-timer-apply/, 'the next-recording timer requires Apply')
const countdownHtml = renderToStaticMarkup(createElement(RecordingVideoTimerControl, {
  ...timerControlProps, recording: false, timer: { kind: 'countdown', duration_ms: 300_000 },
}))
for (const minutes of [1, 5, 10]) {
  assert.match(countdownHtml, new RegExp(`data-cut-rec-video-timer-preset="${minutes}"`),
    `${minutes}-minute suggestion is available without applying it`)
}
assert.match(countdownHtml, /data-cut-rec-video-timer-clock[^>]*value="05:00"/,
  'the editable clock reflects the committed duration')
assert.match(countdownHtml, /Changes take effect for the next recording after Apply or Enter/)
assert.doesNotMatch(idleHtml, /data-cut-rec-scene="(?:screen|presenter-corner|demo-corner)"/,
  'there are no duplicate scene preset cards')
const elapsedHtml = renderToStaticMarkup(createElement(RecordingVideoTimerControl, {
  ...timerControlProps, timer: { kind: 'elapsed' },
}))
assert.doesNotMatch(elapsedHtml, /data-cut-rec-video-timer-action="pause"[^>]*disabled=""/,
  'a configured live timer keeps Pause available')
for (const [state, disabledActions] of [
  ['awaiting_ack', ['resume']], ['running', ['resume']], ['paused', ['pause']], ['ended', ['pause', 'resume', 'end']],
] as const) {
  const html = renderToStaticMarkup(createElement(RecordingVideoTimerControl, {
    ...timerControlProps, timer: { kind: 'elapsed' }, status: { state, message: state },
  }))
  for (const action of ['pause', 'resume', 'reset', 'restart', 'end']) {
    assert.equal(new RegExp(`data-cut-rec-video-timer-action="${action}"[^>]*disabled=""`).test(html),
      (disabledActions as readonly string[]).includes(action), `${state} enables only valid ${action} transitions`)
  }
}

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const read = (path: string) => readFileSync(resolve(uiRoot, path), 'utf8')
const panel = read('src/panels/Record/index.tsx')
const control = read('src/panels/Record/RecordingVideoTimerControl.tsx')
const hook = read('src/panels/Record/useRecordingScenes.ts')
const preview = read('src/panels/Record/StudioPreview.tsx')
const client = read('src/lib/client.ts')

for (const selector of [
  'data-cut-rec-video-timer=',
  'data-cut-rec-video-timer-mode=',
  'data-cut-rec-video-timer-action=',
  'data-cut-rec-video-timer-state=',
]) assert.match(control, new RegExp(selector), `video timer keeps ${selector} inspectable`)
assert.match(panel, /data-cut-rec-scene-recovery="error"/, 'scene recovery remains reachable when it fails')
assert.match(preview, /data-cut-rec-scene-preview=/, 'the main Studio preview shows the current recording layout')
const session = read('src/app/useRecordingSession.ts')
assert.match(panel, /scenes: sceneStartConfig/, 'the panel saves the negotiated scene config in the validated preset')
assert.match(session, /args\.scenes = preset\.scenes/, 'the session sends that saved scene config to Start')
assert.match(panel, /setSceneCaptureId\(active\.captureId\)/, 'the live scene route receives the exact session capture id')
assert.match(panel, /markSceneCaptureStarted\(active\.startResult\.scenes\)/, 'start consumes the explicit initial-scene acknowledgement')
assert.match(hook, /screen_record\.scene_activate/, 'live selection uses the proposed bounded activation verb')
assert.match(hook, /preset_revision: scene\.preset_revision/, 'activation uses the exact numeric preset revision, not the catalog revision')
assert.match(hook, /recordingSceneAcknowledged\(response\.result, scene\)/, 'a live scene preview changes only after an exact saved acknowledgement')
assert.match(hook, /screen_record\.scene_timer/, 'live timer controls use their own bounded recorder verb')
assert.match(hook, /recordingSceneTimerAcknowledged\(timerResult, action\)/, 'a timer change requires a matching action acknowledgement and explicit state')
assert.match(control, /\['pause', 'resume', 'reset', 'restart', 'end'\]/, 'the control exposes all bounded live timer actions')
assert.match(hook, /screen_record\.recovery_status/, 'recovery state uses the existing read-only recorder inventory')
assert.match(client, /'screen_record\.scene_activate': RecordingSceneActivateArgs/, 'the shared UI client carries the narrow proposed activation interface')
assert.match(client, /'screen_record\.scene_timer': RecordingSceneTimerArgs/, 'the shared UI client carries bounded live timer actions')

console.log('PASS recording scenes UI contract, tagged layouts, numeric revisions, timer truth, and recovery states')
