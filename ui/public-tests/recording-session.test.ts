import assert from 'node:assert/strict'
import { RecordingToggleGate, classifyStopFailure, stopArgs } from '../src/app/recordingSessionModel'
import { doctorAllowsPortalDisplay, parseRecordingPreset, sameProjectIdentity, validateRecordingPreset, type RecordingPreset } from '../src/app/recordingPreset'

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
assert.match(validateRecordingPreset(preset, { ...doctor, start_allowed: false }) ?? '', /unavailable/)
const portalDoctor = { start_allowed: true, monitors: [], cards: [{ name: 'gstreamer' }, { name: 'wayland_input' }] }
assert.equal(doctorAllowsPortalDisplay(portalDoctor), true)
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

console.log('recording session model: passed')
