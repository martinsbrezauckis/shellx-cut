import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

const oldScan = '2026-10-04T08:00:00Z'
const newScan = '2026-10-04T08:01:00Z'
const doctor = (matte, premium = 'missing', scanned_at = oldScan) => ({
  ok: true,
  result: { schema: 'shellx-cut/doctor/1', scanned_at, cards: [
    { id: 'matte', status: matte, hint: matte === 'missing' ? 'Install RVM' : null, details: {} },
    { id: 'matte_premium', status: premium, details: {} },
  ] },
})
const fulfill = (route, body) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) })

async function mounted(t, fixturePath = '/__matte_refresh__') {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-matte-refresh-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'matte-refresh-fixture', configureServer(vite) {
      vite.middlewares.use('/__matte_owner__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/matte-doctor-owner-browser.tsx"></script>`)
      })
      vite.middlewares.use('/__matte_refresh__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/matte-refresh-browser.tsx"></script>`)
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
  const doctors = []
  const setups = []
  const jobs = []
  await page.route('**/api/verb/**', route => {
    const verb = new URL(route.request().url()).pathname.split('/').at(-1)
    if (verb === 'system.doctor') { doctors.push(route); return }
    if (verb === 'system.setup_matte') { setups.push(route); return }
    if (verb === 'jobs.status') { jobs.push(route); return }
    return fulfill(route, { ok: true, result: {} })
  })
  await page.goto(new URL(fixturePath, server.resolvedUrls.local[0]).href)
  const waitFor = async (items, count) => {
    for (let i = 0; i < 100 && items.length < count; i++) await page.waitForTimeout(25)
    assert.ok(items.length >= count, `expected request ${count}, saw ${items.length}`)
    return items[count - 1]
  }
  return { page, errors, doctors, setups, jobs, waitFor }
}

test('Matte cached open and forced Re-check honor latest success and failure', async t => {
  const { page, errors, doctors, waitFor } = await mounted(t)
  const initial = await waitFor(doctors, 1)
  assert.deepEqual(initial.request().postDataJSON(), {})
  await fulfill(initial, doctor('missing'))
  await page.locator('[data-cut-matte-requirements]').waitFor()
  await page.locator('[data-cut-matte-recheck]').click()
  const fresh = await waitFor(doctors, 2)
  assert.deepEqual(fresh.request().postDataJSON(), { refresh: true })
  assert.ok(Date.parse(newScan) > Date.parse(oldScan))
  await fulfill(fresh, doctor('ok', 'ok', newScan))
  await page.locator('[data-cut-matte-ready="true"]').waitFor()
  await page.locator('[data-test-project-b]').click()
  const bCached = await waitFor(doctors, 3)
  assert.deepEqual(bCached.request().postDataJSON(), {})
  await fulfill(bCached, doctor('missing'))
  await page.locator('[data-cut-matte-requirements]').waitFor()
  await page.locator('[data-test-project-a]').click()
  const aCached = await waitFor(doctors, 4)
  await fulfill(aCached, doctor('missing'))
  await page.locator('[data-cut-matte-requirements]').waitFor()
  await page.locator('[data-cut-matte-recheck]').click()
  const latestFailure = await waitFor(doctors, 5)
  await fulfill(latestFailure, { ok: false, error: { code: 'unavailable', message: 'temporarily unavailable' } })
  await page.locator('[data-cut-matte-probe-error]').waitFor()
  assert.equal(await page.locator('[data-cut-matte]').getAttribute('data-cut-matte-ready'), 'false')
  await page.locator('[data-cut-matte-recheck]').click()
  const retry = await waitFor(doctors, 6)
  assert.deepEqual(retry.request().postDataJSON(), { refresh: true })
  await fulfill(retry, doctor('ok', 'ok', newScan))
  await page.locator('[data-cut-matte-ready="true"]').waitFor()
  assert.deepEqual(errors, [])
})

test('Matte rejects old project replies and stale install completion without a Doctor refresh', async t => {
  const { page, errors, doctors, setups, jobs, waitFor } = await mounted(t)
  const oldA = await waitFor(doctors, 1)
  await page.locator('[data-test-project-b]').click()
  const currentB = await waitFor(doctors, 2)
  await fulfill(currentB, doctor('ok', 'ok', newScan))
  await page.locator('[data-cut-matte-ready="true"]').waitFor()
  await fulfill(oldA, doctor('missing'))
  assert.equal(await page.locator('[data-cut-matte]').getAttribute('data-cut-matte-ready'), 'true')
  await page.locator('[data-test-project-a]').click()
  const aCached = await waitFor(doctors, 3)
  await fulfill(aCached, doctor('missing'))
  await page.locator('[data-cut-matte-requirements]').waitFor()
  await page.locator('[data-cut-matte-install-rvm]').click()
  const setup = await waitFor(setups, 1)
  await fulfill(setup, { ok: true, result: { job_id: 'setup-a' } })
  const status = await waitFor(jobs, 1)
  await page.locator('[data-test-project-b]').click()
  const bAgain = await waitFor(doctors, 4)
  await fulfill(bAgain, doctor('ok', 'ok', newScan))
  await fulfill(status, { ok: true, result: { state: 'done', progress: 1 } })
  await page.waitForTimeout(150)
  assert.equal(doctors.length, 4, 'A install completion cannot start a B-scoped Doctor read')
  assert.equal(await page.locator('[data-cut-matte]').getAttribute('data-cut-matte-ready'), 'true')
  await page.locator('[data-test-unmount]').click()
  await page.locator('[data-cut-matte]').waitFor({ state: 'detached' })
  assert.deepEqual(errors, [])
})

test('Premium Re-check is fresh; latest failure revokes readiness and later retry restores it', async t => {
  const { page, errors, doctors, waitFor } = await mounted(t)
  const initial = await waitFor(doctors, 1)
  await fulfill(initial, doctor('ok', 'missing'))
  await page.locator('[data-cut-matte-model-premium]').click()
  await page.locator('[data-cut-matte-premium-consent]').waitFor()
  await page.locator('[data-cut-matte-premium-recheck]').click()
  const premiumRecheck = await waitFor(doctors, 2)
  assert.deepEqual(premiumRecheck.request().postDataJSON(), { refresh: true })
  await fulfill(premiumRecheck, doctor('ok', 'ok', newScan))
  await page.locator('[data-cut-matte-model-premium]').waitFor()
  await page.locator('[data-test-project-b]').click()
  const bCached = await waitFor(doctors, 3)
  await fulfill(bCached, doctor('ok', 'missing'))
  await page.locator('[data-cut-matte-model-premium]').click()
  await page.locator('[data-cut-matte-premium-consent]').waitFor()
  await page.locator('[data-test-rename]').click()
  assert.equal(doctors.length, 3, 'same-origin rename does not restart global Doctor')
  await page.locator('[data-test-project-a]').click()
  const aCached = await waitFor(doctors, 4)
  await fulfill(aCached, doctor('missing', 'ok'))
  await page.locator('[data-cut-matte-ready="true"]').waitFor()
  // A failed current read must not reuse the earlier Premium-ready card.
  await page.locator('[data-test-project-b]').click()
  const currentA = await waitFor(doctors, 5)
  await fulfill(currentA, { ok: false, error: { code: 'unavailable', message: 'Doctor failed' } })
  await page.locator('[data-cut-matte-probe-error]').waitFor()
  assert.equal(await page.locator('[data-cut-matte]').getAttribute('data-cut-matte-ready'), 'false')
  await page.locator('[data-cut-matte-recheck]').click()
  const recovery = await waitFor(doctors, 6)
  assert.deepEqual(recovery.request().postDataJSON(), { refresh: true })
  await fulfill(recovery, doctor('missing', 'ok', newScan))
  await page.locator('[data-cut-matte-ready="true"]').waitFor()
  assert.deepEqual(errors, [])
})

test('Current setup completion forces fresh Doctor, while unmounted replies are ignored', async t => {
  const { page, errors, doctors, setups, jobs, waitFor } = await mounted(t)
  await fulfill(await waitFor(doctors, 1), doctor('missing'))
  await page.locator('[data-cut-matte-install-rvm]').click()
  await fulfill(await waitFor(setups, 1), { ok: true, result: { job_id: 'setup-current' } })
  await fulfill(await waitFor(jobs, 1), { ok: true, result: { state: 'done', progress: 1 } })
  const completed = await waitFor(doctors, 2)
  assert.deepEqual(completed.request().postDataJSON(), { refresh: true })
  await fulfill(completed, doctor('ok', 'missing', newScan))
  await page.locator('[data-cut-matte-ready="true"]').waitFor()
  await page.locator('[data-test-project-b]').click()
  const pending = await waitFor(doctors, 3)
  await page.locator('[data-test-unmount]').click()
  await fulfill(pending, { ok: false, error: { code: 'unavailable', message: 'stale error' } })
  await page.locator('[data-test-remount]').click()
  const newMount = await waitFor(doctors, 4)
  assert.deepEqual(newMount.request().postDataJSON(), {})
  await fulfill(newMount, doctor('ok'))
  await page.locator('[data-cut-matte-ready="true"]').waitFor()
  assert.deepEqual(errors, [])
})

test('Doctor owner accepts only latest same-scope success or failure', async t => {
  const { page, errors, doctors, waitFor } = await mounted(t, '/__matte_owner__')
  await fulfill(await waitFor(doctors, 1), doctor('missing', 'ok'))
  await page.locator('[data-test-owner-ready="true"]').waitFor()
  await page.locator('[data-test-refresh]').click()
  const older = await waitFor(doctors, 2)
  assert.deepEqual(older.request().postDataJSON(), { refresh: true })
  await page.locator('[data-test-owner-ready="false"]').waitFor()
  await page.locator('[data-test-refresh]').click()
  const newer = await waitFor(doctors, 3)
  await fulfill(newer, doctor('missing', 'ok', newScan))
  await page.locator('[data-test-owner-ready="true"]').waitFor()
  await older.abort('failed')
  assert.equal(await page.locator('[data-test-owner-state]').getAttribute('data-test-owner-state'), 'absent')
  await page.locator('[data-test-refresh]').click()
  const latestError = await waitFor(doctors, 4)
  await fulfill(latestError, { ok: false, error: { code: 'unavailable', message: 'latest failed' } })
  await page.locator('[data-test-owner-state="error"]').waitFor()
  assert.equal(await page.locator('[data-test-owner-ready]').getAttribute('data-test-owner-ready'), 'false')
  await page.locator('[data-test-refresh]').click()
  await fulfill(await waitFor(doctors, 5), doctor('missing', 'ok', newScan))
  await page.locator('[data-test-owner-ready="true"]').waitFor()
  assert.deepEqual(errors, [])
})

test('Old installer status cannot clear newer install or refresh after A-B-A', async t => {
  const { page, errors, doctors, setups, jobs, waitFor } = await mounted(t)
  await fulfill(await waitFor(doctors, 1), doctor('missing'))
  await page.locator('[data-cut-matte-install-rvm]').click()
  await fulfill(await waitFor(setups, 1), { ok: true, result: { job_id: 'old-a' } })
  const oldStatus = await waitFor(jobs, 1)
  await page.locator('[data-test-project-b]').click()
  await fulfill(await waitFor(doctors, 2), doctor('missing'))
  await page.locator('[data-cut-matte-install-rvm]').click()
  await fulfill(await waitFor(setups, 2), { ok: true, result: { job_id: 'new-b' } })
  const newStatus = await waitFor(jobs, 2)
  await oldStatus.abort('failed')
  await page.waitForTimeout(75)
  assert.equal(await page.locator('[data-cut-matte-install-rvm]').isDisabled(), true, 'A rejection cannot clear B install')
  assert.equal(await page.locator('[data-cut-matte-error]').count(), 0, 'A rejection cannot paint B error')
  await fulfill(newStatus, { ok: true, result: { state: 'done', progress: 1 } })
  const bFresh = await waitFor(doctors, 3)
  assert.deepEqual(bFresh.request().postDataJSON(), { refresh: true })
  await fulfill(bFresh, doctor('ok', 'missing', newScan))
  await page.locator('[data-cut-matte-ready="true"]').waitFor()

  await page.locator('[data-test-project-a]').click()
  await fulfill(await waitFor(doctors, 4), doctor('missing'))
  await page.locator('[data-cut-matte-install-rvm]').click()
  await fulfill(await waitFor(setups, 3), { ok: true, result: { job_id: 'aba-a' } })
  const abaStatus = await waitFor(jobs, 3)
  await page.locator('[data-test-project-b]').click()
  await fulfill(await waitFor(doctors, 5), doctor('ok'))
  await page.locator('[data-test-project-a]').click()
  const newA = await waitFor(doctors, 6)
  await page.locator('[data-cut-matte-probing]').waitFor()
  assert.equal(await page.locator('[data-cut-matte]').getAttribute('data-cut-matte-ready'), 'false')
  await fulfill(newA, doctor('missing'))
  await fulfill(abaStatus, { ok: false, error: { code: 'unavailable', message: 'old A status refused' } })
  await page.waitForTimeout(75)
  assert.equal(doctors.length, 6, 'A-B-A return does not re-authorize old A completion')
  assert.equal(await page.locator('[data-cut-matte-error]').count(), 0, 'old A refusal cannot paint new A scope')
  assert.deepEqual(errors, [])
})
