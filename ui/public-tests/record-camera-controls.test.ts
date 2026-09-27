import { strict as assert } from 'node:assert'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import {
  CameraControl, cameraLayoutUnavailableReason,
  type CameraCapability,
} from '../src/panels/Record/CameraControl'
import { defaultStudioState } from '../src/panels/Record/studioTypes'

const ready: CameraCapability = {
  supported: true,
  detail: 'Camera sources available.',
  devices: [{ id: 'front', label: 'Front camera', state: 'ready', detail: 'Ready to record.' }],
}
const layout = defaultStudioState().camera
const callbacks = {
  onEnabled: () => {}, onDevice: () => {}, onPosition: () => {},
  onShape: () => {}, onSize: () => {}, onReset: () => {},
}
const render = (overrides: Partial<Parameters<typeof CameraControl>[0]> = {}) => renderToStaticMarkup(createElement(CameraControl, {
  capability: ready, enabled: true, deviceId: 'front', disabled: false,
  rawCapture: false, layout, liveAdjustDisabled: false, ...callbacks, ...overrides,
}))

const supported = render()
for (const corner of ['top_left', 'top_right', 'bottom_right', 'bottom_left']) {
  assert.match(supported, new RegExp(`data-cut-rec-camera-position="${corner}"`), `${corner} is directly visible`)
}
assert.match(supported, /data-cut-rec-camera-shape="circle"/)
assert.match(supported, /data-cut-rec-camera-shape="rounded_rect"/)
assert.match(supported, /data-cut-rec-camera-size-value="true">22%<\/output>/)
assert.match(supported, /data-cut-action="record-camera-layout-reset"/)
assert.doesNotMatch(supported, /data-cut-rec-camera-layout-reason/)
assert.equal(cameraLayoutUnavailableReason(ready, true, 'front', false, false), null)

const unsupportedDetail = 'Camera capture is unavailable on this host.'
const unsupported = render({ capability: { supported: false, devices: [], detail: unsupportedDetail } })
assert.match(unsupported, /data-cut-rec-camera-layout-disabled="true"/)
assert.match(unsupported, /Camera capture is unavailable on this host\./)
assert.doesNotMatch(unsupported, /data-cut-rec-camera-shape=/)
assert.doesNotMatch(unsupported, /data-cut-rec-camera-position=/)
assert.doesNotMatch(unsupported, /data-cut-rec-camera-size=/)

const busy = { ...ready, devices: [{ ...ready.devices[0], state: 'busy' as const, detail: 'Camera is in use by another app.' }] }
assert.equal(cameraLayoutUnavailableReason(busy, true, 'front', false, false), 'Camera is in use by another app.')
assert.equal(cameraLayoutUnavailableReason(ready, true, 'gone', false, false), 'Choose an available camera source to adjust its layout.')
assert.equal(cameraLayoutUnavailableReason(ready, false, 'front', false, false), 'Turn on Camera to adjust its layout.')
assert.equal(cameraLayoutUnavailableReason(ready, true, 'front', true, false), 'Switch to Auto-edit to adjust camera layout.')
assert.equal(cameraLayoutUnavailableReason(ready, true, 'front', false, true), 'Wait for the current capture operation to finish.')

const unavailable = render({ capability: busy })
assert.match(unavailable, /Camera is in use by another app\./)
assert.match(unavailable, /data-cut-rec-camera-shape="circle"[^>]*disabled=""/)
assert.match(unavailable, /data-cut-action="record-camera-layout-reset"[^>]*disabled=""/)

console.log('PASS visible camera layout controls and exact unavailable reasons')
