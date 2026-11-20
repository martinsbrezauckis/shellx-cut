import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

const doctor = {
  ready: true, start_allowed: true, cards: [],
  monitors: [{ id: 'display-A', index: 1, name: 'Primary', width: 1920, height: 1080, primary: true }],
  windows: [], window_capture_supported: true,
  camera: { supported: true, detail: 'Ready', devices: [{ id: 'camera-A', label: 'Camera A', state: 'ready', detail: 'Ready' }] },
}

test('mounted Record keeps failed explicit setup closed against stale Doctor advisories', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-record-doctor-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), {
      name: 'record-doctor-mic-stub', enforce: 'pre',
      resolveId(source, importer) {
        if (source === './MicInputControl' && importer?.endsWith('/Record/index.tsx')) return '\0record-doctor-mic-stub'
      },
      load(id) { if (id === '\0record-doctor-mic-stub') return 'export function MicInputControl() { return null }' },
    }, { name: 'record-doctor-fixture', configureServer(vite) {
      vite.middlewares.use('/__record_doctor__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/record-doctor-lifecycle.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  await page.addInitScript(() => {
    window.__recordUnhandled = []
    window.addEventListener('unhandledrejection', event => window.__recordUnhandled.push(String(event.reason)))
    window.__TAURI__ = {
      core: { invoke: command => command === 'get_record_hotkey_capability'
        ? new Promise(resolve => { window.__resolveInitialHotkey = resolve }) : Promise.resolve({}) },
      event: { listen: async (name, callback) => {
        if (name === 'cut:record-hotkey-capability') window.__hotkeyEvent = callback
        return () => { if (window.__hotkeyEvent === callback) window.__hotkeyEvent = null }
      } },
    }
    // Drive advisory reads with devicechange so the 3s poll cannot race the
    // deliberately held explicit and stale responses in this fixture.
    const setIntervalReal = window.setInterval.bind(window)
    window.setInterval = (callback, delay, ...args) => delay === 3000 ? 0 : setIntervalReal(callback, delay, ...args)
  })
  const doctors = []
  await page.route('**/api/verb/**', route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'screen_record.doctor') { doctors.push(route); return }
    void route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: {} }) })
  })
  const next = async count => {
    for (let i = 0; i < 160; i += 1) {
      if (doctors.length >= count) return doctors[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected Doctor read ${count}, saw ${doctors.length}; ${errors.join('; ')}`)
  }
  const reply = (route, result) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })
  const refresh = page.locator('[data-cut-action="record-source-refresh"]')
  const readiness = page.locator('[data-cut-rec-readiness]')
  const start = page.locator('[data-cut-action="record-start"]')
  const advisory = async () => page.evaluate(() => navigator.mediaDevices?.dispatchEvent(new Event('devicechange')))

  await page.goto(new URL('/__record_doctor__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-panel="record"]').waitFor()
  await page.waitForFunction(() => typeof window.__resolveInitialHotkey === 'function' && typeof window.__hotkeyEvent === 'function')
  await page.evaluate(() => {
    window.__hotkeyEvent({ payload: { schema: 'shellx-cut/record-hotkey-capability@1',
      state: 'registered', scope: 'global', backend: 'native_global_shortcut', enabled: true, can_enable: false, reason: null } })
  })
  await page.waitForFunction(() => document.querySelector('[data-cut-panel="record"]')?.getAttribute('data-cut-record-hotkey-scope') === 'global')
  await page.evaluate(() => window.__resolveInitialHotkey({ schema: 'shellx-cut/record-hotkey-capability@1',
    state: 'configured', scope: 'focused_only', backend: 'none', enabled: true, can_enable: false, reason: null }))
  await page.waitForTimeout(30)
  assert.equal(await page.locator('[data-cut-panel="record"]').getAttribute('data-cut-record-hotkey-scope'), 'global',
    'late initial hotkey read cannot overwrite a newer shell event')
  // A failed native registration (another program holds F9) is told where the shortcut is shown.
  await page.evaluate(() => window.__hotkeyEvent({ payload: { schema: 'shellx-cut/record-hotkey-capability@1',
    state: 'unavailable', scope: 'focused_only', backend: 'native_global_shortcut', enabled: false, can_enable: false,
    reason: 'Global F9 could not be registered: HotKey already registered. Another program may already use F9. F9 still works while ShellX Cut is focused.' } }))
  await page.locator('[data-cut-rec-hotkey-notice]').waitFor({ timeout: 10_000 })
  assert.match(await page.locator('[data-cut-rec-hotkey-notice]').innerText(), /could not be registered.*another program/i)
  await page.evaluate(() => window.__hotkeyEvent({ payload: { schema: 'shellx-cut/record-hotkey-capability@1',
    state: 'registered', scope: 'global', backend: 'native_global_shortcut', enabled: true, can_enable: false, reason: null } }))
  await page.waitForFunction(() => !document.querySelector('[data-cut-rec-hotkey-notice]'))
  await (await next(1)).abort('failed')
  await page.locator('[data-cut-rec-doctor-error]').waitFor()
  assert.equal(await readiness.getAttribute('data-cut-rec-readiness-state'), 'attention')
  assert.equal(await start.isDisabled(), true)
  await refresh.click()
  await reply(await next(2), doctor)
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-readiness]')?.getAttribute('data-cut-rec-readiness-state') === 'ready')
  assert.equal(await page.locator('[data-cut-rec-doctor-error]').count(), 0)

  await advisory()
  const olderAdvisory = await next(3)
  await refresh.click({ noWaitAfter: true })
  const explicitFailure = await next(4)
  await reply(explicitFailure, { ...doctor, cards: null })
  await page.locator('[data-cut-rec-source-refresh-error]').waitFor()
  await page.locator('[data-cut-rec-doctor-error]').waitFor()
  await reply(olderAdvisory, doctor)
  await page.waitForTimeout(50)
  assert.equal(await readiness.getAttribute('data-cut-rec-readiness-state'), 'attention')
  assert.equal(await start.isDisabled(), true, 'older advisory success cannot reopen failed explicit admission')
  await advisory()
  await page.waitForTimeout(50)
  assert.equal(doctors.length, 4, 'advisory reads stay paused behind failed explicit refresh')

  await refresh.click()
  await reply(await next(5), { ...doctor, camera: { supported: true, detail: 'Ready', devices: null } })
  await page.locator('[data-cut-rec-source-refresh-error]').waitFor()
  assert.equal(await start.isDisabled(), true, 'malformed camera devices cannot admit Start')
  await refresh.click()
  await reply(await next(6), doctor)
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-readiness]')?.getAttribute('data-cut-rec-readiness-state') === 'ready')
  assert.equal(await page.locator('[data-cut-rec-source-refresh-error]').count(), 0)
  assert.equal(await page.locator('[data-cut-studio-camera-available]').first().getAttribute('data-cut-studio-camera-available'), 'true')
  assert.equal(await page.locator('[data-cut-rec-source-kind-button="window"]').count(), 1)

  await advisory()
  await (await next(7)).abort('failed')
  await page.locator('[data-cut-rec-doctor-error]').waitFor()
  assert.equal(await start.isDisabled(), true, 'failed device-change read closes stale admission')
  await refresh.click()
  await reply(await next(8), doctor)
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-readiness]')?.getAttribute('data-cut-rec-readiness-state') === 'ready')

  await advisory()
  const oldProject = await next(9)
  await page.locator('[data-test-project-b]').click()
  assert.equal(await page.locator('[data-test-origin]').textContent(), 'origin-B')
  assert.equal(await start.isDisabled(), true, 'same-name origin switch waits for its own Doctor read')
  assert.equal(await page.locator('[data-cut-studio-camera-available]').first().getAttribute('data-cut-studio-camera-available'), 'false')
  assert.equal(await page.locator('[data-cut-rec-source-kind-button="window"]').count(), 0)
  const currentProject = await next(10)
  await reply(oldProject, doctor)
  assert.equal(await start.isDisabled(), true, 'old origin advisory cannot admit new project')
  await reply(currentProject, doctor)
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-readiness]')?.getAttribute('data-cut-rec-readiness-state') === 'ready')

  await advisory()
  const afterUnmount = await next(11)
  await page.locator('[data-test-edit]').click()
  await reply(afterUnmount, doctor)
  await page.waitForTimeout(50)
  assert.equal(await page.locator('[data-cut-panel="record"]').count(), 0)
  assert.deepEqual(await page.evaluate(() => window.__recordUnhandled), [])
  assert.deepEqual(errors, [])
})

// Opening Record, polling and device changes only show readiness. They must
// use the status read, which never starts a capture or a system prompt; the
// full check (a short real capture) runs only from Check or Start.
test('Record shows readiness without a screen capture check until the person asks', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-record-status-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), {
      name: 'record-status-mic-stub', enforce: 'pre',
      resolveId(source, importer) {
        if (source === './MicInputControl' && importer?.endsWith('/Record/index.tsx')) return '\0record-status-mic-stub'
      },
      load(id) { if (id === '\0record-status-mic-stub') return 'export function MicInputControl() { return null }' },
    }, { name: 'record-status-fixture', configureServer(vite) {
      vite.middlewares.use('/__record_status__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/record-doctor-lifecycle.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  await page.addInitScript(() => {
    window.__TAURI__ = {
      core: { invoke: () => Promise.resolve({}) },
      event: { listen: async () => () => {} },
    }
    const setIntervalReal = window.setInterval.bind(window)
    window.setInterval = (callback, delay, ...args) => delay === 3000 ? 0 : setIntervalReal(callback, delay, ...args)
  })
  const doctors = []
  await page.route('**/api/verb/**', route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'screen_record.doctor') { doctors.push(route); return }
    void route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: {} }) })
  })
  const next = async count => {
    for (let i = 0; i < 160; i += 1) {
      if (doctors.length >= count) return doctors[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected Doctor read ${count}, saw ${doctors.length}; ${errors.join('; ')}`)
  }
  const mode = route => route.request().postDataJSON()?.mode ?? 'check'
  const reply = (route, result) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })
  const unchecked = { ...doctor, ready: false, start_allowed: true, screen_check: 'not_checked',
    cards: [{ name: 'screen_capture', status: 'unknown', detail: 'not checked yet' }] }
  const label = () => page.locator('[data-cut-rec-readiness-label]').textContent()

  await page.goto(new URL('/__record_status__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-panel="record"]').waitFor()
  const opened = await next(1)
  assert.equal(mode(opened), 'status', 'opening Record must not run the screen capture check')
  await reply(opened, unchecked)
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-readiness-label]')?.textContent === 'Screen capture not checked yet')
  assert.equal(await page.locator('[data-cut-action="record-capture-check"]').textContent(), 'Check now')
  assert.equal(await page.locator('[data-cut-action="record-start"]').isDisabled(), false, 'Start stays available; it runs the check')

  await page.evaluate(() => navigator.mediaDevices?.dispatchEvent(new Event('devicechange')))
  const deviceChange = await next(2)
  assert.equal(mode(deviceChange), 'status', 'a device change must not run the screen capture check')
  await reply(deviceChange, { ...unchecked, ready: false, start_allowed: false, screen_check: 'permission_needed', monitors: [] })
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-readiness-label]')?.textContent === 'Screen recording permission needed')
  assert.match(await page.locator('[data-cut-rec-readiness-detail]').textContent(), /Privacy & Security > Screen & System Audio Recording/)
  assert.equal(await page.locator('[data-cut-rec-readiness-detail]').isVisible(), true, 'the missing permission is shown, not hidden')

  await page.locator('[data-cut-action="record-capture-check"]').click()
  const asked = await next(3)
  assert.equal(mode(asked), 'check', 'the explicit Check runs the full check')
  await reply(asked, doctor)
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-readiness]')?.getAttribute('data-cut-rec-readiness-state') === 'ready')
  assert.equal(await label(), 'Capture checks passed')
  assert.deepEqual(errors, [])
})
