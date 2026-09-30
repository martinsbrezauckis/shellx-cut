import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('production AssetWords stays responsive to imported ranges, new words and restore', { timeout: 30_000 }, async t => {
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), configFile: false,
    plugins: [react(), {
      name: 'transcript-word-range-fixture',
      configureServer(vite) {
        vite.middlewares.use('/__transcript_word_range_fixture__', (_request, response) => {
          response.setHeader('content-type', 'text/html')
          response.end(`<div id="root"></div><script type="module">
            import RefreshRuntime from '/@react-refresh'
            RefreshRuntime.injectIntoGlobalHook(window)
            window.$RefreshReg$ = () => {}
            window.$RefreshSig$ = () => type => type
            window.__vite_plugin_react_preamble_installed__ = true
          </script><script type="module" src="/public-tests/fixtures/transcript-word-range-browser.tsx"></script>`)
        })
      },
    }], server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  page.setDefaultTimeout(5_000)
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  await page.route('**/api/verb/**', route => route.fulfill({
    status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: { cards: [], entries: [] } }),
  }))
  await page.goto(new URL('/__transcript_word_range_fixture__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-word="fixture:2"]').waitFor()
  const heartbeat = async () => {
    const before = Number(await page.locator('[data-heartbeat]').textContent())
    await page.waitForFunction(value => Number(document.querySelector('[data-heartbeat]')?.textContent) > value, before)
  }
  assert.equal(await page.locator('[data-cut-removed]').count(), 0, 'malformed optional metadata mounts without annotations')
  await heartbeat()
  await page.locator('[data-fixture-cuts]').click()
  assert.equal(await page.locator('[data-cut-removed="first"] [data-cut-word]').count(), 2)
  assert.equal(await page.locator('[data-cut-removed="huge"] [data-word-idx]').getAttribute('data-word-idx'), String(Number.MAX_SAFE_INTEGER))
  assert.equal(await page.locator('[data-cut-removed]').count(), 2, 'overlap preserves first-cut grouping and sparse high index')
  await heartbeat()
  await page.locator('[data-fixture-words]').click()
  assert.equal(await page.locator('[data-cut-word="fixture:2"]').count(), 0)
  assert.equal(await page.locator('[data-cut-removed="first"] [data-word-idx]').getAttribute('data-word-idx'), '3')
  assert.equal(await page.locator('[data-cut-removed="huge"] [data-cut-word]').count(), 2, 'new words rerender against the same cuts identity')
  await heartbeat()
  await page.locator('[data-cut-action="restore"][data-cut-op="first"]').click()
  assert.equal(await page.locator('[data-restored]').textContent(), 'first')
  assert.equal(await page.locator('[data-cut-removed="first"]').count(), 0)
  assert.equal(await page.locator('[data-cut-removed="huge"] [data-cut-word]').count(), 3, 'restore reveals the overlapping cut')
  await page.locator('[data-cut-action="restore"][data-cut-op="huge"]').click()
  assert.equal(await page.locator('[data-restored]').textContent(), 'huge')
  assert.equal(await page.locator('[data-cut-removed]').count(), 0)
  assert.equal(await page.locator('[data-cut-action="word"][role="button"]').count(), 3)
  await heartbeat()
  assert.deepEqual(errors, [])
  const panel = page.locator('[data-panel-fixture]')
  await panel.locator('[data-cut-action="view-source"]').click()
  assert.equal(await panel.locator('[data-cut-transcript="fixture"]').count(), 1,
    'production Transcript mounts with prototype-key metadata lacking matching project assets/transcripts')
  assert.equal(await panel.locator('[data-cut-removed="ordinary"] [data-word-idx]').getAttribute('data-word-idx'), '2')
  await panel.locator('[data-cut-action="restore"][data-cut-op="ordinary"]').click()
  assert.equal(await panel.locator('[data-cut-removed]').count(), 0)
  assert.equal(await panel.locator('[data-cut-action="word"][role="button"]').count(), 2)
  await heartbeat()
  assert.deepEqual(errors, [])
})
