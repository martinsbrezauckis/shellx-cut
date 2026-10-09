import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const MINIMUM = [2, 0, 1]

function parseVersion(value) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(value)
  assert.ok(match, `expected an exact three-part version, got ${value}`)
  return match.slice(1).map(Number)
}

function atLeast(actual, minimum) {
  for (let index = 0; index < minimum.length; index += 1) {
    if (actual[index] !== minimum[index]) return actual[index] > minimum[index]
  }
  return true
}

function assertLifecycleFloor(manifest, lockfile) {
  const requirement = manifest.match(/windows-capture\s*=\s*\{\s*version\s*=\s*"([^"]+)"/)
  assert.ok(requirement, 'record-capture must declare the windows-capture dependency version')
  assert.ok(
    atLeast(parseVersion(requirement[1]), MINIMUM),
    'windows-capture must retain the 2.0.1 lifecycle-fix floor',
  )

  const locked = lockfile.match(
    /\[\[package\]\]\s*\nname = "windows-capture"\s*\nversion = "([^"]+)"/,
  )
  assert.ok(locked, 'Cargo.lock must contain windows-capture')
  assert.ok(
    atLeast(parseVersion(locked[1]), MINIMUM),
    'Cargo.lock must not resolve windows-capture below 2.0.1',
  )
}

test('Windows capture dependency retains the upstream lifecycle-fix floor', () => {
  const manifest = readFileSync(
    new URL('../../app/recorder/record-capture/Cargo.toml', import.meta.url),
    'utf8',
  )
  const lockfile = readFileSync(new URL('../../app/Cargo.lock', import.meta.url), 'utf8')
  assertLifecycleFloor(manifest, lockfile)
})

test('Windows capture lifecycle guard rejects the unsafe 2.0.0 resolution', () => {
  assert.throws(
    () => assertLifecycleFloor(
      '[target.\'cfg(windows)\'.dependencies]\nwindows-capture = { version = "2.0.0", optional = true }\n',
      '[[package]]\nname = "windows-capture"\nversion = "2.0.0"\n',
    ),
    /lifecycle-fix floor/,
  )
})

test('Windows capture retains one process MTA usage while WinRT factory caches exist', () => {
  const runtime = readFileSync(
    new URL('../../app/recorder/record-capture/src/windows_runtime.rs', import.meta.url),
    'utf8',
  )
  const probe = readFileSync(
    new URL('../../app/recorder/record-capture/src/windows_probe.rs', import.meta.url),
    'utf8',
  )
  const capture = readFileSync(
    new URL('../../app/recorder/record-capture/src/windows.rs', import.meta.url),
    'utf8',
  )
  assert.match(runtime, /static PROCESS_MTA_PIN: OnceLock/)
  assert.match(runtime, /CoIncrementMTAUsage[(][)]/)
  assert.doesNotMatch(runtime, /use .*CoDecrementMTAUsage/)
  assert.doesNotMatch(runtime, /unsafe \{ CoDecrementMTAUsage/)
  assert.match(probe, /windows_runtime::pin_process_mta[(][)]/)
  assert.match(capture, /windows_runtime::pin_process_mta[(][)]/)
})

function assertWindowCursorPolicy(windows, macTarget, coordinates) {
  assert.match(windows, /Src::Monitor\(m\)[\s\S]*?WcSettings::new\(\s*m,\s*CursorCaptureSettings::WithoutCursor,/,
    'monitor must keep the synthetic-cursor path')
  assert.match(windows, /Src::Window\(win\)[\s\S]*?WcSettings::new\(\s*win,\s*CursorCaptureSettings::WithCursor,/,
    'window must retain native cursor pixels')
  assert.match(macTarget, /recording_stream_config\(\s*width,\s*height,\s*fps,\s*cfg\.capture_cursor \|\| cfg\.window\.is_some\(\),?\s*\)/,
    'macOS window fallback must preserve explicit display cursor configuration')
  assert.match(coordinates, /fn unavailable_window_rdevin_input\([\s\S]*?mark_unavailable\(clicks\);\s*unavailable_output\(/,
    'window cursor pixels must not turn unavailable input into synthetic coordinates')
  assert.match(coordinates, /fn unavailable_output\([\s\S]*?cursor: vec!\[\],[\s\S]*?state: CursorCoordinateState::Unavailable/)
}

const captureSource = (leaf) => readFileSync(
  new URL(`../../app/recorder/record-capture/src/${leaf}`, import.meta.url), 'utf8',
)

test('native window cursor fallback preserves monitor policy and empty synthetic window tracks', () => {
  assertWindowCursorPolicy(captureSource('windows.rs'), captureSource('macos_capture_target.rs'),
    captureSource('surface_coordinates.rs'))
  const macTarget = captureSource('macos_capture_target.rs')
  assert.match(macTarget, /prepare_region_capture_from_config\(cfg, fps\)/)
  assert.match(captureSource('macos.rs'), /with_shows_cursor\(capture_cursor\)/)
  for (const leaf of ['windows.rs', 'macos.rs']) {
    assert.match(captureSource(leaf), /surface_coordinates::unavailable_window_rdevin_input/)
  }
  for (const leaf of ['windows_window_clicks.rs', 'macos_window_clicks.rs']) {
    assert.match(captureSource(leaf), /cursor: vec!\[\]/,
      'exact window clicks must not add a second synthetic cursor')
  }
})

test('cursor policy rejects hidden window cursor, double monitor cursor, and fabricated coordinates', () => {
  const windows = captureSource('windows.rs')
  const macTarget = captureSource('macos_capture_target.rs')
  const coordinates = captureSource('surface_coordinates.rs')
  assert.throws(() => assertWindowCursorPolicy(windows.replace('CursorCaptureSettings::WithCursor',
    'CursorCaptureSettings::WithoutCursor'), macTarget, coordinates), /window must retain/)
  assert.throws(() => assertWindowCursorPolicy(windows.replace('CursorCaptureSettings::WithoutCursor',
    'CursorCaptureSettings::WithCursor'), macTarget, coordinates), /monitor must keep/)
  assert.throws(() => assertWindowCursorPolicy(windows, macTarget.replace('cfg.capture_cursor || cfg.window.is_some()',
    'cfg.capture_cursor'), coordinates), /preserve explicit display/)
  assert.throws(() => assertWindowCursorPolicy(windows, macTarget,
    coordinates.replace('cursor: vec![]', 'cursor: fabricated')), /cursor: vec/)
})
