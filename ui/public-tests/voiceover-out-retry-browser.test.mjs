import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('finite Voiceover Out retries exact observations and retires stale owners', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-voiceover-out-retry-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'voiceover-out-fixture', configureServer(vite) {
      vite.middlewares.use('/__voiceover_out_retry__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/voiceover-out-retry-browser.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  await page.addInitScript(() => {
    window.__outStops = 0
    window.__unhandled = []
    document.addEventListener('cut:voiceover-stop', () => { window.__outStops += 1 })
    window.addEventListener('unhandledrejection', event => window.__unhandled.push(String(event.reason)))
  })
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  const observed = []
  const ticks = []
  await page.route('**/api/verb/**', route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'voiceover.observe_playhead') { observed.push(route); return }
    if (name === 'voiceover.tick') { ticks.push(route); return }
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: {} }) })
  })
  const waitFor = async (items, count) => {
    for (let i = 0; i < 200; i += 1) {
      if (items.length >= count) return items[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected request ${count}, saw ${items.length}; page errors: ${errors.join('; ')}`)
  }
  const reply = (route, body) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) })
  const status = (id, phase) => ({ ok: true, result: {
    request_id: id, phase, owner_claim: { session_id: `session-${id}`, capability: `capability-${id}` },
  } })
  const placed = id => ({ ok: true, result: {
    schema: 'shellx-cut/voiceover-timeline/1', request_id: id, phase: 'placed', terminal: 'saved',
    placement: { asset_id: 'asset-1', clip_id: 'clip-1', op_id: 'op_1', already_applied: false },
  } })
  const empty = id => ({ ok: true, result: {
    schema: 'shellx-cut/voiceover-timeline/1', request_id: id, phase: 'finished', terminal: 'zero_samples',
  } })
  const start = async (id, epoch = 1) => page.evaluate(({ id, epoch }) => {
    document.dispatchEvent(new CustomEvent('cut:voiceover-playback', { detail: {
      request_id: id, request_fingerprint: 'a'.repeat(64), bridge_epoch: epoch,
      start_ms: 1_000, out_ms: 1_000, owner_session_id: `session-${id}`,
      owner_capability: `capability-${id}`,
    } }))
  }, { id, epoch })
  const stops = () => page.evaluate(() => window.__outStops)

  await page.goto(new URL('/__voiceover_out_retry__', server.resolvedUrls.local[0]).href)
  await start('take-a')
  const first = await waitFor(observed, 1)
  assert.deepEqual(first.request().postDataJSON(), {
    owner_session_id: 'session-take-a', owner_capability: 'capability-take-a',
    request_id: 'take-a', request_fingerprint: 'a'.repeat(64), bridge_epoch: 1, playhead_ms: 1_000,
  })
  await first.abort('failed')
  await reply(await waitFor(ticks, 1), status('take-a', 'recording'))
  await page.evaluate(() => window.voiceoverFixtureRerender())
  await reply(await waitFor(observed, 2), placed('take-a'))
  await page.waitForFunction(() => window.__outStops === 1)
  assert.equal(await page.locator('[data-test-rate]').textContent(), '0', 'only accepted terminal truth stops local playback')

  await start('take-b')
  await reply(await waitFor(observed, 3), { ok: false, error: { code: 'unavailable' } })
  await reply(await waitFor(ticks, 2), status('take-other', 'finishing'))
  assert.equal(await stops(), 1, 'foreign terminal status cannot stop current take')
  await reply(await waitFor(observed, 4), empty('take-b'))
  await page.waitForFunction(() => window.__outStops === 2)

  await start('take-c')
  await (await waitFor(observed, 5)).abort('failed')
  await reply(await waitFor(ticks, 3), status('take-c', 'finishing'))
  await page.waitForFunction(() => window.__outStops === 3)
  assert.equal(observed.length, 5, 'accepted response loss reconciles by tick without another Out submission')

  await start('take-epoch', 1)
  const old = await waitFor(observed, 6)
  await start('take-epoch', 2)
  await reply(old, status('take-epoch', 'finishing'))
  const replacement = await waitFor(observed, 7)
  assert.equal(await stops(), 3, 'late accepted old epoch cannot stop a replacement with the same request ID')
  assert.equal(replacement.request().postDataJSON().bridge_epoch, 2)
  await reply(replacement, status('take-epoch', 'finishing'))
  await page.waitForFunction(() => window.__outStops === 4)

  await start('take-manual')
  const manual = await waitFor(observed, 8)
  await page.evaluate(() => document.dispatchEvent(new CustomEvent('cut:voiceover-stop')))
  await reply(manual, status('take-manual', 'finishing'))
  await page.waitForTimeout(320)
  assert.equal(await stops(), 5, 'manual Stop retires held automatic response and timer')

  await start('take-project')
  const project = await waitFor(observed, 9)
  await page.evaluate(() => window.voiceoverFixtureProject())
  await reply(project, status('take-project', 'finishing'))
  await page.waitForTimeout(320)
  assert.equal(await stops(), 5, 'project-keyed remount retires old request')

  await start('take-exhaust')
  for (let attempt = 0; attempt < 5; attempt += 1) {
    await (await waitFor(observed, 10 + attempt)).abort('failed')
    await reply(await waitFor(ticks, 4 + attempt), status('take-exhaust', 'recording'))
  }
  await page.locator('[data-cut-voiceover-out-unconfirmed]').waitFor()
  const finalCount = observed.length
  await page.waitForTimeout(2_200)
  assert.equal(observed.length, finalCount, 'finite retry budget cannot spin at a clamped Out')
  assert.match(await page.locator('[data-cut-voiceover-out-unconfirmed]').textContent(), /Stop or Cancel/)
  await start('take-owner')
  const wrongOwner = status('take-owner', 'finishing')
  wrongOwner.result.owner_claim.capability = 'foreign-capability'
  await reply(await waitFor(observed, 15), wrongOwner)
  assert.equal(await stops(), 5, 'same-ID terminal for a foreign owner cannot stop playback')
  const ownerlessFinishing = status('take-owner', 'finishing')
  delete ownerlessFinishing.result.owner_claim
  await reply(await waitFor(ticks, 9), ownerlessFinishing)
  await reply(await waitFor(observed, 16), status('take-owner', 'finishing'))
  await page.waitForFunction(() => window.__outStops === 6)
  assert.equal(await page.locator('[data-cut-voiceover-out-unconfirmed]').count(), 0, 'new owner clears exhausted warning')
  await start('take-timeout')
  await waitFor(observed, 17) // Leave this response unresolved: the request timeout must release the serial gate.
  await reply(await waitFor(ticks, 10), status('take-timeout', 'recording'))
  await reply(await waitFor(observed, 18), status('take-timeout', 'finishing'))
  await page.waitForFunction(() => window.__outStops === 7)
  await start('take-pause')
  const pause = await waitFor(observed, 19)
  await page.evaluate(() => window.voiceoverFixturePause())
  await page.waitForFunction(() => document.querySelector('[data-test-rate]')?.textContent === '0')
  await reply(pause, status('take-pause', 'finishing'))
  await page.waitForTimeout(320)
  assert.equal(await stops(), 7, 'paused transport retires an in-flight automatic response')
  await page.evaluate(() => window.voiceoverFixtureUnmount())
  await page.locator('[data-test-owner]').waitFor({ state: 'detached' })
  assert.deepEqual(await page.evaluate(() => window.__unhandled), [])
  assert.deepEqual(errors, [])
})
