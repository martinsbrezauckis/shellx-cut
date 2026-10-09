import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('mounted queue holds the exact two edited profiles through one deferred preflight', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-queue-preflight-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false, plugins: [react(), {
      name: 'queue-preflight-fixture',
      configureServer(vite) {
        vite.middlewares.use('/__queue_pregate__', (_request, response) => {
          response.setHeader('content-type', 'text/html')
          response.end(`<div id="root"></div><script type="module">
            import RefreshRuntime from '/@react-refresh'
            RefreshRuntime.injectIntoGlobalHook(window)
            window.$RefreshReg$ = () => {}
            window.$RefreshSig$ = () => type => type
            window.__vite_plugin_react_preamble_installed__ = true
          </script><script type="module" src="/public-tests/fixtures/render-queue-preflight-browser.tsx"></script>`)
        })
      },
    }], server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  const pregates = []
  const warning = { pass: false, summary: 'pregate FAIL — fix before spending the render', risks: [{ kind: 'black_or_frozen', severity: 'high', detail: '13066ms frozen of source', range_ms: [0, 29167] }, { kind: 'slideshow_risk', severity: 'med' }, { kind: 'silent_output', severity: 'med' }] }
  const enqueued = []
  await page.route('**/api/verb/**', async route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'verify.pregate') { pregates.push(route); return }
    if (name === 'render.queue') enqueued.push(route.request().postDataJSON())
    const result = name === 'jobs.list' ? { jobs: [] }
      : name === 'render.queue' ? { queue_id: 'q1', jobs: [] }
      : name === 'jobs.status' ? { job_id: 'q1', kind: 'render_queue', state: 'done', progress: 1, result: { count: 2, succeeded: 2, failed: 0, jobs: [{ ok: true }, { ok: true }] } }
      : {}
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })
  })
  await page.goto(new URL('/__queue_pregate__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-export-btn]').click()
  await page.locator('[data-cut-render-queue-open]').click()
  await page.locator('[data-cut-render-queue-row]').first().waitFor()
  const start = page.locator('[data-cut-render-queue-start]')
  await start.click()
  await page.waitForFunction(() => document.querySelector('[data-cut-render-queue-preflight-status]'))
  assert.equal(pregates.length, 1)
  await page.locator('[data-cut-render-queue-close]').click()
  await pregates[0].fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: warning }) })
  await page.waitForTimeout(100)
  assert.equal(await page.locator('[data-cut-pregate-warning]').count(), 0, 'late high-risk result cannot revive a closed queue')
  assert.equal(enqueued.length, 0, 'closed pending preflight submits no jobs')
  await page.locator('[data-cut-export-btn]').click()
  await page.locator('[data-cut-render-queue-open]').click()
  await page.locator('[data-cut-render-queue-row]').first().waitFor()
  await page.evaluate(() => {
    const button = document.querySelector('[data-cut-render-queue-start]')
    button.click()
    button.click() // same event turn, before React can commit disabled state
  })
  await page.locator('[data-cut-render-queue-preflight-status]').waitFor()
  await page.waitForFunction(() => document.querySelector('[data-cut-render-queue-fields]')?.matches(':disabled'))
  assert.equal(pregates.length, 2, 'rapid double click starts one preflight')
  assert.equal(await start.isDisabled(), true)
  assert.equal(await page.locator('[data-cut-render-queue-close]').isEnabled(), true, 'Close stays available while checking')
  const preset0 = page.locator('[data-cut-render-queue-preset="0"]')
  await assert.rejects(preset0.selectOption('high', { timeout: 250 }), /disabled|Timeout/i)
  assert.equal(await preset0.inputValue(), 'standard', 'rows cannot change while check is pending')

  await pregates[1].fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: warning }) })
  await page.locator('[data-cut-pregate-warning]').waitFor()
  const layers = await page.evaluate(() => ({
    warning: Number(getComputedStyle(document.querySelector('[data-cut-pregate-warning]')).zIndex),
    queue: Number(getComputedStyle(document.querySelector('[data-cut-render-queue]')).zIndex),
  }))
  assert.ok(layers.warning > layers.queue, 'warning remains visible above queue modal')
  assert.equal(await preset0.isDisabled(), true, 'warning keeps rows locked')
  assert.equal(enqueued.length, 0)
  assert.equal(await page.locator('[data-cut-pregate-continue]').isEnabled(), true, 'high-risk quality prediction allows human override')
  assert.equal(await page.locator('[data-cut-pregate-continue]').textContent(), 'Queue anyway')
  assert.match(await page.locator('[data-cut-pregate-warning]').textContent(), /screen recording/)
  await page.locator('[data-cut-pregate-cancel]').click()
  await page.locator('[data-cut-pregate-warning]').waitFor({ state: 'detached' })
  assert.equal(await preset0.isEnabled(), true, 'Cancel unlocks queue edits')
  assert.equal(enqueued.length, 0, 'Cancel submits no jobs')
  await preset0.selectOption('high')
  await page.locator('[data-cut-render-queue-preset="1"]').selectOption('draft')
  await start.click()
  await page.waitForFunction(() => document.querySelector('[data-cut-render-queue-preflight-status]'))
  assert.equal(pregates.length, 3, 'resubmission starts one new preflight')
  await pregates[2].fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: warning }) })
  await page.locator('[data-cut-pregate-warning]').waitFor()
  assert.equal(enqueued.length, 0, 'no batch is submitted until Continue')
  await page.evaluate(() => {
    const button = document.querySelector('[data-cut-pregate-continue]')
    button.click()
    button.click() // owner admission guards still prevent a duplicate batch
  })
  await page.locator('[data-cut-render-queue-done]').waitFor()
  assert.equal(enqueued.length, 1, 'exactly one queue is submitted after acknowledgment')
  assert.deepEqual(enqueued[0].jobs, [
    { preset: 'high' },
    { preset: 'draft', aspect: '9:16' },
  ])
  assert.deepEqual(errors, [])
})
