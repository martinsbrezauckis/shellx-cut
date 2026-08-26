import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8')

const picker = read('../../app/recorder/record-capture/src/windows_picker.rs')
const windowsCapture = read('../../app/recorder/record-capture/src/windows.rs')
const macosCapture = read('../../app/recorder/record-capture/src/macos.rs')
const identity = read('../../app/recorder/record-capture/src/window_target.rs')
const record = read('../src/panels/Record/index.tsx')
const schema = JSON.parse(read('../../schema/verbs.json'))

assert.match(identity, /windows-hwnd-v1/, 'Windows ids are explicitly versioned')
assert.match(identity, /macos-scwindow-v1/, 'macOS ids are explicitly versioned')
assert.match(identity, /hwnd != 0 && pid != 0/, 'Windows refuses zero native identities')
assert.match(identity, /Fixture Window.*None/, 'a display title cannot parse as a Windows target')

assert.match(picker, /GetWindowThreadProcessId/, 'Windows binds each HWND to its current process')
assert.match(picker, /QueryFullProcessImageNameW/, 'Windows returns truthful owning-process metadata')
assert.match(picker, /current_pid != expected_pid/, 'capture revalidates against HWND reuse')
assert.match(picker, /window_is_capturable/, 'capture revalidates current window eligibility')
assert.match(windowsCapture, /resolve_window\(window_id\)/, 'WGC consumes the exact enumerated identity')
assert.match(windowsCapture, /from_raw_hwnd/, 'WGC opens the resolved HWND directly')
assert.doesNotMatch(windowsCapture, /from_contains_name/, 'WGC never searches by title substring')
assert.match(macosCapture, /parse_macos_window_id/, 'ScreenCaptureKit validates its opaque identity')
assert.match(macosCapture, /w\.window_id\(\) == want_id/, 'ScreenCaptureKit selects by exact native id')

assert.match(record, /value=\{`win:\$\{w\.id\}`\}/, 'the visible option carries the opaque identity')
assert.match(record, /startArgs\.window = windowTargetId/, 'the UI returns the identity unchanged')
assert.match(record, /selectedWindowMissing/, 'the UI keeps a vanished selection visible')
assert.match(record, /selected window is no longer available/, 'the UI explains the refusal')
assert.match(record, /disabled=\{busy \|\| startAllowed === false \|\| selectedWindowMissing\}/, 'a vanished target cannot start capture')
assert.doesNotMatch(record, /startArgs\.window = windowTitle/, 'the UI never submits display copy')

for (const name of ['screen_record.start', 'debug.screenshot']) {
  const verb = schema.verbs.find((candidate: { name?: string }) => candidate.name === name)
  assert.ok(verb, `${name} remains public`)
  assert.match(verb.args.properties.window.description, /opaque live native/i, `${name} documents native identity authority`)
  assert.match(verb.args.properties.window.description, /title.*never/i, `${name} rejects title selection`)
}
const doctor = schema.verbs.find((candidate: { name?: string }) => candidate.name === 'screen_record.doctor')
assert.match(doctor.result, /windows\[\]\.id.*passed unchanged/i, 'Doctor owns the picker-to-capture identity contract')

console.log('PASS stable native recording-window identity contract')
