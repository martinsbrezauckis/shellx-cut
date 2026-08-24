import { strict as assert } from 'node:assert'
import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Keyframe } from '../src/lib/client'
import {
  clampAutomationPercent,
  clampAutomationTime,
  createVolumeAutomationMutationController,
  createVolumeAutomationStaticGainGuard,
  formatAutomationPercent,
  formatAutomationTime,
  removeVolumeAutomationPoint,
  replaceVolumeAutomationPoint,
  volumeAutomationInterpolation,
  volumeAutomationKeyframesFingerprint,
  volumeAutomationNeedsAuthoritativeRefresh,
  volumeAutomationTrack,
  volumeAutomationUnavailableReason,
} from '../src/panels/Inspector/volumeAutomationModel'

// `edit.keyframe` replaces a complete parameter track. These pure contracts
// protect the Inspector's optimistic projection before its browser proof runs.
const track = volumeAutomationTrack([
  { param: 'opacity', points: [{ t_ms: 0, value: 1 }] },
  {
    param: 'volume',
    interp: 'ease_in_out_cubic',
    points: [{ t_ms: 800, value: 1 }, { t_ms: 0, value: 0.5 }],
  },
] as Keyframe[])
assert.deepEqual(track, {
  points: [{ t_ms: 0, value: 0.5 }, { t_ms: 800, value: 1 }],
  interp: 'ease_in_out_cubic',
}, 'volume automation reads only the volume track and sorts its points')
assert.deepEqual(replaceVolumeAutomationPoint(track.points, 800, 75), [
  { t_ms: 0, value: 0.5 },
  { t_ms: 800, value: 0.75 },
], 'volume automation replaces exactly one timestamp under SET semantics')
assert.deepEqual(replaceVolumeAutomationPoint(track.points, 400, 125), [
  { t_ms: 0, value: 0.5 },
  { t_ms: 400, value: 1.25 },
  { t_ms: 800, value: 1 },
], 'volume automation inserts a sorted new control point')
assert.deepEqual(removeVolumeAutomationPoint(track.points, 0), [{ t_ms: 800, value: 1 }], 'volume automation removes exactly one control point')
assert.equal(clampAutomationTime(-10, 840), 0, 'automation clamps a negative clip time')
assert.equal(clampAutomationTime(1000, 840), 840, 'automation clamps time to the realized clip duration')
assert.equal(clampAutomationPercent(-2), 0, 'automation clamps negative levels')
assert.equal(clampAutomationPercent(999), 400, 'automation keeps normal Inspector level entry bounded')
assert.equal(volumeAutomationInterpolation('hold', 'linear'), 'hold', 'automation accepts a supported interpolation')
assert.equal(volumeAutomationInterpolation('unknown', 'linear'), 'linear', 'automation keeps a safe interpolation fallback')
assert.equal(formatAutomationTime(500), '0.50s', 'automation labels clip-local times in seconds')
assert.equal(formatAutomationPercent(1.25), '125%', 'automation labels gain multipliers as a human percentage')
assert.equal(volumeAutomationUnavailableReason(1_000, false), null, 'automation remains available for a timed constant-speed clip')
assert.equal(
  volumeAutomationUnavailableReason(1_000, true),
  'Clear the Speed ramp before editing volume automation.',
  'automation names the engine speed-ramp conflict before submit',
)
assert.equal(
  volumeAutomationUnavailableReason(0, false),
  'This audio clip has no usable duration yet, so automation is unavailable.',
  'automation names a missing timeline duration',
)

// `edit.keyframe` uses SET semantics, so the controller must serialize saves
// locally *and* pass the durable project revision to the server. This models a
// controlled deferred response: R1 saves one point, R2 starts from that full
// projected track, then a stale R1 project refresh arrives while R2 is pending.
const sequence = createVolumeAutomationMutationController('op_000001')
const r1Track = replaceVolumeAutomationPoint([], 0, 60)
const r1 = sequence.begin('volume-r1-0001')
assert.deepEqual(r1, { request_id: 'volume-r1-0001', expected_revision: 'op_000001' }, 'R1 carries its durable request identity and base revision')
assert.equal(sequence.begin('volume-duplicate-0001'), null, 'the real in-flight lock rejects an immediate duplicate dispatch')
assert.equal(sequence.complete('volume-r1-0001', { ok: true, projectRevision: 'op_000002' }).status, 'saved', 'R1 accepts its returned durable revision')
assert.equal(sequence.observeAuthoritative('op_000001').applied, false, 'a pre-R1 project refresh cannot replace a newer accepted revision')
assert.equal(sequence.state().projectRevision, 'op_000002', 'R1 projection remains based on its returned revision')

const r2Track = replaceVolumeAutomationPoint(r1Track, 500, 100)
const r2 = sequence.begin('volume-r2-0001')
assert.deepEqual(r2, { request_id: 'volume-r2-0001', expected_revision: 'op_000002' }, 'R2 starts from R1’s returned revision')
sequence.observeAuthoritative('op_000002') // delayed R1 snapshot while R2 is in flight
assert.equal(sequence.state().inFlight, true, 'a delayed R1 refresh never unlocks R2')
assert.equal(sequence.begin('volume-r3-0001'), null, 'the delayed R1 refresh cannot permit an R3 dispatch against the pre-R2 track')
assert.deepEqual(r2Track, [{ t_ms: 0, value: 0.6 }, { t_ms: 500, value: 1 }], 'R2 retains R1’s point while adding its own SET-semantics point')
assert.equal(sequence.complete('volume-r2-0001', { ok: true, projectRevision: 'op_000003' }).status, 'saved', 'R2 accepts its own returned durable revision')
assert.deepEqual(sequence.begin('volume-r3-0002'), { request_id: 'volume-r3-0002', expected_revision: 'op_000003' }, 'only the completed R2 revision can authorize R3')

// An external edit makes the server reject the stale expected revision. A newer
// project.state snapshot may become the next safe base; without one, the editor
// stays fail-closed instead of reusing the rejected revision.
const externalConflict = createVolumeAutomationMutationController('op_000010')
assert.ok(externalConflict.begin('volume-external-0001'))
externalConflict.observeAuthoritative('op_000011')
const refreshedConflict = externalConflict.complete('volume-external-0001', { ok: false, errorCode: 'conflict' })
assert.equal(refreshedConflict.status, 'conflict', 'external revision conflict is surfaced locally')
assert.equal(refreshedConflict.state.projectRevision, 'op_000011', 'a received newer authoritative snapshot is the only retry base after conflict')
assert.deepEqual(externalConflict.begin('volume-after-external-0001'), { request_id: 'volume-after-external-0001', expected_revision: 'op_000011' }, 'retry never reuses the rejected external-conflict base')

const unsyncedConflict = createVolumeAutomationMutationController('op_000020')
const unsyncedRequest = unsyncedConflict.begin('volume-unsynced-0001')
assert.ok(unsyncedRequest)
const failClosed = unsyncedConflict.complete('volume-unsynced-0001', { ok: false, errorCode: 'conflict' })
assert.equal(failClosed.state.projectRevision, null, 'a conflict without a newer authoritative snapshot clears the stale base')
assert.equal(unsyncedConflict.begin('volume-unsynced-retry-0001'), null, 'a conflict without refresh cannot dispatch a speculative retry')
unsyncedConflict.requireAuthoritativeAfter(unsyncedRequest.expected_revision)
assert.equal(unsyncedConflict.observeAuthoritative('op_000020').applied, false, 'the same stale conflict base cannot satisfy the refresh barrier')
assert.equal(unsyncedConflict.observeAuthoritative('op_000020').applied, false, 'rebuilt same-P20 project props cannot clear the local refresh fence')
assert.equal(unsyncedConflict.state().projectRevision, null, 'same-revision project props stay unavailable after conflict')
assert.equal(unsyncedConflict.begin('volume-unsynced-stale-retry-0001'), null, 'same-revision props cannot authorize a speculative retry')
assert.equal(unsyncedConflict.observeAuthoritative('op_000021').applied, true, 'strictly newer project props clear the refresh barrier')
assert.deepEqual(
  unsyncedConflict.begin('volume-unsynced-fresh-retry-0001'),
  { request_id: 'volume-unsynced-fresh-retry-0001', expected_revision: 'op_000021' },
  'only the newer authoritative revision becomes the next safe mutation base',
)
assert.equal(
  volumeAutomationNeedsAuthoritativeRefresh(false, { status: 'failed', state: { projectRevision: 'op_000020', inFlight: false } }),
  true,
  'a lost response remains unknown even when the local controller retained its old revision',
)
assert.equal(
  volumeAutomationNeedsAuthoritativeRefresh(true, { status: 'conflict', state: { projectRevision: null, inFlight: false } }),
  true,
  'a conflict before refresh remains unknown until authoritative project props arrive',
)
assert.equal(
  volumeAutomationNeedsAuthoritativeRefresh(true, { status: 'unavailable', state: { projectRevision: null, inFlight: false } }),
  true,
  'an ok response without project revision remains unknown because the request may have committed',
)
assert.equal(
  volumeAutomationNeedsAuthoritativeRefresh(true, { status: 'failed', state: { projectRevision: 'op_000020', inFlight: false } }),
  false,
  'an explicit non-conflict server rejection safely leaves the current authoritative state usable',
)

// VolumeSection receives this transient report from the editor before React can
// repaint the sibling Gain control. This deferred sequence proves the actual
// shared guard cannot dispatch static Gain from the first save through its
// successful optimistic projection, even while project props still report zero
// volume points.
const gainGuard = createVolumeAutomationStaticGainGuard()
let gainDispatches = 0
let gainControls: { request_id: string; expected_revision: string } | null = null
gainGuard.observeAuthoritative('audio-clip-A', 0, 'op_000020')
const attemptStaticGain = () => gainGuard.runIfAllowed('audio-clip-A', (controls) => { gainDispatches += 1; gainControls = controls })
gainGuard.report({ clipId: 'audio-clip-A', inFlight: true, effectivePointCount: 1, projected: true, projectRevision: 'op_000020', refreshRequired: false })
assert.equal(attemptStaticGain(), false, 'first automation save synchronously blocks a stale mounted Gain input')
gainGuard.report({ clipId: 'audio-clip-A', inFlight: false, effectivePointCount: 1, projected: true, projectRevision: 'op_000021', refreshRequired: false })
assert.equal(attemptStaticGain(), false, 'successful optimistic automation projection stays ahead of stale zero-point props')
gainGuard.report({ clipId: 'audio-clip-A', inFlight: false, effectivePointCount: 1, projected: false, refreshRequired: false })
gainGuard.observeAuthoritative('audio-clip-A', 1, 'op_000021')
assert.equal(attemptStaticGain(), false, 'authoritative refresh with a persisted point keeps static Gain blocked')
assert.equal(gainDispatches, 0, 'no static Gain dispatch escapes the first-save-to-refresh coordination window')
gainGuard.report({ clipId: 'audio-clip-A', inFlight: false, effectivePointCount: 0, projected: false, refreshRequired: false })
assert.equal(
  gainGuard.runIfAllowed('audio-clip-A', () => { gainDispatches += 1 }),
  false,
  'a conflicted stale zero-point completion cannot override external authoritative automation',
)
assert.equal(gainDispatches, 0, 'external automation after a conflict still prevents static Gain dispatch')
gainGuard.observeAuthoritative('audio-clip-B', 0, 'op_000022')
assert.equal(attemptStaticGain(), false, 'a queued old-clip handler cannot dispatch against a newly selected clip')
gainGuard.report({ clipId: 'audio-clip-B', inFlight: false, effectivePointCount: 1, projected: true, projectRevision: 'op_000022', refreshRequired: false })
assert.equal(gainGuard.runIfAllowed('audio-clip-B', () => { gainDispatches += 1 }), false, 'new clip automation still blocks its own static Gain')
gainGuard.report({ clipId: 'audio-clip-B', inFlight: false, effectivePointCount: 0, projected: false, refreshRequired: false })
assert.equal(gainGuard.runIfAllowed('audio-clip-B', (controls) => { gainDispatches += 1; gainControls = controls }), true, 'a remounted clip can dispatch after its own guard clears')
assert.equal(gainDispatches, 1, 'only the new clip may dispatch its unaffected static Gain')
assert.equal(gainControls?.expected_revision, 'op_000022', 'permanent static Gain reads the current guard revision, not a stale render closure')
assert.match(gainControls?.request_id ?? '', /^ui-volume-/, 'permanent static Gain allocates a durable request identity')

const refreshRequiredGuard = createVolumeAutomationStaticGainGuard()
let refreshRequiredDispatches = 0
refreshRequiredGuard.observeAuthoritative('audio-clip-C', 0, 'op_000030')
refreshRequiredGuard.report({ clipId: 'audio-clip-C', inFlight: false, effectivePointCount: 0, projected: false, refreshRequired: true })
assert.equal(
  refreshRequiredGuard.runIfAllowed('audio-clip-C', () => { refreshRequiredDispatches += 1 }),
  false,
  'a conflict before project refresh keeps static Gain blocked despite stale zero-point props',
)
assert.equal(
  refreshRequiredGuard.runIfAllowed('audio-clip-C', () => { refreshRequiredDispatches += 1 }),
  false,
  'a lost response also keeps static Gain blocked because the request may have committed',
)
refreshRequiredGuard.observeAuthoritative('audio-clip-C', 0, 'op_000031')
refreshRequiredGuard.report({ clipId: 'audio-clip-C', inFlight: false, effectivePointCount: 0, projected: false, refreshRequired: false })
assert.equal(
  refreshRequiredGuard.runIfAllowed('audio-clip-C', () => { refreshRequiredDispatches += 1 }),
  true,
  'the later authoritative refresh clears the unknown state when its track has no automation',
)
const successfulClearGuard = createVolumeAutomationStaticGainGuard()
let successfulClearDispatches = 0
let clearControls: { request_id: string; expected_revision: string } | null = null
successfulClearGuard.observeAuthoritative('audio-clip-D', 2, 'op_000040')
successfulClearGuard.report({ clipId: 'audio-clip-D', inFlight: false, effectivePointCount: 0, projected: true, projectRevision: 'op_000041', refreshRequired: false })
assert.equal(
  successfulClearGuard.runIfAllowed('audio-clip-D', (controls) => { successfulClearDispatches += 1; clearControls = controls }),
  true,
  'a confirmed projected clear may re-enable static Gain before its stale server point list refreshes',
)
assert.equal(successfulClearDispatches, 1, 'successful clear is the only safe zero-point optimistic exception')
assert.equal(clearControls?.expected_revision, 'op_000041', 'a projected clear carries its returned revision instead of stale project props')

const missingRevisionGuard = createVolumeAutomationStaticGainGuard()
missingRevisionGuard.observeAuthoritative('audio-clip-E', 0, null)
assert.equal(missingRevisionGuard.state().blocked, true, 'static Gain fails closed while project revision is unavailable')
assert.equal(missingRevisionGuard.runIfAllowed('audio-clip-E', () => { gainDispatches += 1 }), false, 'missing project revision cannot emit edit.gain')

// An editor-local refresh fence disappears on A→B→A or a right-rail remount.
// The static action still carries the current guard revision, so server OCC
// rejects P20 against external P21 before an old Gain request can mutate state.
const remountedGainGuard = createVolumeAutomationStaticGainGuard()
remountedGainGuard.observeAuthoritative('audio-clip-A', 0, 'op_000020')
remountedGainGuard.observeAuthoritative('audio-clip-B', 0, 'op_000020')
remountedGainGuard.observeAuthoritative('audio-clip-A', 0, 'op_000020')
let remountControls: { request_id: string; expected_revision: string } | null = null
let serverStyleGainMutations = 0
let serverStyleConflict = false
assert.equal(remountedGainGuard.runIfAllowed('audio-clip-A', (controls) => {
  remountControls = controls
  if (controls.expected_revision === 'op_000021') serverStyleGainMutations += 1
  else serverStyleConflict = true
}), true, 'a remounted zero-point control can only submit a controlled static-Gain request')
assert.equal(remountControls?.expected_revision, 'op_000020', 'A→B→A stale Gain carries its P20 base to the server')
assert.match(remountControls?.request_id ?? '', /^ui-volume-/, 'A→B→A static Gain also gets a fresh request identity')
assert.notEqual(remountControls?.request_id, clearControls?.request_id, 'separate static Gain requests never reuse a durable request identity')
assert.equal(serverStyleConflict, true, 'the server-style P21 revision check rejects stale P20')
assert.equal(serverStyleGainMutations, 0, 'the rejected stale Gain request produces no mutation')

assert.notEqual(
  volumeAutomationKeyframesFingerprint([{ param: 'volume', points: r1Track }] as Keyframe[]),
  volumeAutomationKeyframesFingerprint([{ param: 'volume', points: r2Track }] as Keyframe[]),
  'semantically changed server keyframes distinguish the complete projected tracks',
)

// Source contracts keep the compact Inspector point editor honest: no faux lane,
// no raw-millisecond input, and no control advertised where the engine refuses it.
const uiRoot = resolve(import.meta.dirname, '..')
const repoRoot = resolve(uiRoot, '..')
const source = (relative: string) => readFileSync(resolve(uiRoot, relative), 'utf8')
const inspector = source('src/panels/Inspector/index.tsx')
const app = source('src/App.tsx')
const appRightRail = source('src/app/AppRightRail.tsx')
const client = source('src/lib/client.ts')
const volumeSection = source('src/panels/Inspector/VolumeSection.tsx')
const editor = source('src/panels/Inspector/VolumeAutomationEditor.tsx')
const model = source('src/panels/Inspector/volumeAutomationModel.ts')
const propertyRow = source('src/components/inspector/PropertyRow.tsx')
const verifier = source('public-tests/full-coverage-verify.mjs')
const publicFeatures = readFileSync(resolve(repoRoot, 'docs/public/FEATURES.md'), 'utf8')
const coreEdit = readFileSync(resolve(repoRoot, 'app/core/src/edit.rs'), 'utf8')
const moduleSizeGate = readFileSync(resolve(repoRoot, 'scripts/module-size-gate.mjs'), 'utf8')

assert.ok(inspector.includes("from './VolumeSection'"), 'Inspector imports the dedicated Volume section')
assert.ok(app.includes('projectRevision={project?.project_revision ?? null}'), 'App keeps the project.state revision ephemeral while passing it to the rail')
assert.ok(appRightRail.includes('projectRevision={projectRevision}'), 'right rail carries the ephemeral revision to Inspector')
assert.ok(inspector.includes('projectRevision={projectRevision}'), 'Inspector carries the ephemeral revision to Volume')
assert.ok(volumeSection.includes('sectionKey="volume"'), 'Volume section owns the Inspector section')
assert.ok(volumeSection.includes('mediaClipTimelineDurationMs(clip)'), 'Volume uses the canonical retimed clip duration helper')
assert.ok(volumeSection.includes("from './VolumeAutomationEditor'"), 'Volume composes the dedicated automation editor')
assert.ok(volumeSection.includes('durationMs={durationMs}'), 'Volume passes realized clip duration to automation')
assert.ok(volumeSection.includes('projectRevision={projectRevision}'), 'Volume passes the current durable revision only to automation')
assert.ok(volumeSection.includes('key={clipId}'), 'Changing audio clips remounts the editor with clean draft, status, and busy state')
assert.ok(volumeSection.includes('createVolumeAutomationStaticGainGuard'), 'Volume owns a synchronous sibling-control guard')
assert.ok(volumeSection.includes('observeAuthoritative(clipId, automationPoints, projectRevision)'), 'Volume stores current server revision and point count inside the synchronous guard')
assert.ok(volumeSection.includes('runIfAllowed(clipId, (controls) =>'), 'Gain and reset dispatch through the synchronous current-identity guard')
assert.ok(volumeSection.includes("rationale: 'inspector: reset gain', ...controls"), 'Volume reset carries a fresh controlled mutation request')
assert.ok(volumeSection.includes('rationale: `inspector: gain ${v} dB`, ...controls'), 'Volume static Gain carries a fresh controlled mutation request')
assert.ok(volumeSection.includes('disabled={gainState.blocked}'), 'Volume disables static Gain for both in-flight and projected automation')
assert.ok(volumeSection.includes('Clear automation to edit static Gain'), 'Volume explains why static Gain is temporarily unavailable')
assert.ok(volumeSection.includes('Volume controls are waiting for the current project revision.'), 'Volume visibly fails closed when static Gain lacks a safe mutation base')
assert.ok(volumeSection.includes('hasSpeedRamp={clip.speed_ramp != null}'), 'Volume passes speed-ramp state to the automation guard')
assert.ok(volumeSection.includes('isAudioClip ?'), 'Volume automation is limited to eligible audio clips')
assert.ok(propertyRow.includes('disabled?: boolean'), 'PropertyRow exposes an explicit disabled contract')
assert.ok(propertyRow.includes('disabled={disabled}'), 'PropertyRow disables its native controls rather than only blocking CSS pointers')

assert.ok(existsSync(resolve(uiRoot, 'src/panels/Inspector/VolumeAutomationEditor.tsx')), 'Volume automation editor has an owned source module')
assert.ok(existsSync(resolve(uiRoot, 'src/panels/Inspector/volumeAutomationModel.ts')), 'Volume automation model has an owned pure source module')
assert.ok(editor.includes("runUserVerb(\n        'edit.keyframe'"), 'Volume automation dispatches edit.keyframe with local feedback')
assert.ok(editor.includes("param: 'volume'"), 'Volume automation binds the engine volume parameter')
assert.ok(editor.includes('...controls'), 'Volume automation passes request identity and expected revision only for its controlled mutation')
assert.ok(editor.includes('createVolumeAutomationMutationController'), 'Volume automation owns a revision-aware in-flight controller')
assert.ok(editor.includes('nextVolumeAutomationRequestId'), 'Volume automation allocates a new durable request identity per save')
assert.ok(editor.includes('if (!controls)'), 'Automation guards commit itself before any duplicate dispatch can escape')
assert.ok(editor.includes('current project revision'), 'Automation visibly explains a missing safe mutation base')
assert.ok(editor.includes('onAutomationStateChange'), 'Automation publishes transient point state to its sibling Volume controls')
assert.ok(editor.includes('reportAutomationState(true, points.length, true, false)'), 'Automation reports its first-save lock before awaiting the server')
assert.ok(editor.includes('refreshRequiredRef.current = true'), 'Unknown save outcomes keep sibling static Gain blocked until refresh')
assert.ok(editor.includes('volumeAutomationNeedsAuthoritativeRefresh'), 'Automation distinguishes unknown results from explicit safe server rejections')
assert.ok(client.includes('export interface MutationControls'), 'client exports the shared mutation-control contract instead of a keyframe-only cast')
assert.ok(client.includes('ControlledVerbArgs<N extends VerbName>'), 'client provides a controlled args intersection for any live verb')
assert.ok(client.includes('project_revision?: string'), 'client exposes the server envelope project revision')
for (const action of [
  'volume-automation-time',
  'volume-automation-level',
  'volume-automation-interpolation',
  'volume-automation-add',
  'volume-automation-point',
  'volume-automation-remove',
  'volume-automation-clear',
]) assert.ok(editor.includes(`data-cut-action="${action}"`), `Volume automation exposes stable ${action} ownership`)
assert.ok(!editor.includes('full timeline automation lane remains future work'), 'Volume automation keeps roadmap debt out of product copy')
assert.ok(publicFeatures.includes('a full lane remains future work'), 'Public feature docs retain the future full-lane limitation')
assert.ok(publicFeatures.includes('Automation\n  saves are revision-protected'), 'Public feature docs explain conflict-safe automation saves')
assert.ok(editor.includes('Time in clip (seconds)'), 'Volume automation labels the human time field in seconds')
assert.ok(editor.includes('Interpolation</span>'), 'Volume automation gives interpolation a visible label')
assert.ok(editor.includes('Math.round(Number(event.target.value) * 1000)'), 'Volume automation dispatches typed seconds as exact milliseconds')
assert.ok(editor.includes('aria-label="Volume automation interpolation"'), 'Interpolation has a keyboard-accessible name')
assert.ok(editor.includes('aria-label={`Edit volume point at ${formatAutomationTime(point.t_ms)}`}'), 'Point selection has a keyboard-accessible name')
assert.ok(editor.includes('aria-label={`Remove volume point at ${formatAutomationTime(point.t_ms)}`}'), 'Point removal has a keyboard-accessible name')
assert.ok(editor.includes('try {') && editor.includes('finally {'), 'Automation clears busy state after every commit outcome')
assert.ok(editor.includes('mutationState.inFlight'), 'Automation derives disabled state from the real in-flight lock')
assert.ok(editor.includes('keyframesFingerprint'), 'Automation reconciles optimistic projection against authoritative keyframes')
assert.ok(editor.includes('data-cut-volume-automation-unavailable'), 'Automation names unavailable states instead of leaving controls unexplained')
assert.ok(model.includes('VOLUME_AUTOMATION_INTERPOLATIONS'), 'Automation model owns the interpolation catalog')
assert.ok(model.includes('createVolumeAutomationStaticGainGuard'), 'Automation model owns the tested static-Gain coordination guard')
assert.ok(model.includes('requireAuthoritativeAfter'), 'Automation model retains its revision-bound refresh fence')
assert.ok(model.includes('compareProjectRevisions(received, refreshAfterRevision) <= 0'), 'same or older project props cannot clear the refresh fence')
assert.ok(model.includes('expected_revision: current.projectRevision'), 'static Gain guard derives mutation controls from its latest revision ref')
assert.ok(model.includes('Clear the Speed ramp before editing volume automation.'), 'Automation model exposes a visible speed-ramp reason')
assert.ok(coreEdit.includes('if c.speed_ramp.is_some()'), 'Core edit.keyframe fails closed for a speed-ramped clip')
assert.ok(verifier.includes('Gain disabled=${gainDisabled}'), 'Focused browser flow proves static Gain becomes disabled after the first persisted point')
assert.ok(verifier.includes("gainReason === 'Clear automation to edit static Gain'"), 'Focused browser flow proves the visible Gain unavailable reason')
const automationViewportReset = "await automation.evaluate((element) => element.scrollIntoView({ block: 'center', inline: 'nearest' }))"
assert.ok(
  verifier.indexOf(automationViewportReset) > verifier.indexOf("rec(S, 'GATE:volume-automation-shown'"),
  'Focused browser flow resets the Volume automation viewport after the editor is admitted',
)
assert.ok(
  verifier.indexOf(automationViewportReset) < verifier.indexOf("name: 'volume-automation-time'"),
  'Focused browser flow resets the Volume automation viewport before the first cached render measurement',
)
assert.ok(moduleSizeGate.includes('ui/src/panels/Inspector/VolumeAutomationEditor.tsx'), 'Module-size gate bounds the Volume automation editor')
assert.ok(moduleSizeGate.includes('ui/src/panels/Inspector/volumeAutomationModel.ts'), 'Module-size gate bounds the Volume automation model')
assert.ok(moduleSizeGate.includes('ui/public-tests/volume-automation.test.ts'), 'Module-size gate bounds the dedicated volume automation regression test')

console.log('PASS volume automation model and source contracts')
