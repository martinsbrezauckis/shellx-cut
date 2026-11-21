import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8')

const identity = read('../../app/recorder/record-capture/src/monitor_identity.rs')
const windows = read('../../app/recorder/record-capture/src/windows_monitor_target.rs')
const macos = read('../../app/recorder/record-capture/src/macos_monitor_target.rs')
const macosTarget = read('../../app/recorder/record-capture/src/macos_capture_target.rs')
const projection = read('../../app/server/src/screen_record/doctor_projection.rs')
const record = read('../src/panels/Record/index.tsx')
const session = read('../src/app/useRecordingSession.ts')
const sourceControl = read('../src/panels/Record/RecordingSourceControl.tsx')
const schema = JSON.parse(read('../../schema/verbs.json'))
const reference = read('../../skill/shellx-cut/reference.md')
const debugApi = read('../../docs/public/DEBUG_API.md')

assert.match(identity, /shellx-monitor-v1/, 'monitor identities are versioned')
assert.match(identity, /Sha256/, 'the public id does not expose native display values')
assert.match(identity, /fn resolve_exact/, 'future target selection has one exact resolver')
assert.match(identity, /no fallback candidate/, 'resolver documentation rejects substitutes')

assert.match(windows, /monitorDevicePath/, 'Windows derives identity from the native target path')
assert.match(windows, /resolve_exact\(/, 'Windows re-enumerates and matches the exact identity')
assert.doesNotMatch(windows, /or_else\(.*primary|or_else\(.*width/i, 'Windows has no primary/geometry fallback')
assert.match(macos, /display\.display_id\(\)/, 'macOS derives identity from ScreenCaptureKit')
assert.match(macos, /resolve_exact\(/, 'macOS re-enumerates and matches the exact identity')
assert.match(projection, /id: monitor\.id/, 'Doctor projects the native id unchanged')
const windowsCapture = read('../../app/recorder/record-capture/src/windows.rs')
const macosCapture = read('../../app/recorder/record-capture/src/macos.rs')
const windowsSource = read('../../app/recorder/record-capture/src/windows_capture_source.rs')

function assertWindowsMonitorIdentity(capture: string, source: string, resolver: string) {
  assert.match(capture, /let monitor =\s*crate::windows_capture_source::monitor\(cfg\.monitor, cfg\.monitor_id\.as_deref\(\)\)\?;/,
    'capture passes its exact configured monitor identity through the extracted owner')
  assert.equal([...capture.matchAll(/\bmonitor\s*=(?!=)/g)].length, 1,
    'the resolved monitor is never reassigned or shadowed before WGC receives it')
  assert.match(capture, /\(Src::Monitor\(monitor\), width, height, surface\)/,
    'capture keeps the monitor returned by the exact resolver')
  assert.match(source, /pub\(crate\) fn monitor\(index: Option<u32>, id: Option<&str>\) -> Result<Monitor> \{\s*if let Some\(id\) = id \{\s*return crate::windows_monitor_target::resolve_monitor\(id\)\s*\.map_err\(\|error\| cap_err\("[^"]*", error\)\);\s*\}\s*match index/,
    'an explicit opaque identity is resolved unchanged and returned before any legacy ordinal or primary path')
  assert.match(resolver, /resolve_exact\(\s*id,\s*NativeMonitorPlatform::Windows,\s*monitors,\s*native_target_path,/,
    'native re-enumeration matches the supplied identity against physical target paths')
  assert.match(capture, /Src::Monitor\(m\) => Handler::start_free_threaded\(WcSettings::new\(\s*m,/,
    'WGC starts with that exact monitor, not a fresh primary or ordinal selection')
  assert.doesNotMatch(resolver, /or_else\(.*primary|or_else\(.*width/i, 'an unavailable identity has no primary or geometry fallback')
}

assertWindowsMonitorIdentity(windowsCapture, windowsSource, windows)
for (const [label, original, before, after] of [
  ['capture discards the supplied identity', windowsCapture, 'cfg.monitor, cfg.monitor_id.as_deref()', 'cfg.monitor, None'],
  ['resolver re-derives an identity', windowsSource, 'resolve_monitor(id)', 'resolve_monitor(&index.to_string())'],
  ['capture rebinds the resolved monitor', windowsCapture, '            (Src::Monitor(monitor), width, height, surface)', '            let monitor = WcMonitor::primary().unwrap();\n            (Src::Monitor(monitor), width, height, surface)'],
  ['resolver continues into legacy selection', windowsSource, 'return crate::windows_monitor_target::resolve_monitor(id)', 'let _ = crate::windows_monitor_target::resolve_monitor(id)'],
  ['resolver substitutes primary on failure', windowsSource, '.map_err(|error| cap_err("resolve the selected monitor identity", error))', '.or_else(|_| Monitor::primary())'],
  ['native matching uses a display name', windows, '        native_target_path,', '        display_name,'],
  ['WGC substitutes a primary monitor', windowsCapture, '                    m,\n                    CursorCaptureSettings::WithoutCursor', '                    WcMonitor::primary().unwrap(),\n                    CursorCaptureSettings::WithoutCursor'],
] as const) {
  assert.ok(original.includes(before), `mutation fixture exists: ${label}`)
  const changed = original.replace(before, after)
  assert.throws(() => assertWindowsMonitorIdentity(
    original === windowsCapture ? changed : windowsCapture,
    original === windowsSource ? changed : windowsSource,
    original === windows ? changed : windows,
  ), { code: 'ERR_ASSERTION' }, label)
}
assert.match(macosCapture, /macos_capture_target::prepare_capture_target\(cfg, fps\)/,
  'ScreenCaptureKit hands the unchanged capture config to its exact-target owner')
assert.match(macosTarget, /fn select_display[\s\S]*if let Some\(id\) = cfg\.monitor_id\.as_deref\(\)/,
  'the macOS target owner accepts an explicit exact monitor identity')
assert.match(macosTarget, /macos_monitor_target::monitor_id\(display\)\.as_deref\(\) == Some\(id\)/,
  'the macOS target owner re-enumerates and matches only the current opaque identity')
assert.match(macosTarget, /selected display is no longer available; reopen the source picker/,
  'a missing exact macOS display fails closed instead of substituting a fallback')

assert.doesNotMatch(read('../../app/recorder/record-capture/src/windows_picker.rs'), /monitor_id\(monitor\)\?/, 'a missing future id does not hide the current display row')
assert.match(sourceControl, /export interface MonitorInfo \{\s+id\?: string/s, 'the source control types the optional additive id')
assert.match(record, /selectedMonitorCurrent && monitorTargetId[\s\S]*?monitorId: monitorTargetId/, 'Record keeps the selected Doctor identity in its preset')
assert.match(session, /if \(preset\.source\.kind === 'display'\) args\.monitor_id = preset\.source\.monitorId/, 'shared Start returns the opaque monitor identity unchanged')
assert.doesNotMatch(session, /args\.monitor_id = .*(name|width|height|primary)/, 'Start never derives monitor identity from display copy')

const doctor = schema.verbs.find((candidate: { name?: string }) => candidate.name === 'screen_record.doctor')
assert.ok(doctor, 'screen_record.doctor remains public')
assert.match(doctor.result, /monitors:\[\{id\?,index,name,width,height,primary\}\]/, 'schema exposes the additive monitor id shape')
assert.match(doctor.result, /Pass that `id` unchanged as `screen_record\.start\.monitor_id`/i, 'schema owns the exact picker-to-start contract')
const start = schema.verbs.find((candidate: { name?: string }) => candidate.name === 'screen_record.start')
assert.ok(start, 'screen_record.start remains public')
assert.match(start.args.properties.monitor_id.description, /opaque exact native identity/i, 'start accepts the Doctor identity explicitly')
assert.match(start.args.properties.monitor_id.description, /no ordinal.*display-name.*primary-state.*geometry fallback/i, 'start rejects surrogate monitor selection')
assert.doesNotMatch(JSON.stringify(doctor), /screen_record\.select_region/, 'this seam does not claim a Region verb')
assert.match(reference, /Pass that `id` unchanged as `screen_record\.start\{monitor_id\}`/, 'agent reference explains exact start admission')
assert.match(debugApi, /Pass that\s+`id` unchanged as `screen_record\.start\{monitor_id\}`/i, 'public docs explain current exact monitor admission')

console.log('PASS exact native monitor identity contract')
