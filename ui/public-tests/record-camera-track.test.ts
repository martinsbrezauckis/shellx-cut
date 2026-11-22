import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import * as presets from '../src/app/recordingPreset'
import { CameraControl } from '../src/panels/Record/CameraControl'
import { defaultStudioState } from '../src/panels/Record/studioTypes'

const preset: presets.RecordingPreset = { schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'one' }, fps: 30, durationMs: null, audio: false, systemAudio: false, keys: false, raw: false }
for (const mode of process.argv[2] ? [process.argv[2]] : ['default', 'storage', 'builder', 'export']) {
if (mode === 'storage') {
  const data = new Map<string, string>()
  Object.assign(globalThis, { localStorage: { getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => data.set(key, value) } })
  presets.saveRecordingPreset({ ...preset, cameraSeparate: false } as presets.RecordingPreset)
  assert.equal((presets.loadRecordingPreset() as any)?.cameraSeparate, false, 'camera preference survives stored settings reload')
} else if (mode === 'builder') {
  const build = (presets as any).recordingPolishArgs
  assert.equal(typeof build, 'function', 'pure polish argument builder exists')
  assert.deepEqual(build('take', 'plan', { ...preset, cameraId: 'cam' }), { source: 'take', plan: 'plan', raw: false, camera: 'separate' })
  assert.equal(build('take', 'plan', { ...preset, cameraId: 'cam', cameraSeparate: false }).camera, 'baked')
  assert.equal(Object.hasOwn(build('take', 'plan', preset), 'camera'), false)
} else if (mode === 'export') {
  const source = readFileSync(new URL('../src/panels/Record/RecordingResultPanel.tsx', import.meta.url), 'utf8')
  assert.match(source, /Export as recorded/)
  assert.ok(source.includes('Edits you make later in Edit are not in this file. Export from Edit to include them.'))
} else {
  const props = { capability: { supported: true, detail: '', devices: [{ id: 'cam', label: 'Camera', state: 'ready' as const, detail: '' }] }, enabled: true, deviceId: 'cam', disabled: false, rawCapture: false, layout: defaultStudioState().camera, liveAdjustDisabled: false, onEnabled() {}, onDevice() {}, onPosition() {}, onShape() {}, onSize() {}, onReset() {} }
  const html = renderToStaticMarkup(createElement(CameraControl, props))
  assert.match(html, /data-cut-rec-camera-separate[^>]*checked=""/, 'own camera track defaults on')
  const off = renderToStaticMarkup(createElement(CameraControl, { ...props, enabled: false }))
  assert.match(off, /data-cut-rec-camera-separate[^>]*disabled=""/)
  assert.match(off, /Turn on Camera to edit it later/)
}
console.log(`PASS camera track ${mode}`)
}
