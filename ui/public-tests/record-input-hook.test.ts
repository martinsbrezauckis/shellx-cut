import assert from 'node:assert/strict'
import { recordingInputHook, recordingInputHookWarning } from '../src/panels/Record/recordingInputHook'

const registered = { state: 'registered', backend: 'rdevin_x11', reason: null, capture_keys: false }
assert.equal(recordingInputHook(registered).state, 'registered', 'native null-reason registration acknowledgment is preserved')
for (const value of [null, [], {}, { ...registered, backend: 'unknown' }, { ...registered, backend: 'wayland_evdev' },
  { ...registered, state: 'unobserved' }, { ...registered, reason: 'startup_failed' },
  { ...registered, extra: true }, { ...registered, capture_keys: 1 }]) {
  assert.equal(recordingInputHook(value).state, 'unobserved')
  assert.equal(recordingInputHookWarning(recordingInputHook(value), false), null)
}
for (const backend of ['rdevin_windows', 'rdevin_macos', 'rdevin_x11']) {
  const observation = recordingInputHook({ state: 'unavailable', backend, reason: 'startup_failed', capture_keys: false })
  assert.equal(observation.state, 'unavailable')
  assert.match(recordingInputHookWarning(observation, true) ?? '', /Video saved/)
}
for (const backend of ['unknown', 'wayland_evdev']) {
  assert.equal(recordingInputHook({ state: 'unobserved', backend, reason: null, capture_keys: true }).backend, backend)
  assert.equal(recordingInputHook({ state: 'unavailable', backend, reason: 'startup_failed', capture_keys: true }).backend, 'unknown')
}
console.log('PASS observed input startup parser and warning contract')
