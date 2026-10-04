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

test('render queue keeps exact admission and status across modal, mode, and project changes', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-render-queue-owner-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), {
      name: 'queue-owner-dialog-stub', enforce: 'pre',
      resolveId(source) { if (source === '@tauri-apps/plugin-dialog') return '\0queue-owner-dialog-stub' },
      load(id) { if (id === '\0queue-owner-dialog-stub') return 'export async function save() { if (window.__rejectQueuePicker) { window.__rejectQueuePicker = false; throw Error("fixture picker refusal") } return new Promise(resolve => { window.__releaseQueuePicker = resolve }) }' },
    }, { name: 'queue-owner-fixture', configureServer(vite) {
      vite.middlewares.use('/__queue_owner__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/render-queue-owner-browser.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  const submitted = []
  const statuses = []
  const statusTimes = []
  const authorizations = []
  let preflights = 0
  let holdAuthorization = false
  await page.route('**/api/verb/**', route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'render.queue') { submitted.push(route); return }
    if (name === 'jobs.status') { statuses.push(route); statusTimes.push(Date.now()); return }
    if (name === 'project.set_output_dir' && holdAuthorization) { authorizations.push(route); return }
    if (name === 'verify.pregate') preflights += 1
    const result = name === 'verify.pregate' ? { pass: true, risks: [] }
      : name === 'jobs.list' ? { jobs: [] } : {}
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })
  })
  const waitFor = async (items, count) => {
    for (let i = 0; i < 180; i += 1) {
      if (items.length >= count) return items[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected request ${count}, saw ${items.length}; page errors: ${errors.join('; ')}`)
  }
  const reply = (route, result) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(result) })
  const record = (id, state, extra = {}) => ({ job_id: id, kind: 'render_queue', state, progress: 0.4, ...extra })
  const openQueue = async () => {
    await page.locator('[data-cut-export-btn]').click()
    await page.locator('[data-cut-render-queue-open]').click()
    await page.locator('[data-cut-render-queue]').waitFor()
  }
  const start = async () => { await page.locator('[data-cut-render-queue-start]').click() }

  await page.goto(new URL('/__queue_owner__', server.resolvedUrls.local[0]).href)
  await openQueue()
  await page.evaluate(() => {
    const button = document.querySelector('[data-cut-render-queue-start]')
    button.click(); button.click()
  })
  const queued = await waitFor(submitted, 1)
  assert.equal(queued.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  assert.equal(preflights, 1, 'same-turn double click starts one preflight and one submission')
  await page.locator('[data-cut-render-queue-close]').click()
  await page.locator('[data-test-record]').click()
  await reply(queued, { ok: true, result: { queue_id: 'q1', count: 2, jobs: [] } })
  await page.waitForFunction(() => document.querySelector('[data-test-qid]')?.textContent === 'q1')
  const first = await waitFor(statuses, 1)
  assert.equal(first.request().postDataJSON().job_id, 'q1')
  assert.equal(first.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await page.waitForTimeout(750)
  assert.equal(statuses.length, 1, 'slow status read never overlaps another')
  await first.abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'status_unknown')
  await reply(await waitFor(statuses, 2), { ok: false, error: { code: 'conflict', message: 'job project changed' } })
  assert.equal(await page.locator('[data-test-qid]').textContent(), 'q1', 'backend B while React still A cannot complete A queue')
  await reply(await waitFor(statuses, 3), { ok: true, result: record('other', 'done', { progress: 1 }) })
  assert.equal(await page.locator('[data-test-qid]').textContent(), 'q1', 'wrong ID cannot complete the queue')
  const running = await waitFor(statuses, 4)
  await reply(running, { ok: true, result: record('q1', 'running', { result: { count: 2, jobs: [{ idx: 0, output: '/fixture/one.mp4', ok: true }] } }) })
  await page.waitForFunction(() => document.querySelector('[data-test-progress]')?.textContent === '0.4')
  assert.equal(await page.locator('[data-test-result-rows]').textContent(), '1')
  const nextPaced = await waitFor(statuses, 5)
  assert.ok(statusTimes[4] - statusTimes[3] >= 600, 'running reply retains the 700ms poll pace')
  await reply(nextPaced, { ok: false, error: { code: 'temporary', message: 'status unavailable' } })
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'status_unknown')
  await reply(await waitFor(statuses, 6), { ok: true, result: record('q1', 'running') })
  assert.ok(statusTimes[5] - statusTimes[4] >= 850, 'failure backoff survives running-to-unknown transition')
  await page.locator('[data-test-edit]').click()
  await openQueue()
  assert.equal(await page.locator('[data-cut-render-queue-form]').count(), 0, 'reopening keeps admitted queue view')
  assert.equal(await page.locator('[data-cut-render-queue-error-back]').count(), 0, 'status unknown never exposes Back to resubmit')
  await page.locator('[data-cut-render-queue-close]').click()
  await page.locator('[data-test-a-rename]').click()
  assert.equal(await page.locator('[data-test-qid]').textContent(), 'q1', 'same-origin rename retains queue')
  const held = await waitFor(statuses, 7)
  assert.equal(held.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await page.locator('[data-test-b]').click()
  await openQueue()
  await page.locator('[data-cut-render-queue-owner-other]').waitFor()
  assert.equal(await page.locator('[data-cut-render-queue-start]').count(), 0)
  await reply(held, { ok: true, result: record('q1', 'done', { progress: 1, result: { count: 2, succeeded: 2, failed: 0, jobs: [{ ok: true }, { ok: true }] } }) })
  assert.equal(await page.locator('[data-test-qid]').textContent(), 'q1', 'B historical matching ID cannot complete A queue')
  await page.waitForTimeout(750)
  assert.equal(statuses.length, 7, 'B never queries A queue')
  await page.evaluate(() => window.queueFixtureProject('a'))
  const reconciled = await waitFor(statuses, 8)
  assert.equal(reconciled.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await reply(reconciled, { ok: true, result: record('q1', 'done', { progress: 1, result: { count: 2, succeeded: 1, failed: 1, jobs: [{ ok: true }, { ok: false }] } }) })
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'error')
  assert.match(await page.locator('[data-test-error]').textContent(), /1 failed delivery/)
  await openQueue()
  await page.locator('[data-cut-render-queue-error-back]').click()
  await page.locator('[data-cut-render-queue-form]').waitFor()

  await start()
  await reply(await waitFor(submitted, 2), { ok: false, error: { code: 'invalid_args', message: 'Definite engine refusal' } })
  await page.waitForFunction(() => document.querySelector('[data-cut-render-queue-form-error]')?.textContent?.includes('Definite engine refusal'))
  assert.equal(submitted.length, 2)
  await start()
  await (await waitFor(submitted, 3)).abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'submit_unknown')
  assert.match(await page.locator('[data-test-error]').textContent(), /another attempt may duplicate deliveries/i)
  await page.waitForTimeout(100)
  assert.equal(submitted.length, 3, 'lost acknowledgement never automatically resubmits')
  await page.locator('[data-cut-render-queue-error-back]').click()
  await start()
  await reply(await waitFor(submitted, 4), { ok: true, result: { queue_id: 42 } })
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'submit_unknown')
  assert.equal(await page.locator('[data-test-qid]').textContent(), '', 'numeric queue ID is not admitted')
  await page.locator('[data-cut-render-queue-error-back]').click()
  await start()
  await reply(await waitFor(submitted, 5), { ok: true, result: { queue_id: 'q5' } })
  await reply(await waitFor(statuses, 9), { ok: true, result: record('q5', 'done', { progress: 1, result: { count: 2, succeeded: 2, failed: 0, jobs: [{ ok: true }, { ok: true }] } }) })
  await page.locator('[data-cut-render-queue-done-close]').click()

  await openQueue()
  await start()
  const oldProjectRefusal = await waitFor(submitted, 6)
  assert.equal(oldProjectRefusal.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await page.evaluate(() => window.queueFixtureProject('b'))
  await reply(oldProjectRefusal, { ok: false, error: { code: 'conflict', message: 'job project changed' } })
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'form')
  await openQueue()
  await page.locator('[data-cut-render-queue-form]').waitFor()
  await start()
  await (await waitFor(submitted, 7)).abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'submit_unknown')
  assert.match(await page.locator('[data-test-error]').textContent(), /another attempt may duplicate deliveries/i)
  assert.equal(submitted.length, 7, 'B has no automatic retry after a lost submit response')
  await page.locator('[data-cut-render-queue-error-back]').click()
  await page.locator('[data-cut-render-queue-form]').waitFor()
  await start()
  await reply(await waitFor(submitted, 8), { ok: true, result: { queue_id: 'q8' } })
  await reply(await waitFor(statuses, 10), { ok: true, result: record('q8', 'failed', { error: { code: 'interrupted', message: 'Queue interrupted' } }) })
  assert.equal(statuses[9].request().postDataJSON().expected_origin_path_sha256, PROJECT_B)
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'error')
  assert.match(await page.locator('[data-test-error]').textContent(), /Queue interrupted/)
  await page.locator('[data-cut-render-queue-error-back]').click()
  await page.locator('[data-cut-render-queue-close]').click()
  await page.locator('[data-test-a]').click()

  await page.evaluate(() => localStorage.setItem('cut.outputDir', '/fixture/output'))
  await openQueue()
  holdAuthorization = true
  await start()
  const heldAuthorization = await waitFor(authorizations, 1)
  await page.evaluate(() => window.queueFixtureProject('b'))
  await page.evaluate(() => window.queueFixtureProject('a'))
  holdAuthorization = false
  await reply(heldAuthorization, { ok: true, result: {} })
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'form')
  assert.equal(submitted.length, 8, 'A to B to A during output authorization cannot submit stale queue')

  await page.evaluate(() => { window.__TAURI__ = { core: { invoke: async () => ({}) }, event: { listen: async () => () => {} } } })
  await openQueue()
  await page.locator('[data-cut-render-queue-output-pick="0"]').click()
  await page.waitForFunction(() => typeof window.__releaseQueuePicker === 'function')
  await page.locator('[data-cut-render-queue-close]').click()
  await page.evaluate(() => window.__releaseQueuePicker('/fixture/stale-picker.mp4'))
  await page.waitForTimeout(50)
  assert.equal(await page.locator('[data-test-row-output]').textContent(), '', 'closed picker cannot mutate app-owned rows')
  await openQueue()
  await page.evaluate(() => { window.__rejectQueuePicker = true })
  await page.locator('[data-cut-render-queue-output-pick="0"]').click()
  await page.waitForTimeout(50)
  assert.equal(await page.locator('[data-test-row-output]').textContent(), '', 'rejected native picker leaves output unchanged')
  assert.deepEqual(errors, [])
})
