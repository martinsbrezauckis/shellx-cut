import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8')

const picker = read('../../app/recorder/record-capture/src/windows_picker.rs')
const windowsCapture = read('../../app/recorder/record-capture/src/windows.rs')
const windowsSource = read('../../app/recorder/record-capture/src/windows_capture_source.rs')
const macosCapture = read('../../app/recorder/record-capture/src/macos.rs')
const macosTarget = read('../../app/recorder/record-capture/src/macos_capture_target.rs')
const identity = read('../../app/recorder/record-capture/src/window_target.rs')
const record = read('../src/panels/Record/index.tsx')
const session = read('../src/app/useRecordingSession.ts')
const sourceControl = read('../src/panels/Record/RecordingSourceControl.tsx')
const schema = JSON.parse(read('../../schema/verbs.json'))

assert.match(identity, /windows-hwnd-v1/, 'Windows ids are explicitly versioned')
assert.match(identity, /macos-scwindow-v1/, 'macOS ids are explicitly versioned')
assert.match(identity, /hwnd != 0 && pid != 0/, 'Windows refuses zero native identities')
assert.match(identity, /Fixture Window.*None/, 'a display title cannot parse as a Windows target')

function assertWindowsWindowIdentity(capture: string, source: string, currentPicker: string) {
  assert.match(currentPicker, /GetWindowThreadProcessId/, 'Windows binds each HWND to its current process')
  assert.match(currentPicker, /QueryFullProcessImageNameW/, 'Windows returns truthful owning-process metadata')
  assert.match(currentPicker, /parse_windows_window_id\(id\)/, 'the resolver parses the supplied opaque identity')
  assert.match(currentPicker, /current_pid != expected_pid \|\| !window_is_capturable\(hwnd,/,
    'capture rejects both HWND reuse and a window that is no longer capturable')
  assert.match(currentPicker, /let Some\(\(raw, expected_pid\)\) = crate::window_target::parse_windows_window_id\(id\) else \{\s*return Err\("[^"]*"\);\s*\};\s*let hwnd = HWND\(raw as \*mut std::ffi::c_void\);\s*let current_pid = window_pid\(hwnd\);\s*if current_pid != expected_pid \|\| !window_is_capturable\(hwnd, unsafe \{ GetCurrentProcessId\(\) \}\)\s*\{\s*return Err\("[^"]*"\);\s*\}\s*Ok\(hwnd\)/,
    'the native resolver rejects malformed, reused and ineligible identities before returning the supplied HWND')
  assert.match(capture, /if let Some\(ref window_id\) = cfg\.window\s*\{\s*let \(win, width, height\) = crate::windows_capture_source::window\(\s*window_id,\s*crate::windows_capture_source::WindowResolution::RecordingStart,\s*\)\?;\s*\(Src::Window\(win\),/,
    'capture passes its exact configured window identity to the resolver and keeps that returned window')
  assert.match(source, /pub\(crate\) fn window\(id: &str, mode: WindowResolution\) -> Result<\(Window, u32, u32\)> \{\s*let target = crate::windows_picker::resolve_window\(id\)\s*\.map_err\(crate::window_target::window_unavailable\)\?;\s*let window = Window::from_raw_hwnd\(target\.0\);/,
    'the extracted owner resolves the unchanged identity, propagates refusal, and opens only its HWND')
  assert.equal([...source.matchAll(/\blet\s+(?:mut\s+)?window\s*=/g)].length, 1,
    'the resolved window is never rebound to a derived or fallback selection')
  assert.match(source, /Ok\(\(window, width, height\)\)/, 'the owner returns that resolved window')
  assert.match(capture, /Src::Window\(win\) => Handler::start_free_threaded\(WcSettings::new\(\s*win,/,
    'WGC starts with the exact resolved window, not a fresh selection')
  assert.doesNotMatch(capture + source, /from_contains_name/, 'WGC never searches by title substring')
}

assertWindowsWindowIdentity(windowsCapture, windowsSource, picker)
// Guard the assertions themselves against the substitutions this contract forbids.
for (const [label, original, before, after] of [
  ['capture changes the selected identity', windowsCapture, '                window_id,', '                window_title,'],
  ['resolver derives a different identity', windowsSource, 'resolve_window(id)', 'resolve_window(title)'],
  ['resolver substitutes a native window', windowsSource, 'from_raw_hwnd(target.0)', 'from_raw_hwnd(std::ptr::null_mut())'],
  ['native capture substitutes a window', windowsCapture, '                    win,\n                    CursorCaptureSettings::WithCursor', '                    WcWindow::from_raw_hwnd(std::ptr::null_mut()),\n                    CursorCaptureSettings::WithCursor'],
  ['resolver loosens HWND reuse protection', picker, 'current_pid != expected_pid', 'current_pid == 0'],
  ['resolver returns a reused window', picker, 'return Err("the selected window is no longer available; reopen the source picker");', 'return Ok(hwnd);'],
  ['resolver rebinds the window', windowsSource, '    Ok((window, width, height))', '    let window = Window::from_raw_hwnd(std::ptr::null_mut());\n    Ok((window, width, height))'],
] as const) {
  assert.ok(original.includes(before), `mutation fixture exists: ${label}`)
  const changed = original.replace(before, after)
  assert.throws(() => assertWindowsWindowIdentity(
    original === windowsCapture ? changed : windowsCapture,
    original === windowsSource ? changed : windowsSource,
    original === picker ? changed : picker,
  ), { code: 'ERR_ASSERTION' }, label)
}
assert.match(macosCapture, /macos_capture_target::prepare_capture_target\(cfg, fps\)/,
  'ScreenCaptureKit delegates the unchanged config to its exact-target owner')
assert.match(macosTarget, /parse_macos_window_id\(want\)/,
  'the macOS target owner validates the opaque window identity')
assert.match(macosTarget, /window\.window_id\(\) == want_id/,
  'the macOS target owner selects by exact native window id')
assert.match(macosTarget, /SCContentFilter::create\(\)\.with_window\(window\)\.build\(\)/,
  'ScreenCaptureKit opens that exact selected window directly')
assert.match(macosTarget, /selected window is no longer available; reopen the source picker/,
  'a vanished exact macOS window fails closed instead of selecting another source')
assert.doesNotMatch(macosTarget, /title\(\)[\s\S]*want_id|want_id[\s\S]*title\(\)/,
  'the macOS target owner never substitutes a title for the opaque identity')

assert.match(sourceControl, /value=\{`win:\$\{window\.id\}`\}/, 'the visible option carries the opaque identity')
assert.match(record, /windowTargetId \? \{ kind: 'window' as const, windowId: windowTargetId \}/, 'Record stores the selected opaque window identity')
assert.match(session, /if \(preset\.source\.kind === 'window'\) args\.window = preset\.source\.windowId/, 'shared Start returns the identity unchanged')
assert.match(record, /selectedWindowMissing/, 'the UI keeps a vanished selection visible')
assert.match(record, /selected window is no longer available/, 'the UI explains the refusal')
assert.match(record, /if \(selectedWindowMissing\) return 'The selected window is no longer available/, 'a vanished target cannot start capture')
assert.doesNotMatch(session, /args\.window = windowTitle/, 'the UI never submits display copy')

for (const name of ['screen_record.start', 'debug.screenshot']) {
  const verb = schema.verbs.find((candidate: { name?: string }) => candidate.name === name)
  assert.ok(verb, `${name} remains public`)
  assert.match(verb.args.properties.window.description, /opaque live native/i, `${name} documents native identity authority`)
  assert.match(verb.args.properties.window.description, /title.*never/i, `${name} rejects title selection`)
}
const doctor = schema.verbs.find((candidate: { name?: string }) => candidate.name === 'screen_record.doctor')
assert.match(doctor.result, /windows\[\]\.id.*passes the selected id unchanged.*revalidates it/i, 'Doctor owns the picker-to-capture identity contract')
assert.match(doctor.result, /title and app remain display-only/i, 'Doctor keeps window presentation fields non-authoritative')

console.log('PASS stable native recording-window identity contract')
