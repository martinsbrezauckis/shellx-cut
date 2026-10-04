import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('mounted Assets smart bins recover reads without cross-project or stale-list paint', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-assets-smart-bin-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'assets-smart-bin-fixture', configureServer(vite) {
      vite.middlewares.use('/__assets_smart_bins__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/assets-smart-bins-browser.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  await page.addInitScript(() => {
    window.__unhandled = []
    window.addEventListener('unhandledrejection', event => window.__unhandled.push(String(event.reason)))
    window.prompt = () => 'saved-B'
  })
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  const reads = []
  const saves = []
  await page.route('**/api/verb/**', route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'media.bin_list') { reads.push(route); return }
    if (name === 'media.bin_save') { saves.push(route); return }
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: {} }) })
  })
  const waitFor = async (items, count) => {
    for (let i = 0; i < 180; i += 1) {
      if (items.length >= count) return items[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected request ${count}, saw ${items.length}; page errors: ${errors.join('; ')}`)
  }
  const reply = (route, body) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) })
  const listed = (...names) => ({ ok: true, result: { bins: names.map(name => ({ name, kind: 'video', match_count: 1, matches: [] })) } })

  await page.goto(new URL('/__assets_smart_bins__', server.resolvedUrls.local[0]).href)
  await (await waitFor(reads, 1)).abort('failed')
  await page.locator('[data-cut-bin-unavailable]').waitFor()
  assert.equal(await page.locator('[data-cut-bin]').count(), 0, 'failed first read is unavailable, not an empty server list')
  await page.locator('[data-cut-action="bin-retry"]').click()
  const oldA = await waitFor(reads, 2)
  await page.locator('[data-test-b]').click()
  await reply(await waitFor(reads, 3), listed('B-bin'))
  await page.locator('[data-cut-bin="B-bin"]').waitFor()
  await reply(oldA, listed('A-bin'))
  await page.waitForTimeout(60)
  assert.equal(await page.locator('[data-cut-bin="A-bin"]').count(), 0, 'late A read cannot paint B')
  await page.locator('[data-test-revision]').click()
  await page.waitForTimeout(60)
  assert.equal(reads.length, 3, 'ordinary revision does not restart a smart-bin read')

  await page.locator('[data-cut-asset-kind-filter="video"]').click()
  await page.locator('[data-cut-action="bin-save"]').click()
  await reply(await waitFor(saves, 1), { ok: true, result: { name: 'saved-B' } })
  await (await waitFor(reads, 4)).abort('failed')
  await page.locator('[data-cut-bin-unavailable]').waitFor()
  assert.equal(await page.locator('[data-cut-bin="B-bin"]').count(), 1, 'same-project error retains confirmed bins')
  await page.locator('[data-cut-action="bin-retry"]').click()
  await reply(await waitFor(reads, 5), { ok: false, error: { code: 'unavailable', message: 'temporary refusal' } })
  await page.locator('[data-cut-bin-unavailable]').waitFor()
  await page.locator('[data-cut-action="bin-retry"]').click()
  await reply(await waitFor(reads, 6), { ok: true, result: { bins: null } })
  await page.locator('[data-cut-bin-unavailable]').waitFor()
  await page.locator('[data-cut-action="bin-retry"]').click()
  await reply(await waitFor(reads, 7), listed('B-bin', 'saved-B'))
  await page.locator('[data-cut-bin="saved-B"]').waitFor()
  assert.equal(await page.locator('[data-cut-bin-unavailable]').count(), 0)
  assert.equal(saves.length, 1, 'retry never repeats the save mutation')

  await page.locator('[data-test-a]').click()
  await reply(await waitFor(reads, 8), listed('A-only'))
  await page.locator('[data-cut-bin="A-only"]').waitFor()
  await page.locator('[data-cut-action="bin-save"]').click()
  const heldSave = await waitFor(saves, 2)
  await page.locator('[data-test-b]').click()
  await reply(await waitFor(reads, 9), listed('B-bin', 'saved-B'))
  await page.locator('[data-cut-bin="B-bin"]').waitFor()
  await reply(heldSave, { ok: true, result: { name: 'saved-B' } })
  await page.waitForTimeout(60)
  assert.equal(reads.length, 9, 'old-project Save acknowledgement cannot start a B list refresh')
  assert.equal(await page.locator('[data-cut-bin="A-only"]').count(), 0)
  assert.deepEqual(await page.evaluate(() => window.__unhandled), [])
  assert.deepEqual(errors, [])
})
