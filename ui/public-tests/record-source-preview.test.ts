import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { defaultStudioState } from '../src/panels/Record/studioTypes'
import { createServer } from 'vite'
import {
  recordingSourcePreviewCapability,
  recordingSourcePreviewPresentation,
  recordingSourcePreviewTarget,
  type RecordingSourcePreviewPresentation,
} from '../src/panels/Record/recordingNativeSourcePreview'
import {
  recordingSourcePreviewControlArgs,
  recordingSourcePreviewFailedStartStatus,
  recordingSourcePreviewLease,
  recordingSourcePreviewLeaseMatches,
} from '../src/panels/Record/recordingSourcePreviewLease'
import type {
  ScreenRecordSourcePreviewFrame,
  ScreenRecordSourcePreviewStatus,
} from '../src/lib/clientResults'

const leaseNonce = '00112233445566778899aabbccddeeff'
const ready: ScreenRecordSourcePreviewStatus = {
  state: 'ready', recursion: 'none', has_frame: true, generation: 7, lease_nonce: leaseNonce,
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

assert.deepEqual(
  recordingSourcePreviewControlArgs(7, leaseNonce),
  { expected_generation: 7, expected_lease_nonce: leaseNonce },
  'a cleanup request carries the exact generation it observed',
)
assert.notDeepEqual(
  recordingSourcePreviewControlArgs(7, leaseNonce),
  recordingSourcePreviewControlArgs(8, leaseNonce),
  'a late cleanup for generation 7 cannot be encoded as a stop for a newer generation 8',
)
assert.notDeepEqual(
  recordingSourcePreviewControlArgs(7, leaseNonce),
  recordingSourcePreviewControlArgs(7, 'ffeeddccbbaa99887766554433221100'),
  'a delayed control from a restarted server cannot be encoded with the new owner nonce',
)
assert.equal(recordingSourcePreviewControlArgs(7, null), null,
  'an absent lease nonce cannot create a destructive preview control')
assert.equal(recordingSourcePreviewLeaseMatches(ready, 7, leaseNonce), true,
  'a poll result is accepted only for the exact lease that this panel acknowledged')
assert.equal(recordingSourcePreviewLeaseMatches(ready, 7, 'ffeeddccbbaa99887766554433221100'), false,
  'a same-generation poll response from a restarted owner is never adopted')
assert.equal(recordingSourcePreviewLeaseMatches({ ...ready, generation: 8 }, 7, leaseNonce), false,
  'a later-generation poll response is never adopted by the prior lease')
assert.equal(
  recordingSourcePreviewLease('monitor:opaque-monitor', {
    ...ready,
    state: 'unavailable',
    has_frame: false,
    generation: null,
  }),
  null,
  'a failed restart reconciled to unavailable cannot retain the old ready lease',
)
assert.deepEqual(
  recordingSourcePreviewFailedStartStatus({ ...ready, generation: 9 }),
  { state: 'unavailable', recursion: 'none', has_frame: false, generation: null },
  'a failed start strips the nonce because a non-active status cannot retain a lease pair',
)
assert.deepEqual(
  recordingSourcePreviewFailedStartStatus({
    ...ready,
    state: 'unavailable',
    has_frame: false,
    generation: null,
    unavailable_reason: 'Restart ShellX Cut before recording.',
  }),
  {
    state: 'unavailable',
    recursion: 'none',
    has_frame: false,
    generation: null,
    unavailable_reason: 'Restart ShellX Cut before recording.',
  },
  'a terminal failed-start status retains the process-restart reason',
)
const unresolvedRelease = recordingSourcePreviewPresentation(exactCapability, {
  ...ready,
  state: 'unavailable',
  has_frame: false,
  generation: null,
  unavailable_reason: 'Restart ShellX Cut before recording.',
}, null)
assert.equal(
  unresolvedRelease.detail,
  'Restart ShellX Cut before recording.',
  'a persistent native-release failure remains actionable in the visible preview state',
)

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
assert.doesNotMatch(component, /data-cut-action="record-source-preview"/, 'the lifecycle panel does not duplicate the Studio start action')
for (const action of [
  'record-source-preview-pause',
  'record-source-preview-resume',
  'record-source-preview-hide',
  'record-source-preview-stop',
]) {
  assert.match(component, new RegExp(`data-cut-action="${action}"`), `${action} has a stable lifecycle action id`)
}
assert.match(component, /onClick=\{onPause\}.*onClick=\{onResume\}.*onClick=\{onHide\}.*onClick=\{onStop\}/s,
  'all lifecycle controls delegate rather than mutate locally')
assert.match(component, /const ownsPreview = active \|\| paused[\s\S]*\{ownsPreview && <>[\s\S]*onClick=\{onHide\} disabled=\{busy\}[\s\S]*onClick=\{onStop\} disabled=\{busy\}/,
  'lease-only controls are shown only for an owned native preview and remain disabled while busy')
assert.doesNotMatch(component, /<select|onSelectSource|sources:/, 'Screen & sound remains the only source selector')
assert.doesNotMatch(component, /getDisplayMedia|canvas\.toDataURL/, 'the UI has no browser/synthetic capture fallback')

const hook = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/useRecordingSourcePreview.ts', import.meta.url)),
  'utf8',
)
const previewLifecycle = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/recordingSourcePreviewLifecycle.ts', import.meta.url)),
  'utf8',
)
const previewPoller = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/useRecordingSourcePreviewPoller.ts', import.meta.url)),
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
assert.match(previewPoller, /screen_record\.preview_frame/, 'only the native mailbox projection is polled')
assert.match(hook, /commandEpochRef/, 'the owner invalidates stale lifecycle commands')
assert.match(previewPoller, /sessionEpochRef/, 'the poller separately invalidates stale frame sessions')
assert.match(previewPoller, /recordingSourcePreviewLeaseMatches\([\s\S]*pollSession\.expectedGeneration[\s\S]*pollSession\.expectedLeaseNonce/s,
  'the poller verifies a frame response against its acknowledged generation-and-nonce pair before adoption')
assert.match(previewPoller, /const abandonReplacedPreview[\s\S]*setStatus\(\{ state: 'unavailable'[\s\S]*invalidatePoll\(\)[\s\S]*stopPolling\(\)/s,
  'a replaced preview clears local ownership without sending a destructive control to the foreign lease')
assert.match(previewPoller, /const abandonTerminalPreview[\s\S]*recordingSourcePreviewFailedStartStatus\(terminalStatus\)[\s\S]*invalidatePoll\(\)[\s\S]*stopPolling\(\)/s,
  'a generation-null terminal poll result preserves its truthful release-failure reason without cleanup')
assert.match(previewPoller, /if \(recordingSourcePreviewGeneration\(snapshot\.status\) === null\) \{[\s\S]*abandonTerminalPreview\(snapshot\.status\)/,
  'terminal status is separated from an active foreign lease before replacement refusal')
assert.match(hook, /targetKeyRef\.current !== targetKey/, 'a replaced selected source invalidates a late preview start')
assert.match(previewLifecycle, /recordingSourcePreviewControlArgs\(expectedGeneration, expectedLeaseNonce\)/, 'lifecycle actions bind their request to the observed native generation and owner nonce')
assert.match(hook, /reconcileFailedStart/, 'a failed restart reconciles the UI lease against the server lifecycle')
assert.match(hook, /recording && .*void stop\(\)/s, 'record admission stops the owned preview')
assert.match(hook, /document\.visibilityState === 'hidden'/,
  'document visibility loss is observed explicitly')
assert.match(hook, /document\.addEventListener\('visibilitychange', onVisibilityChange\)/,
  'visibility loss is wired to the document lifecycle')
assert.match(hook, /if \(document\.visibilityState === 'hidden'\) void hide\(\)/,
  'document visibility loss hides and releases the native preview')
assert.match(hook, /screen_record\.preview_stop/, 'replacement and unmount release the native source lease')
assert.doesNotMatch(`${hook}\n${previewLifecycle}\n${previewPoller}`, /getDisplayMedia|navigator\.(platform|userAgent)|canvas\.toDataURL/,
  'the hook has no browser/OS inference or synthetic fallback')
assert.match(studio, /sourcePreview\.frameUrl/, 'the admitted BMP is rendered in the existing Studio screen plane')
assert.match(studio, /data-cut-rec-native-preview-frame/, 'the native image has a stable inspection selector')
assert.match(studio, /rec-studio-preview__camera/, 'camera composition remains over the source image')
const studioProps = {
  studio: defaultStudioState(), phase: 'idle' as const, elapsed: '0:00', sceneName: 'Screen only',
  sceneState: 'ready', sourcePreviewCanStart: true, onSourcePreviewStart: () => {},
}
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true, entries: [] }, server: { middlewareMode: true }, appType: 'custom' })
const { StudioPreview } = await vite.ssrLoadModule('/src/panels/Record/StudioPreview.tsx') as typeof import('../src/panels/Record/StudioPreview')
const { RecordingSourcePreview } = await vite.ssrLoadModule('/src/panels/Record/RecordingSourcePreview.tsx') as typeof import('../src/panels/Record/RecordingSourcePreview')
const idlePreview = { available: true, state: 'idle' as const, detail: 'Choose one current source to begin preview.', frameUrl: null }
const renderStudio = (preview: RecordingSourcePreviewPresentation) => renderToStaticMarkup(createElement(StudioPreview, {
  ...studioProps, sourcePreview: preview,
}))
const idleHtml = renderStudio(idlePreview)
assert.match(idleHtml, /data-cut-rec-preview-unavailable[^>]*>[\s\S]*data-cut-action="record-source-preview"/, 'the one start action is inside the black preview')
assert.equal((idleHtml.match(/data-cut-action="record-source-preview"/g) ?? []).length, 1)
assert.match(idleHtml, />Preview source<\/button>/, 'the preview action uses the approved label')
assert.doesNotMatch(renderStudio({ ...idlePreview, state: 'ready', frameUrl: 'data:image/bmp;base64,Qk0B' }), /data-cut-action="record-source-preview"/, 'real native pixels replace the idle action')
assert.doesNotMatch(renderStudio({ ...idlePreview, state: 'paused' }), /data-cut-action="record-source-preview"/, 'paused preview retains its Resume route without an extra start action')
const lifecycleHtml = renderToStaticMarkup(createElement(RecordingSourcePreview, {
  preview: idlePreview, status: { ...ready, state: 'paused', has_frame: false }, target: null,
  statusError: null, onPause: () => {}, onResume: () => {}, onHide: () => {}, onStop: () => {},
}))
assert.match(lifecycleHtml, /data-cut-action="record-source-preview-resume"/)
assert.match(lifecycleHtml, /data-cut-action="record-source-preview-hide"/)
assert.match(lifecycleHtml, /data-cut-action="record-source-preview-stop"/)
assert.doesNotMatch(lifecycleHtml, /data-cut-action="record-source-preview"/)
await vite.close()
assert.doesNotMatch(sourceControl, /navigator\.(platform|userAgent)/,
  'Screen & sound does not infer a preview backend from the browser platform')
assert.match(record, /useRecordingSourcePreview\(/, 'Record mounts the bounded native preview owner')
assert.match(record, /<RecordingSourceSetup/, 'Record mounts the single source setup field')
assert.match(sourceSetup, /<RecordingSourceControl/, 'Screen & sound keeps the only source selector')
assert.doesNotMatch(sourceSetup, /<RecordingSourcePreview/, 'preview actions are not duplicated in Screen & sound')
assert.match(record, /<StudioPreview[\s\S]*onSourcePreviewStart=\{\(\) => \{ void sourcePreview\.start\(\) \}\}/,
  'the Studio action delegates to the existing native preview owner')
assert.match(record, /1 · Screen and sound/, 'the source rail uses the approved heading')
assert.match(record, /Polished clip in Edit/, 'the polished mode names the Edit destination')
assert.match(record, /Raw MP4 file/, 'the raw mode names the saved file')

const schema = readFileSync(
  fileURLToPath(new URL('../../schema/verbs/fragments/064-screen_record_preview.json', import.meta.url)),
  'utf8',
)
assert.match(schema, /mime:'image\/bmp'/, 'the public contract permits BMP only')
assert.match(schema, /source_selection:'exact'\|'portal'/, 'capability advertises exact versus portal source selection')
assert.match(schema, /"agent_chat": "deny"/, 'live pixels remain outside agent-chat exposure')
assert.doesNotMatch(schema, /"process": true/, 'preview never owns an encoder child process')
