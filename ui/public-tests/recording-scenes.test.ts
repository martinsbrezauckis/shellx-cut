import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  RECORDING_SCENE_PRESETS,
  recordingSceneAcknowledged,
  recordingSceneTimerAcknowledged,
  recordingSceneCapability,
  recordingSceneStartConfig,
  recordingSceneTimerLabel,
  sceneUsesCamera,
  studioStateForRecordingScene,
} from '../src/panels/Record/recordingScenes'
import { defaultStudioState } from '../src/panels/Record/studioTypes'

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
const customBackgroundStudio = { ...defaultStudioState(), background: 'solid' as const }
assert.equal(studioStateForRecordingScene(RECORDING_SCENE_PRESETS[0], customBackgroundStudio).background, 'solid',
  'selecting Screen only preserves independent Studio background choices')

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const read = (path: string) => readFileSync(resolve(uiRoot, path), 'utf8')
const panel = read('src/panels/Record/index.tsx')
const control = read('src/panels/Record/RecordingScenesControl.tsx')
const hook = read('src/panels/Record/useRecordingScenes.ts')
const preview = read('src/panels/Record/StudioPreview.tsx')
const client = read('src/lib/client.ts')

for (const selector of [
  'data-cut-rec-scenes',
  'data-cut-rec-scene=',
  'data-cut-rec-scene-status=',
  'data-cut-rec-scene-recovery=',
  'data-cut-rec-scene-timer-choice=',
  'data-cut-rec-scene-timer-action=',
  'data-cut-rec-scene-timer-state=',
]) assert.match(control, new RegExp(selector), `scene control keeps ${selector} stable for UI inspection`)
assert.match(preview, /data-cut-rec-scene-preview=/, 'the main Studio preview names the selected scene')
assert.match(panel, /startArgs\.scenes = sceneStartConfig/, 'scene config joins Start only through the negotiated start payload')
assert.match(panel, /setSceneCaptureId\(res\.capture_id\)/, 'the live scene route receives the exact capture id')
assert.match(panel, /markSceneCaptureStarted\(res\.scenes\)/, 'start consumes the explicit initial-scene acknowledgement')
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
