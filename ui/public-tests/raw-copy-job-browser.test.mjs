import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'
const PROJECT_A = `sha256:${'a'.repeat(64)}`
const PROJECT_B = `sha256:${'b'.repeat(64)}`

test('raw copy retains exact project job through status and cancel failures', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-raw-copy-job-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false, plugins: [react(), {
      name: 'raw-copy-dialog-stub', enforce: 'pre',
      resolveId(source) { if (source === '@tauri-apps/plugin-dialog') return '\0raw-copy-dialog-stub' },
      load(id) { if (id === '\0raw-copy-dialog-stub') return 'export async function save() { if (window.__holdCopyPicker) { window.__holdCopyPicker = false; return new Promise(resolve => { window.__releaseCopyPicker = resolve }) } return "/fixture/output/copy.mp4" }' },
    }, {
      name: 'raw-copy-job-fixture',
      configureServer(vite) {
        vite.middlewares.use('/__raw_copy_job__', (_request, response) => {
          response.setHeader('content-type', 'text/html')
          response.end(`<div id="root"></div><script type="module">
            import RefreshRuntime from '/@react-refresh'
            RefreshRuntime.injectIntoGlobalHook(window)
            window.$RefreshReg$ = () => {}
            window.$RefreshSig$ = () => type => type
            window.__vite_plugin_react_preamble_installed__ = true
          </script><script type="module" src="/public-tests/fixtures/raw-copy-job-browser.tsx"></script>`)
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
  const authorized = []
  let holdAuthorization = false
  await page.route('**/api/verb/**', route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'screen_record.copy_raw') { submitted.push(route); return }
    if (name === 'jobs.status') { status.push(route); return }
    if (name === 'jobs.cancel') { cancelled.push(route); return }
    if (name === 'project.set_output_dir' && holdAuthorization) { authorized.push(route); return }
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: {} }) })
  })
  const waitFor = async (items, count) => {
    for (let i = 0; i < 120; i += 1) {
      if (items.length >= count) return items[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected request ${count}, saw ${items.length}`)
  }
  const reply = (route, result) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(result) })
  const record = (id, state, extra = {}) => ({ job_id: id, kind: 'screen_record_copy_raw', state, progress: state === 'done' ? 1 : 0, ...extra })

  await page.goto(new URL('/__raw_copy_job__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-test-copy]').waitFor()
  await page.evaluate(() => {
    const button = document.querySelector('[data-test-copy]')
    button.click()
    button.click()
  })
  await waitFor(submitted, 1)
  assert.equal(submitted[0].request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  assert.equal(submitted.length, 1, 'picker and submission share one synchronous pending guard')
  assert.equal(await page.locator('[data-test-copy]').isDisabled(), true)
  await reply(submitted[0], { ok: true, result: { job_id: 'copy-1' } })
  const first = await waitFor(status, 1)
  assert.equal(first.request().postDataJSON().job_id, 'copy-1')
  assert.equal(first.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await page.waitForTimeout(650)
  assert.equal(status.length, 1, 'slow status request cannot overlap the next poll')
  await first.abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('status unknown'))
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), 'copy-1')
  assert.equal(await page.locator('[data-test-copy]').isDisabled(), true)
  await reply(await waitFor(status, 2), { ok: false, error: { code: 'conflict', message: 'job project changed' } })
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), 'copy-1', 'backend B while React still A cannot complete A copy')
  await reply(await waitFor(status, 3), { ok: true, result: record('other', 'done') })
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), 'copy-1')
  await page.locator('[data-test-cancel]').click()
  const cancel = await waitFor(cancelled, 1)
  assert.equal(cancel.request().postDataJSON().job_id, 'copy-1')
  assert.equal(cancel.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await cancel.abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('cancellation not confirmed'))
  await reply(await waitFor(status, 4), { ok: true, result: record('copy-1', 'done') })
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('Copy saved'))

  await page.locator('[data-test-copy]').click()
  await reply(await waitFor(submitted, 2), { ok: true, result: { job_id: 'copy-2' } })
  await reply(await waitFor(status, 5), { ok: true, result: record('copy-2', 'failed', { outcome: 'cancelled' }) })
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('Copy cancelled'))

  await page.locator('[data-test-copy]').click()
  await reply(await waitFor(submitted, 3), { ok: true, result: { job_id: 'copy-3' } })
  const oldProjectStatus = await waitFor(status, 6)
  assert.equal(oldProjectStatus.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await page.evaluate(() => window.changeCopyProject())
  await page.waitForFunction(project => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === `${project}:/fixture/raw-b.mp4`, PROJECT_B)
  assert.equal(await page.locator('[data-test-cancel]').count(), 0, 'old-project copy cannot be cancelled in B')
  await reply(oldProjectStatus, { ok: true, result: record('copy-3', 'done') })
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), 'copy-3', 'matching historical ID and kind in B cannot complete A copy')
  await page.waitForTimeout(650)
  assert.equal(status.length, 6, 'B never queries the A copy')
  await page.evaluate(() => window.returnCopyProject())
  const heldReconcile = await waitFor(status, 7)
  await page.evaluate(() => window.changeCopyProject())
  await page.waitForFunction(project => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === `${project}:/fixture/raw-b.mp4`, PROJECT_B)
  await page.evaluate(() => window.returnCopyProject())
  await page.waitForFunction(project => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === `${project}:/fixture/raw-a.mp4`, PROJECT_A)
  await page.waitForTimeout(650)
  assert.equal(status.length, 7, 'A to B to A does not overlap a held A status request')
  await reply(heldReconcile, { ok: true, result: record('copy-3', 'done') })
  const finalReconcile = await waitFor(status, 8)
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), 'copy-3')
  await reply(finalReconcile, { ok: true, result: record('copy-3', 'done') })
  await page.waitForFunction(() => document.querySelector('[data-test-job]')?.getAttribute('data-test-job') === '')

  await page.locator('[data-test-copy]').click()
  const lost = await waitFor(submitted, 4)
  assert.equal(lost.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await page.evaluate(() => window.changeCopyCapture())
  await lost.abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('admission could not be confirmed'))
  assert.match(await page.locator('[data-test-note]').textContent(), /another attempt may create a duplicate/i)
  await page.waitForTimeout(100)
  assert.equal(submitted.length, 4, 'lost pre-ID response is never resubmitted automatically')
  assert.equal(await page.locator('[data-test-copy]').isDisabled(), false, 'manual copy stays available after pre-ID loss')
  await page.locator('[data-test-copy]').click()
  await reply(await waitFor(submitted, 5), { ok: true, result: { job_id: 17 } })
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('admission could not be confirmed'))
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), '', 'numeric job ID is not admitted')
  await page.locator('[data-test-copy]').click()
  await reply(await waitFor(submitted, 6), { ok: true, result: { job_id: ' copy-6 ' } })
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('admission could not be confirmed'))
  await page.waitForFunction(() => !document.querySelector('[data-test-copy]')?.disabled)
  assert.equal(await page.locator('[data-test-job]').getAttribute('data-test-job'), '', 'padded job ID is not normalized or admitted')
  await page.evaluate(() => { window.__holdCopyPicker = true })
  await page.locator('[data-test-copy]').click()
  await page.waitForFunction(() => typeof window.__releaseCopyPicker === 'function')
  await page.evaluate(() => window.changeCopyProject())
  await page.waitForFunction(project => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === `${project}:/fixture/raw-b.mp4`, PROJECT_B)
  await page.evaluate(() => window.__releaseCopyPicker('/fixture/output/copy.mp4'))
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('Recording changed before copy started'))
  assert.equal(submitted.length, 6, 'stale picker result cannot submit old source to a new project')
  assert.equal(await page.locator('[data-test-copy]').isDisabled(), false)
  holdAuthorization = true
  await page.locator('[data-test-copy]').click()
  const heldAuthorization = await waitFor(authorized, 1)
  await page.evaluate(() => window.returnCopyProject())
  await page.waitForFunction(project => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === `${project}:/fixture/raw-a.mp4`, PROJECT_A)
  await page.evaluate(() => window.changeCopyProject())
  await page.waitForFunction(project => document.querySelector('[data-test-owner]')?.getAttribute('data-test-owner') === `${project}:/fixture/raw-b.mp4`, PROJECT_B)
  holdAuthorization = false
  await reply(heldAuthorization, { ok: true, result: {} })
  await page.waitForFunction(() => document.querySelector('[data-test-note]')?.textContent?.includes('Recording changed before copy started'))
  await page.waitForFunction(() => !document.querySelector('[data-test-copy]')?.disabled)
  assert.equal(submitted.length, 6, 'A to B to A owner generation change invalidates held authorization even when keys match again')
  holdAuthorization = true
  await page.locator('[data-test-copy]').click()
  const unmountedAuthorization = await waitFor(authorized, 2)
  await page.evaluate(() => window.unmountCopyHook())
  await page.locator('[data-test-unmounted]').waitFor({ state: 'attached' })
  holdAuthorization = false
  await reply(unmountedAuthorization, { ok: true, result: {} })
  await page.waitForTimeout(100)
  assert.equal(submitted.length, 6, 'authorization resolved after unmount cannot send a copy request')
  assert.deepEqual(errors, [], 'no unhandled status, cancel, or submission rejection')
})
