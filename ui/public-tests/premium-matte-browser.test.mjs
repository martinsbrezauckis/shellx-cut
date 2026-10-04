import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

const card = (status, details = {}, hint = null) => ({ id: 'matte_premium', kind: 'matte', status, details, hint })
const doctor = (base, premium) => ({ ok: true, result: { scanned_at: new Date().toISOString(), cards: [
  { id: 'matte', kind: 'matte', status: base, details: {} }, premium,
] } })
const fulfill = (route, body) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) })

async function mounted(t) {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-premium-matte-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), cacheDir,
    configFile: false, plugins: [react(), { name: 'fixture', configureServer(vite) {
      vite.middlewares.use('/__premium_matte__', (_req, res) => {
        res.setHeader('content-type', 'text/html')
        res.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
          window.$RefreshReg$ = () => {}; window.$RefreshSig$ = () => type => type;
          window.__vite_plugin_react_preamble_installed__ = true;
        </script><script type="module" src="/public-tests/fixtures/premium-matte-browser.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 } })
  await server.listen(); t.after(() => server.close())
  const browser = await chromium.launch({ headless: true }); t.after(() => browser.close())
  const page = await browser.newPage(); const errors = []; const doctors = []; const edits = []; const setups = []
  page.on('pageerror', error => errors.push(String(error)))
  await page.route('**/api/verb/**', route => {
    const verb = new URL(route.request().url()).pathname.split('/').at(-1)
    if (verb === 'system.doctor') { doctors.push(route); return }
    if (verb === 'edit.matte') { edits.push(route); return fulfill(route, { ok: true, result: {} }) }
    if (verb === 'system.setup_matte') { setups.push(route); return fulfill(route, { ok: true, result: {} }) }
    return fulfill(route, { ok: true, result: {} })
  })
  await page.goto(new URL('/__premium_matte__', server.resolvedUrls.local[0]).href)
  const waitDoctor = async (count) => {
    for (let i = 0; i < 100 && doctors.length < count; i++) await page.waitForTimeout(25)
    assert.ok(doctors.length >= count, `expected Doctor request ${count}, saw ${doctors.length}`)
    return doctors[count - 1]
  }
  return { page, errors, doctors, edits, setups, waitDoctor }
}

test('Environment Premium statuses offer setup only for confirmed missing', async t => {
  const { page, errors, waitDoctor } = await mounted(t)
  await fulfill(await waitDoctor(1), doctor('ok', card('missing')))
  const row = page.locator('[data-cut-env-card="matte_premium"]')
  await row.locator('[data-cut-env-setup-matte]').waitFor()
  for (const [key, hint] of [['hardware', 'NVIDIA CUDA unavailable'], ['unknown', 'GPU probe timed out'], ['rejected', 'Prepared runtime rejected']]) {
    await page.locator(`[data-test-env-status="${key}"]`).click()
    assert.equal(await row.locator('[data-cut-env-setup-matte]').count(), 0, `${key} cannot reinstall`)
    assert.match(await row.locator('[data-cut-env-hint]').innerText(), new RegExp(hint))
  }
  await page.locator('[data-test-env-status="unknown"]').click()
  assert.equal(await row.locator('[data-cut-env-rescan]').count(), 1)
  await page.locator('[data-test-env-status="ready"]').click()
  assert.equal(await row.locator('[data-cut-env-setup-matte]').count(), 0)
  assert.deepEqual(errors, [])
})

test('Matte missing consent, hardware guidance, unknown and rejected states never offer invalid install', async t => {
  const { page, errors, doctors, setups, waitDoctor } = await mounted(t)
  await fulfill(await waitDoctor(1), doctor('ok', card('missing')))
  await page.locator('[data-cut-matte-model-premium]').click()
  await page.locator('[data-cut-matte-install-premium]').waitFor()
  for (const [premium, expected, hint] of [
    [card('degraded', { installed: true, cuda_available: false }, 'NVIDIA CUDA unavailable'), 'hardware-unavailable', 'NVIDIA CUDA unavailable'],
    [card('unknown', {}, 'GPU probe timed out'), 'unverified', 'GPU probe timed out'],
    [card('degraded', { installed: false, prepared_runtime: { rejected: true } }, 'Prepared runtime rejected'), 'unavailable', 'Prepared runtime rejected'],
  ]) {
    const nextDoctor = doctors.length + 1
    await page.locator('[data-cut-matte-premium-recheck]').click()
    const request = await waitDoctor(nextDoctor)
    assert.deepEqual(request.request().postDataJSON(), { refresh: true })
    await fulfill(request, doctor('ok', premium))
    const status = page.locator(`[data-cut-matte-premium-status="${expected}"]`)
    await status.waitFor()
    assert.match(await status.innerText(), new RegExp(hint))
    assert.equal(await status.locator('[data-cut-matte-install-premium]').count(), 0)
  }
  assert.equal(setups.length, 0)
  assert.deepEqual(errors, [])
})

test('Matte Requirements offers Premium setup only while confirmed missing', async t => {
  const { page, errors, doctors, setups, waitDoctor } = await mounted(t)
  await fulfill(await waitDoctor(1), doctor('missing', card('missing')))
  await page.locator('[data-cut-matte-requirements] [data-cut-matte-install-premium]').waitFor()
  for (const [premium, expected] of [
    [card('degraded', { installed: true, cuda_available: false }, 'NVIDIA CUDA unavailable'), 'hardware-unavailable'],
    [card('unknown', {}, 'GPU probe timed out'), 'unverified'],
    [card('degraded', { installed: false, prepared_runtime: { rejected: true } }, 'Prepared runtime rejected'), 'unavailable'],
  ]) {
    const nextDoctor = doctors.length + 1
    await page.locator('[data-cut-matte-recheck]').click()
    const request = await waitDoctor(nextDoctor)
    assert.deepEqual(request.request().postDataJSON(), { refresh: true })
    await fulfill(request, doctor('missing', premium))
    const status = page.locator(`[data-cut-matte-requirements] [data-cut-matte-premium-status="${expected}"]`)
    await status.waitFor()
    assert.equal(await status.locator('[data-cut-matte-install-premium]').count(), 0)
    assert.equal(await page.locator('[data-cut-matte-install-rvm]').count(), 1)
  }
  assert.equal(setups.length, 0)
  assert.deepEqual(errors, [])
})

test('fresh Premium loss falls back to RVM before Apply; Premium-only ready remains selectable', async t => {
  const { page, errors, doctors, edits, waitDoctor } = await mounted(t)
  await fulfill(await waitDoctor(1), doctor('ok', card('ok', { installed: true, cuda_available: true })))
  await page.locator('[data-cut-matte-model-premium]').click()
  assert.equal(await page.locator('[data-cut-matte-model-premium]').getAttribute('aria-selected'), 'true')
  await page.locator('[data-test-project-switch]').click()
  await fulfill(await waitDoctor(2), doctor('ok', card('degraded', { installed: true, cuda_available: false }, 'NVIDIA CUDA unavailable')))
  await page.locator('[data-cut-matte-premium-status="hardware-unavailable"]').waitFor()
  assert.equal(await page.locator('[data-cut-matte-model-rvm]').getAttribute('aria-selected'), 'true')
  await page.locator('[data-cut-matte-apply]').click()
  for (let i = 0; i < 100 && edits.length < 1; i++) await page.waitForTimeout(25)
  assert.equal(edits[0]?.request().postDataJSON().model, 'rvm')
  await page.locator('[data-test-project-switch]').click()
  await fulfill(await waitDoctor(3), doctor('missing', card('ok', { installed: true, cuda_available: true })))
  await page.locator('[data-cut-matte-ready="true"]').waitFor()
  assert.equal(await page.locator('[data-cut-matte-model-premium]').getAttribute('aria-selected'), 'true')
  assert.deepEqual(errors, [])
})
