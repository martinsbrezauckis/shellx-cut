import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import {
  recordingSourcePreviewCapability,
  recordingSourcePreviewPresentation,
  recordingSourcePreviewTarget,
} from '../src/panels/Record/recordingNativeSourcePreview'
import type {
  ScreenRecordSourcePreviewFrame,
  ScreenRecordSourcePreviewStatus,
} from '../src/lib/clientResults'

const ready: ScreenRecordSourcePreviewStatus = {
  state: 'ready', recursion: 'none', has_frame: true, generation: 7,
}
const frame: ScreenRecordSourcePreviewFrame = {
  mime: 'image/bmp', bytes: 3, generation: 7, captured_at_ms: 42, base64: 'Qk0B',
}
const exactCapability = { state: 'available', source_selection: 'exact' } as const
const portalCapability = { state: 'available', source_selection: 'portal' } as const
const present = recordingSourcePreviewPresentation(exactCapability, ready, frame)
assert.equal(present.frameUrl, 'data:image/bmp;base64,Qk0B', 'only an admitted current BMP becomes an image')
assert.equal(present.detail, 'Receiving a native source frame.')

const stale = recordingSourcePreviewPresentation(exactCapability, ready, { ...frame, generation: 6 })
assert.equal(stale.frameUrl, null, 'a frame from a replaced source generation never renders')

const unavailable = recordingSourcePreviewPresentation(
  { state: 'unsupported', prerequisite: 'capture-linux portal owner is absent' },
  { ...ready, state: 'idle', has_frame: false, generation: null },
  null,
)
assert.equal(unavailable.available, false)
assert.equal(unavailable.frameUrl, null)
assert.match(unavailable.detail, /portal owner/i)

assert.deepEqual(recordingSourcePreviewCapability(exactCapability), exactCapability,
  'only a server-advertised exact selection mode permits opaque monitor/window preview')
assert.deepEqual(recordingSourcePreviewCapability(portalCapability), portalCapability,
  'only a server-advertised portal selection mode permits a system-picker preview')
assert.equal(recordingSourcePreviewCapability({ state: 'available' }).state, 'unsupported',
  'an older availability-only envelope cannot gain a guessed source mode')

const monitors = [
  { id: 'opaque-primary', index: 1, name: 'Primary', width: 1920, height: 1080, primary: true },
  { id: 'opaque-secondary', index: 2, name: 'Secondary', width: 1920, height: 1080, primary: false },
]
const windows = [{ id: 'opaque-window', title: 'Editor', app: 'ShellX Cut' }]
assert.deepEqual(
  recordingSourcePreviewTarget(exactCapability, 'display', monitors, 2, windows, null)?.source,
  { kind: 'monitor', monitor_id: 'opaque-secondary' },
  'an explicit current monitor index resolves to its exact opaque identity',
)
assert.deepEqual(
  recordingSourcePreviewTarget(exactCapability, 'display', monitors, null, windows, null)?.source,
  { kind: 'monitor', monitor_id: 'opaque-primary' },
  'no explicit index resolves only to the current primary/first opaque monitor identity',
)
assert.deepEqual(
  recordingSourcePreviewTarget(exactCapability, 'window', monitors, null, windows, 'opaque-window')?.source,
  { kind: 'window', window_id: 'opaque-window' },
  'a window preview keeps the exact current opaque window id',
)
assert.equal(recordingSourcePreviewTarget(exactCapability, 'window', monitors, null, windows, 'missing'), null,
  'a vanished exact window never falls back to a display preview')
assert.deepEqual(
  recordingSourcePreviewTarget(portalCapability, 'window', monitors, null, windows, null)?.source,
  { kind: 'portal' },
  'only the portal capability admits the system-picker source form',
)

const component = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/RecordingSourcePreview.tsx', import.meta.url)),
  'utf8',
)
assert.match(component, /data-cut-record-source-preview/, 'the mounted lifecycle control remains inspectable')
assert.match(component, /data-cut-action="record-source-preview"/, 'Preview is an explicit user action')
for (const action of [
  'record-source-preview-pause',
  'record-source-preview-resume',
  'record-source-preview-hide',
  'record-source-preview-stop',
]) {
  assert.match(component, new RegExp(`data-cut-action="${action}"`), `${action} has a stable lifecycle action id`)
}
assert.match(component, /onClick=\{onStart\}/, 'start delegates the current source choice to its mounted owner')
assert.match(component, /onClick=\{onPause\}.*onClick=\{onResume\}.*onClick=\{onHide\}.*onClick=\{onStop\}/s,
  'all lifecycle controls delegate rather than mutate locally')
assert.match(component, /const ownsPreview = active \|\| paused[\s\S]*onClick=\{onHide\} disabled=\{!ownsPreview \|\| busy\}[\s\S]*onClick=\{onStop\} disabled=\{!ownsPreview \|\| busy\}/,
  'lease-only controls are unavailable when there is no native preview to release')
assert.doesNotMatch(component, /<select|onSelectSource|sources:/, 'Screen & sound remains the only source selector')
assert.doesNotMatch(component, /getDisplayMedia|canvas\.toDataURL/, 'the UI has no browser/synthetic capture fallback')

const hook = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/useRecordingSourcePreview.ts', import.meta.url)),
  'utf8',
)
const studio = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/StudioPreview.tsx', import.meta.url)),
  'utf8',
)
const sourceControl = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/RecordingSourceControl.tsx', import.meta.url)),
  'utf8',
)
const record = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/index.tsx', import.meta.url)),
  'utf8',
)
const sourceSetup = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/RecordingSourceSetup.tsx', import.meta.url)),
  'utf8',
)
assert.match(hook, /screen_record\.preview_capability/, 'the hook receives source mode from the recorder capability')
assert.match(hook, /screen_record\.preview_frame/, 'only the native mailbox projection is polled')
assert.match(hook, /commandEpochRef.*sessionEpochRef/s, 'command and frame sessions have independent stale-response guards')
assert.match(hook, /targetKeyRef\.current !== targetKey/, 'a replaced selected source invalidates a late preview start')
assert.match(hook, /recording && .*void stop\(\)/s, 'record admission stops the owned preview')
assert.match(hook, /document\.visibilityState === 'hidden'/,
  'document visibility loss is observed explicitly')
assert.match(hook, /document\.addEventListener\('visibilitychange', onVisibilityChange\)/,
  'visibility loss is wired to the document lifecycle')
assert.match(hook, /if \(document\.visibilityState === 'hidden'\) void hide\(\)/,
  'document visibility loss hides and releases the native preview')
assert.match(hook, /screen_record\.preview_stop/, 'replacement and unmount release the native source lease')
assert.doesNotMatch(hook, /getDisplayMedia|navigator\.(platform|userAgent)|canvas\.toDataURL/,
  'the hook has no browser/OS inference or synthetic fallback')
assert.match(studio, /sourcePreview\.frameUrl/, 'the admitted BMP is rendered in the existing Studio screen plane')
assert.match(studio, /data-cut-rec-native-preview-frame/, 'the native image has a stable inspection selector')
assert.match(studio, /rec-studio-preview__camera/, 'camera composition remains over the source image')
assert.doesNotMatch(sourceControl, /navigator\.(platform|userAgent)/,
  'Screen & sound does not infer a preview backend from the browser platform')
assert.match(record, /useRecordingSourcePreview\(/, 'Record mounts the bounded native preview owner')
assert.match(record, /<RecordingSourceSetup/, 'Record mounts the single source setup field')
assert.match(sourceSetup, /<RecordingSourceControl[\s\S]*<RecordingSourcePreview/s,
  'source setup keeps the only selector and preview controls adjacent')
assert.match(sourceSetup, /onStart=\{\(\) => \{ void preview\.start\(\) \}\}/,
  'the extracted field delegates preview lifecycle to the existing owner')

const schema = readFileSync(
  fileURLToPath(new URL('../../schema/verbs/fragments/064-screen_record_preview.json', import.meta.url)),
  'utf8',
)
assert.match(schema, /mime:'image\/bmp'/, 'the public contract permits BMP only')
assert.match(schema, /source_selection:'exact'\|'portal'/, 'capability advertises exact versus portal source selection')
assert.match(schema, /"agent_chat": "deny"/, 'live pixels remain outside agent-chat exposure')
assert.doesNotMatch(schema, /"process": true/, 'preview never owns an encoder child process')
