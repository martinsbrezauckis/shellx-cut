// The update-surface fixture stages window.__TAURI__ / __TAURI_INTERNALS__ so
// the three update rows can be actuated deterministically. It therefore holds a
// dangerous power: it can leave the page WITHOUT the shell bridge. isTauri()
// (ui/src/lib/tauri.ts) is `!!window.__TAURI__`, so a bridge the fixture
// removed turns every desktop-only helper into a silent no-op for the rest of
// the sweep — which is exactly how `settings-ffmpeg-change` failed with "no
// native dialog appeared" on macOS and Windows on 2026-08-06 while the product
// was never reached.
//
// These tests drive the real module against a controlled window for each of the
// three shell shapes and assert the bridge is left exactly as it was found.
import assert from 'node:assert/strict'
import test from 'node:test'
import { createSettingsUpdateCoverage } from '../../ui/public-tests/lib/fullCoverageSettingsUpdate.mjs'

/**
 * Run the update-surface coverage against a fake page whose `evaluate` executes
 * the passed function directly against `fakeWindow`.
 * @param {object} fakeWindow window shape under test (frozen where the real shell freezes)
 * @returns {Promise<void>}
 */
async function runCoverage(fakeWindow) {
  const noopLocator = () => ({
    first: () => noopLocator(),
    click: async () => {},
    count: async () => 0,
    textContent: async () => '',
    getAttribute: async () => null,
    isDisabled: async () => false,
    waitFor: async () => {},
  })
  const page = {
    locator: noopLocator,
    evaluate: async (fn, ...args) => fn(...args),
  }
  const coverage = createSettingsUpdateCoverage({
    // probe never runs its doClick here: the bridge lifecycle is what is under
    // test, and the rows themselves are proven on a rig against a staged feed.
    probe: async () => {},
    sleep: async () => {},
    openSettings: async () => {},
    waitFor: async () => {},
  })
  const saved = {
    window: globalThis.window,
    document: globalThis.document,
    CustomEvent: globalThis.CustomEvent,
  }
  try {
    globalThis.window = fakeWindow
    globalThis.document = { dispatchEvent: () => {} }
    globalThis.CustomEvent = class { constructor(type) { this.type = type } }
    await coverage(page, noopLocator(), 'settings')
  } finally {
    for (const [name, value] of Object.entries(saved)) {
      if (value === undefined) delete globalThis[name]
      else globalThis[name] = value
    }
  }
}

// RED-PROOF for settings-ffmpeg-change (macOS + Windows, 2026-08-06). Before the
// fix this test failed: the sealed shell patches nothing, so restore took the
// unconditional else-branch and deleted the real window.__TAURI__.
test('a sealed native bridge is left intact — the fixture never deletes what it did not create', async () => {
  const realInvoke = async () => ({})
  const bridge = Object.freeze({
    core: Object.freeze({ invoke: realInvoke }),
    event: Object.freeze({ listen: async () => () => {} }),
  })
  const fakeWindow = {
    __TAURI__: bridge,
    // Tauri 2.11 seals this in native shells (WKWebView + WebView2).
    __TAURI_INTERNALS__: Object.freeze({ invoke: realInvoke }),
  }
  await runCoverage(fakeWindow)
  assert.equal(fakeWindow.__TAURI__, bridge, 'the real shell bridge survives the fixture')
  assert.equal(fakeWindow.__TAURI_INTERNALS__.invoke, realInvoke, 'the real dispatcher survives too')
  assert.equal(fakeWindow.__fcvUpdateFixture, undefined, 'the fixture handle is cleaned up')
})

test('an unfrozen shell bridge is restored to the exact dispatcher that was patched', async () => {
  const realInvoke = async () => ({})
  const bridge = { core: { invoke: realInvoke }, event: { listen: async () => () => {} } }
  const fakeWindow = { __TAURI__: bridge, __TAURI_INTERNALS__: { invoke: realInvoke } }
  await runCoverage(fakeWindow)
  assert.equal(fakeWindow.__TAURI__, bridge, 'the bridge object is untouched')
  assert.equal(fakeWindow.__TAURI_INTERNALS__.invoke, realInvoke, 'the patched dispatcher is put back')
})

test('a browser stub the fixture created is the only bridge it removes', async () => {
  const fakeWindow = {}
  await runCoverage(fakeWindow)
  assert.equal(fakeWindow.__TAURI__, undefined, 'the fixture cleans up its own stub')
})
