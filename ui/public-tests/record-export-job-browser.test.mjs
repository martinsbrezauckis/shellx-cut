import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('Record export keeps one admitted job through status and cancel transport failures', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-record-export-job-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false, plugins: [react(), {
      name: 'record-export-job-fixture',
      configureServer(vite) {
        vite.middlewares.use('/__record_export_job__', (_request, response) => {
          response.setHeader('content-type', 'text/html')
          response.end(`<div id="root"></div><script type="module">
            import RefreshRuntime from '/@react-refresh'
            RefreshRuntime.injectIntoGlobalHook(window)
            window.$RefreshReg$ = () => {}
            window.$RefreshSig$ = () => type => type
            window.__vite_plugin_react_preamble_installed__ = true
          </script><script type="module" src="/public-tests/fixtures/record-export-job-browser.tsx"></script>`)
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
  const submitted = []
  const status = []
  const cancelled = []
  await page.route('**/api/verb/**', route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'screen_record.export') { submitted.push(route); return }
    if (name === 'jobs.status') { status.push(route); return }
    if (name === 'jobs.cancel') { cancelled.push(route); return }
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: {} }) })
  })
  const waitFor = async (items, count) => {
    for (let i = 0; i < 100; i += 1) {
      if (items.length >= count) return items[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected request ${count}, saw ${items.length}`)
  }
  const reply = (route, result) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(result) })
  const record = (id, state, extra = {}) => ({ job_id: id, kind: 'screen_record_export', state, progress: state === 'done' ? 1 : 0, ...extra })

  await page.goto(new URL('/__record_export_job__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-test-export]').waitFor()
  await page.evaluate(() => {
    const button = document.querySelector('[data-test-export]')
    button.click()
    button.click()
  })
  await waitFor(submitted, 1)
  assert.equal(submitted.length, 1, 'pending ref blocks a second submission before React commits')
  await reply(submitted[0], { ok: true, result: { job_id: 'e1', status: 'queued' } })
  const first = await waitFor(status, 1)
  assert.equal(first.request().postDataJSON().job_id, 'e1')
  await page.waitForTimeout(650)
  assert.equal(status.length, 1, 'awaited scheduler never overlaps a slow status request')
  await first.abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('status unknown'))
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), 'e1')
  assert.equal(await page.locator('[data-test-export]').isDisabled(), true)
  const second = await waitFor(status, 2)
  await reply(second, { ok: false, error: { code: 'unavailable', message: 'status temporarily unavailable' } })
  const third = await waitFor(status, 3)
  await reply(third, { ok: true, result: record('other', 'done') })
  await page.waitForTimeout(100)
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), 'e1', 'mismatched terminal ID is ignored')
  await page.locator('[data-test-cancel]').click()
  const cancel = await waitFor(cancelled, 1)
  assert.equal(cancel.request().postDataJSON().job_id, 'e1')
  await cancel.abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('cancellation not confirmed'))
  const fourth = await waitFor(status, 4)
  await reply(fourth, { ok: true, result: record('e1', 'done', { result: { elapsed_ms: 1200 } }) })
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('Saved MP4'))
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), '')

  await page.locator('[data-test-export]').click()
  await reply(await waitFor(submitted, 2), { ok: true, result: { job_id: 'e2', status: 'queued' } })
  await reply(await waitFor(status, 5), { ok: true, result: record('e2', 'failed', { error: { code: 'render_failed', message: 'encoder failed' } }) })
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('encoder failed'))

  await page.locator('[data-test-export]').click()
  await reply(await waitFor(submitted, 3), { ok: true, result: { job_id: 'e3', status: 'queued' } })
  await reply(await waitFor(status, 6), { ok: true, result: record('e3', 'failed', { outcome: 'cancelled' }) })
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('Export cancelled'))

  await page.locator('[data-test-export]').click()
  const staleSubmission = await waitFor(submitted, 4)
  await page.evaluate(() => window.changeExportOwner())
  await page.waitForFunction(() => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === 'project-b/capture-2')
  await reply(staleSubmission, { ok: true, result: { job_id: 'old-owner', status: 'queued' } })
  await page.waitForFunction(() => document.querySelector('[data-test-job]')?.getAttribute('data-test-job') === 'old-owner')
  assert.equal(await page.locator('[data-test-export]').isDisabled(), true, 'late old-owner admission retains exact ID and blocks another export')
  assert.match(await page.locator('[data-test-note]').textContent(), /previous project export/i)
  assert.equal(await page.locator('[data-test-cancel]').count(), 0, 'old-project job cannot be cancelled in another project')
  await page.waitForTimeout(650)
  assert.equal(status.length, 6, 'old-project job is not queried against the new project job table')
  await page.evaluate(() => window.returnExportOwner())
  const ownerStatus = await waitFor(status, 7)
  assert.equal(ownerStatus.request().postDataJSON().job_id, 'old-owner')
  await page.evaluate(() => window.changeExportOwner())
  await page.waitForFunction(() => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === 'project-b/capture-2')
  await reply(ownerStatus, { ok: true, result: record('old-owner', 'done') })
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), 'old-owner', 'an in-flight matching ID and kind from another active project cannot complete the old export')
  await page.evaluate(() => window.returnExportOwner())
  await page.waitForFunction(() => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === 'project-a/capture-1')
  const pendingReconcile = await waitFor(status, 8)
  assert.equal(pendingReconcile.request().postDataJSON().job_id, 'old-owner')
  await page.evaluate(() => window.changeExportOwner())
  await page.waitForFunction(() => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === 'project-b/capture-2')
  await page.evaluate(() => window.returnExportOwner())
  await page.waitForFunction(() => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === 'project-a/capture-1')
  await page.waitForTimeout(650)
  assert.equal(status.length, 8, 'A to B to A never overlaps an unresolved status request')
  await reply(pendingReconcile, { ok: true, result: record('old-owner', 'done') })
  const reconciledStatus = await waitFor(status, 9)
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), 'old-owner', 'matching historical ID while another project was active cannot complete the old export')
  assert.equal(reconciledStatus.request().postDataJSON().job_id, 'old-owner')
  await reply(reconciledStatus, { ok: true, result: record('old-owner', 'done') })
  await page.waitForFunction(() => document.querySelector('[data-test-job]')?.getAttribute('data-test-job') === '')
  assert.match(await page.locator('[data-test-note]').textContent(), /Saved MP4/, 'the original project accepts its own reconciled terminal status')

  await page.locator('[data-test-export]').click()
  const lostAdmission = await waitFor(submitted, 5)
  await page.evaluate(() => window.changeExportOwner())
  await page.waitForFunction(() => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === 'project-b/capture-2')
  await lostAdmission.abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('previous project export admission could not be confirmed'))
  assert.match(await page.locator('[data-test-note]').textContent(), /another attempt may create a duplicate/i)
  await page.waitForTimeout(100)
  assert.equal(submitted.length, 5, 'pre-ID transport loss never automatically resubmits')
  assert.equal(await page.locator('[data-test-export]').isDisabled(), false, 'pre-ID transport loss leaves the deliberate Export action available')
  await page.locator('[data-test-export]').click()
  await reply(await waitFor(submitted, 6), { ok: false, error: { code: 'refused', message: 'Export refused' } })
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('Export refused'))
  await page.evaluate(() => window.unmountExportHook())
  await page.locator('[data-test-unmounted]').waitFor({ state: 'attached' })
  assert.deepEqual(errors, [], 'no unhandled status/cancel/submission rejection')
})
