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
  monitors: [{ id: 'display-A', index: 1, primary: true, name: 'Primary display' }],
  windows: [], window_capture_supported: true,
  camera: { supported: false, detail: 'Camera unavailable', devices: [] },
  pause: { supported: true, detail: 'Pause available', incompatible: [] },
}
const unavailable = (keys = false) => ({ state: 'unavailable', backend: 'rdevin_windows', reason: 'startup_failed', capture_keys: keys })
const warning = '[data-cut-rec-input-hook-warning]'

async function fixture(t, { raw = true, keys = false } = {}) {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-record-hook-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'record-input-hook-fixture', configureServer(vite) {
      vite.middlewares.use('/__record_input_hook__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}; window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/record-handoff.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen(); t.after(() => server.close())
  const browser = await chromium.launch({ headless: true }); t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1100, height: 850 } })
  const errors = []; const requests = []
  let take = 0, hook, stopHook, statusId
  page.on('pageerror', error => errors.push(String(error)))
  await page.addInitScript(({ raw, keys }) => localStorage.setItem('shellx-cut.recording-preset.v1', JSON.stringify({
    schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
    fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
    keys, raw, studio: { background: 'gradient' },
  })), { raw, keys })
  await page.route('**/api/verb/**', async route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    requests.push({ name, args: route.request().postDataJSON() })
    if (name === 'screen_record.start') take += 1
    const result = name === 'screen_record.doctor' ? doctor
      : name === 'screen_record.start' ? { capture_id: `take-${take}`, pause: { enabled: false } }
      : name === 'screen_record.status' ? { capture_id: statusId ?? `take-${take}`, terminal: false, input_hook: hook }
      : name === 'screen_record.stop' ? { capture_id: `take-${take}`, raw_path: '/fixture/raw.mp4', source: '/fixture/source.mp4', plan: '/fixture/plan.json', input_hook: stopHook }
      : name === 'screen_record.studio_event' ? { last_event: route.request().postDataJSON().event }
      : name === 'screen_record.polish' ? { clip_id: `clip-${take}` } : {}
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })
  })
  await page.goto(new URL('/__record_input_hook__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-panel="record"]').waitFor()
  const start = async () => {
    await page.locator('[data-test-edit]').click()
    await page.keyboard.press('F9')
    await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
    await page.locator('[data-test-record]').click()
  }
  const nextStatus = async (value, expectedState) => {
    // Record audio meters also poll Status; request entry cannot prove that the
    // recording-session publisher consumed this payload or React committed it.
    const observed = page.waitForResponse(async response => {
      if (!response.url().endsWith('/screen_record.status')) return false
      const reply = await response.json()
      return reply.result?.capture_id === (statusId ?? `take-${take}`)
        && JSON.stringify(reply.result?.input_hook) === JSON.stringify(value)
    }, { timeout: 5000 })
    hook = value
    await observed
    if (expectedState) await page.waitForFunction(expected =>
      document.querySelector('[data-test-input-hook-state]')?.textContent === expected,
    expectedState, { timeout: 5000 })
  }
  return { page, errors, requests, start, nextStatus,
    setStopHook: value => { stopHook = value }, setStatusId: value => { statusId = value } }
}

test('legacy, malformed, unobserved and registered with zero events do not claim startup failure', async t => {
  const f = await fixture(t)
  await f.start()
  for (const value of [undefined, { state: 'unavailable' },
    { ...unavailable(), capture_keys: 'yes' }, { ...unavailable(), untrusted: true }, [], { ...unavailable(), backend: 'wayland_evdev' },
    { state: 'unobserved', backend: 'wayland_evdev', reason: null, capture_keys: false },
    { state: 'registered', backend: 'rdevin_windows', reason: null, capture_keys: false }]) {
    await f.nextStatus(value, value?.state === 'registered' ? 'registered' : undefined)
    assert.equal(await f.page.locator(warning).count(), 0)
    assert.equal(await f.page.locator('[data-test-phase]').textContent(), 'recording')
  }
  assert.equal(await f.page.locator('[data-test-input-hook-state]').textContent(), 'registered', 'actual null-reason acknowledgment is retained')
  await f.page.locator('[data-test-stop]').click()
  await f.page.locator('[data-cut-rec-result="saved"]').waitFor()
  assert.equal(await f.page.locator('[data-test-input-hook-state]').textContent(), 'unobserved', 'legacy Stop becomes unobserved')
  assert.equal(await f.page.locator(warning).count(), 0, 'legacy Stop stays unobserved')
  assert.deepEqual(f.errors, [])
})

for (const raw of [true, false]) test(`startup failure warns during capture and preserves successful ${raw ? 'Raw' : 'Polished'} delivery`, async t => {
  const f = await fixture(t, { raw, keys: !raw })
  await f.start()
  await f.nextStatus(unavailable(!raw), 'unavailable')
  await f.page.locator(warning).waitFor()
  const live = await f.page.locator(warning).textContent()
  assert.match(live, /Mouse input could not start.*pointer animation, click highlights and automatic zoom.*Video recording continues\./)
  assert.equal(live.includes('key capture'), !raw)
  f.setStopHook(unavailable(!raw))
  await f.page.locator('[data-test-stop]').click()
  await f.page.locator('[data-cut-rec-result="saved"]').waitFor()
  assert.match(await f.page.locator(warning).textContent(), /Video saved\./)
  assert.equal(await f.page.locator('[data-cut-action="record-new-take"]').isEnabled(), true)
  assert.equal(await f.page.locator(raw ? '[data-cut-action="record-add-raw"]' : '[data-cut-action="record-export"]').isEnabled(), true)
  assert.equal(await f.page.locator(raw ? '[data-cut-action="record-save-copy"]' : '[data-cut-action="record-save-raw-copy"]').isEnabled(), true)
  assert.equal(f.requests.filter(r => r.name === 'screen_record.polish').length, raw ? 0 : 1)
  const box = await f.page.locator(warning).boundingBox()
  assert.ok(box && box.width > 100 && box.height > 0)
  assert.equal(await f.page.locator(warning).evaluate(el => el.scrollWidth <= el.clientWidth), true, 'warning wraps without overflow')
  if (process.env.CUT_HOOK_SCREENSHOT) await f.page.screenshot({ path: `${process.env.CUT_HOOK_SCREENSHOT}-${raw ? 'raw' : 'polished'}.png`, fullPage: true })
  await f.page.locator('[data-cut-action="record-new-take"]').click()
  assert.equal(await f.page.locator(warning).count(), 0, 'new take clears observation')
  assert.deepEqual(f.errors, [])
})

test('another capture status cannot publish its startup failure', async t => {
  const f = await fixture(t)
  await f.start()
  f.setStatusId('foreign-take')
  await f.nextStatus(unavailable())
  await f.page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recovery')
  assert.equal(await f.page.locator(warning).count(), 0)
  assert.deepEqual(f.errors, [])
})
