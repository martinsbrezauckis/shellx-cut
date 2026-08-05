#!/usr/bin/env node
import { strict as assert } from 'node:assert'
import { readFile } from 'node:fs/promises'
import { createRuntimeActionRecorder } from '../../ui/public-tests/lib/fullCoverageRuntimeActionRecorder.mjs'
import { nativeWindowSizeForViewport } from '../../ui/public-tests/lib/webdriverIoPage.mjs'

let adapterListener = null
const adapterPage = {
  on(name, listener) {
    if (name === 'action') adapterListener = listener
  },
}
const adapterRecorder = await createRuntimeActionRecorder(
  adapterPage,
  ['settings-open', 'settings-open-alias', 'library-open'],
)
assert.equal(typeof adapterListener, 'function')
adapterListener({ actions: ['dynamic-value', 'settings-open', 'settings-open-alias'] })
adapterListener({ actions: ['library-open'] })
adapterListener({ actions: ['unexpected-action'] })
assert.deepEqual(adapterRecorder.observed(), ['library-open', 'settings-open', 'settings-open-alias'])
assert.deepEqual(adapterRecorder.unexpected(), ['unexpected-action'])
assert.deepEqual(
  adapterRecorder.ids(),
  ['library-open', 'settings-open', 'settings-open-alias', 'unexpected-action'],
)

let exposed = null
let initScript = null
const playwrightPage = {
  async exposeFunction(name, callback) {
    assert.equal(name, '__shellxCutRecordRuntimeAction')
    exposed = callback
  },
  async addInitScript(callback) {
    initScript = callback
  },
}
const playwrightRecorder = await createRuntimeActionRecorder(
  playwrightPage,
  ['export-saveas-option', 'export-saveas-option-alias', 'generated-compare-backdrop'],
)
assert.equal(typeof exposed, 'function')
assert.equal(typeof initScript, 'function')
exposed(['dynamic-export-id', 'export-saveas-option', 'export-saveas-option-alias'])
assert.deepEqual(playwrightRecorder.observed(), ['export-saveas-option', 'export-saveas-option-alias'])

const previousWindow = globalThis.window
const previousDocument = globalThis.document
const previousElement = globalThis.Element
const documentListeners = new Map()
class FakeElement {}
const backdrop = new FakeElement()
backdrop.closest = (selector) => selector === '[data-cut-action]' ? backdrop : null
backdrop.getAttribute = (name) => name === 'data-cut-action' ? 'generated-compare-backdrop' : null
globalThis.Element = FakeElement
globalThis.window = {
  __shellxCutRecordRuntimeAction: exposed,
}
globalThis.document = {
  addEventListener(name, listener) {
    documentListeners.set(name, listener)
  },
}
try {
  initScript()
  assert.equal(typeof documentListeners.get('mousedown'), 'function')
  documentListeners.get('mousedown')({ target: backdrop })
  assert.deepEqual(
    playwrightRecorder.observed(),
    ['export-saveas-option', 'export-saveas-option-alias', 'generated-compare-backdrop'],
  )
} finally {
  if (previousWindow === undefined) delete globalThis.window
  else globalThis.window = previousWindow
  if (previousDocument === undefined) delete globalThis.document
  else globalThis.document = previousDocument
  if (previousElement === undefined) delete globalThis.Element
  else globalThis.Element = previousElement
}

const webdriverAdapter = await readFile(
  new URL('../../ui/public-tests/lib/webdriverIoPage.mjs', import.meta.url),
  'utf8',
)
assert.match(webdriverAdapter, /document[.]addEventListener[(]'click', record, true[)]/)
assert.match(webdriverAdapter, /document[.]addEventListener[(]'input', record, true[)]/)
assert.match(webdriverAdapter, /document[.]addEventListener[(]'change', record, true[)]/)
assert.match(webdriverAdapter, /state[.]events[.]push[(][{] type: 'action', actions [}][)]/)
assert.match(
  webdriverAdapter,
  /new URL[(]observedRequestUrl, document[.]baseURI[)][.]href/,
  'native request evidence normalizes relative fetch URLs before payload assertions consume it',
)
const retinaMetrics = {
  innerWidth: 1440,
  innerHeight: 869,
  outerWidth: 1440,
  outerHeight: 900,
  devicePixelRatio: 2,
}
assert.deepEqual(
  nativeWindowSizeForViewport({ width: 1440, height: 869 }, retinaMetrics, 'embedded'),
  { width: 2880, height: 1800 },
  'embedded WKWebView converts requested CSS viewport dimensions to Retina backing pixels',
)
assert.deepEqual(
  nativeWindowSizeForViewport({ width: 1440, height: 869 }, retinaMetrics, 'external'),
  { width: 1440, height: 900 },
  'external WebDrivers retain logical window dimensions',
)

console.log('PASS runtime-action-recorder')
