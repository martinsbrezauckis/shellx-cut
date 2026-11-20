import assert from 'node:assert/strict'
import { RecordingToggleGate, classifyStopFailure, isNothingRecorded, nothingRecordedText, refusalText, savedWithProblemText, savingProgressText, stopArgs, withSavedNotes } from '../src/app/recordingSessionModel'
import { doctorAllowsPortalDisplay, firstUseRecordingPreset, parseRecordingPreset, sameProjectIdentity, validateRecordingPreset, type RecordingPreset } from '../src/app/recordingPreset'

const preset: RecordingPreset = {
  schema: 'shellx-cut/recording-preset/1',
  source: { kind: 'window', windowId: 'window-7' },
  fps: 30, durationMs: null, audio: true, systemAudio: false, keys: false, raw: false,
}
const doctor = { start_allowed: true, windows: [{ id: 'window-7' }], monitors: [{ id: 'monitor-1' }] }

assert.equal(validateRecordingPreset(preset, doctor), null)
assert.match(validateRecordingPreset(preset, { ...doctor, window_capture_supported: false }) ?? '', /choose Display/,
  'a saved Window intent is refused when the backend reports no Window capture, even if a stale row remains')
assert.equal(validateRecordingPreset(preset, { ...doctor, windows: [], window_capture_supported: true }),
  'The saved window is no longer available. Choose a current window.',
  'an empty supported Windows/macOS list means the target is absent, not that Window capture is unsupported')
assert.match(validateRecordingPreset(preset, { ...doctor, windows: [{ id: 'window-8' }] }) ?? '', /saved window/)
assert.match(validateRecordingPreset({ ...preset, source: { kind: 'display', monitorId: 'monitor-1' } }, { ...doctor, monitors: [] }) ?? '', /saved display/)
assert.match(validateRecordingPreset(preset, { ...doctor, start_allowed: false }) ?? '', /Screen capture is not ready.*press Check/)
assert.match(
  validateRecordingPreset(preset, { ...doctor, start_allowed: false, cards: [{ name: 'screen_capture', status: 'degraded', detail: 'turn it on in System Settings > Privacy & Security > Screen & System Audio Recording' }] }) ?? '',
  /Screen capture is not ready: turn it on in System Settings/,
  'a refused start names the missing screen prerequisite and where to provide it')
const portalDoctor = { start_allowed: true, monitors: [], cards: [{ name: 'gstreamer' }, { name: 'wayland_input' }] }
assert.equal(doctorAllowsPortalDisplay(portalDoctor), true)
assert.deepEqual(firstUseRecordingPreset({ start_allowed: true, monitors: [
  { id: 'secondary', primary: false }, { id: 'primary', primary: true },
] }), {
  schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'primary' },
  fps: 30, durationMs: null, startCountdownSeconds: 0,
  audio: false, systemAudio: false, keys: false, raw: false,
  studio: { background: 'gradient' },
}, 'first-use F9 uses the current primary display, without silently enabling audio or camera')
assert.equal(validateRecordingPreset(firstUseRecordingPreset(portalDoctor)!, portalDoctor), null,
  'first-use Linux F9 can admit the current portal chooser without a fabricated monitor')
assert.equal(firstUseRecordingPreset({ start_allowed: false, monitors: [{ id: 'primary', primary: true }] }), null,
  'first-use F9 must not start after a failed current Doctor check')
assert.equal(firstUseRecordingPreset({ start_allowed: true, monitors: [{ primary: true }] }), null,
  'first-use F9 must not invent a monitor identity')
assert.equal(validateRecordingPreset({ ...preset, source: { kind: 'portal_display' } }, portalDoctor), null,
  'Linux portal display starts through its native chooser without a fabricated monitor id')
assert.match(validateRecordingPreset({ ...preset, source: { kind: 'portal_display' } }, doctor) ?? '', /portal/,
  'an ID-less display preset does not silently work on Windows or macOS')
assert.equal(parseRecordingPreset({ ...preset, source: { kind: 'window', windowId: '' } }), null)
assert.deepEqual(parseRecordingPreset(JSON.parse(JSON.stringify(preset))), preset)
assert.equal(sameProjectIdentity(
  { schema: 'shellx-cut/project-identity/1', origin_path_sha256: 'sha256:' + 'a'.repeat(64), project_name: 'A' },
  { schema: 'shellx-cut/project-identity/1', origin_path_sha256: 'sha256:' + 'b'.repeat(64), project_name: 'A' },
), false)

const gate = new RecordingToggleGate()
assert.equal(gate.claim(1000), true)
assert.equal(gate.claim(1001), false, 'native and DOM callbacks share one physical press')
gate.setInFlight(true)
assert.equal(gate.claim(1500), false, 'a pending Start or Stop cannot be toggled')
gate.setInFlight(false)
assert.equal(gate.claim(1501), true)

assert.deepEqual(stopArgs('capture-1', true), { capture_id: 'capture-1', autoedit: false, mux_raw: true })
assert.deepEqual(stopArgs('capture-1', false), { capture_id: 'capture-1', autoedit: true, mux_raw: true })
assert.equal(classifyStopFailure('capture-1', { ok: true, result: { capture_id: 'capture-1', terminal: false } }).state, 'live')
assert.equal(classifyStopFailure('capture-1', { ok: true, result: { capture_id: 'capture-1', terminal: true } }).state, 'ended')
assert.equal(classifyStopFailure('capture-1', { ok: true, result: { capture_id: 'capture-2', terminal: true } }).state, 'unknown')
assert.equal(classifyStopFailure('capture-1', { ok: false, code: 'not_found' }).state, 'ended')
assert.equal(classifyStopFailure('capture-1', { ok: true, result: { capture_id: 'capture-1' } }).state, 'unknown',
  'a missing terminal field never exposes Retry Stop')

// A refusal keeps the engine's way forward; an early Stop says nothing was
// recorded and how to record, and never points at recovery.
assert.equal(
  refusalText({ message: 'the selected window is no longer available', suggested_action: 'reopen the source picker and choose the window again' }),
  'The selected window is no longer available. Reopen the source picker and choose the window again.',
)
assert.equal(refusalText(undefined), 'Unknown error.')
assert.equal(refusalText({ message: 'output folder cannot be written: C:\\ro (access is denied)' }), 'Output folder cannot be written: C:\\ro (access is denied).')
const early = { code: 'capture', message: 'nothing was recorded', suggested_action: 'start the recording again and wait until the timer is running before you stop' }
assert.equal(isNothingRecorded(early), true)
assert.equal(isNothingRecorded({ message: 'screen capture stopped before the first frame' }), true, 'the Linux portal wording counts too')
assert.equal(isNothingRecorded({ message: 'finalize WGC checkpoint' }), false)
assert.equal(nothingRecordedText(early),
  'Nothing was recorded: Stop came before the first screen frame was captured. Start the recording again and wait until the timer is running before you stop.')
assert.doesNotMatch(classifyStopFailure('capture-1', { ok: false, code: 'not_found' }).detail, /inspect Recording Recovery/,
  'the recovery hint names the real place')

console.log('recording session model: passed')

// Rows 181 and 183: a take saved with a problem says what is wrong after the
// saved message; an exact take reads as before. Stop shows which part it saves.
assert.equal(savedWithProblemText(undefined), '')
assert.equal(savedWithProblemText([]), '')
assert.equal(withSavedNotes('Raw MP4 saved.', []), 'Raw MP4 saved.')
assert.equal(
  withSavedNotes('Raw MP4 saved.', ['The screen from 15.0 s to 30.0 s could not be saved, so the picture before it is shown for that time.', 7]),
  'Raw MP4 saved. Saved with a problem: The screen from 15.0 s to 30.0 s could not be saved, so the picture before it is shown for that time.',
)
assert.equal(savingProgressText({ parts_done: 2, parts_total: 5 }, 'Stopping…'), 'Saving the recording: part 3 of 5…')
assert.equal(savingProgressText({ parts_done: 5, parts_total: 5 }, 'Stopping…'), 'Saving the recording: part 5 of 5…')
assert.equal(savingProgressText(null, 'Stopping…'), 'Stopping…')
assert.equal(savingProgressText({ parts_done: 'x' }, 'Stopping…'), 'Stopping…')

