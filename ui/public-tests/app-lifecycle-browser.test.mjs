import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('mounted App settles failed reconnect work and fences older Doctor reads behind events', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-app-lifecycle-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'app-lifecycle-fixture', configureServer(vite) {
      vite.middlewares.use('/__app_lifecycle__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/app-lifecycle-browser.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const pageErrors = []
  page.on('pageerror', error => pageErrors.push(String(error)))
  await page.addInitScript(() => {
    window.__cutUnhandled = []
    window.__cutFeedback = []
    window.addEventListener('unhandledrejection', event => window.__cutUnhandled.push(String(event.reason)))
    document.addEventListener('cut:user-action-feedback', event => window.__cutFeedback.push(event.detail.message))
    window.WebSocket = class {
      static OPEN = 1
      readyState = 1
      constructor() { window.__cutWs = this; queueMicrotask(() => this.onopen?.()) }
      send(data) {
        const message = JSON.parse(data)
        if (message.type === 'ui_state') window.__cutUiState = message.state
      }
      close() {}
    }
  })
  const projectRequests = []
  const doctorRequests = []
  const downloadRequests = []
  const verifyRequests = []
  const navRequests = []
  const opsRequests = []
  let holdNextOps = false
  await page.route('**/api/verb/**', route => {
    const verb = new URL(route.request().url()).pathname.split('/').pop()
    if (verb === 'project.state') projectRequests.push(route)
    else if (verb === 'system.doctor') doctorRequests.push(route)
    else if (verb === 'system.fetch_tool') {
      downloadRequests.push(route.request().postDataJSON())
      void route.fulfill({ status: 200, contentType: 'application/json', body: '{"ok":false}' })
    }
    else if (verb === 'verify.checks') verifyRequests.push(route)
    else if (verb === 'project.undo' || verb === 'project.redo') navRequests.push(route)
    else if (verb === 'project.ops') {
      if (holdNextOps) { opsRequests.push(route); holdNextOps = false }
      else void route.fulfill({ status: 200, contentType: 'application/json', body: '{"ok":false}' })
    }
    else void route.fulfill({ status: 200, contentType: 'application/json', body: '{"ok":false}' })
  })
  const waitFor = async (items, count) => {
    for (let i = 0; i < 120; i += 1) {
      if (items.length >= count) return items[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected request ${count}, saw ${items.length}; ${pageErrors.join('; ')}`)
  }
  const noProject = { status: 200, contentType: 'application/json', body: '{"ok":false,"error":{"code":"no_project"}}' }
  const doctor = (status, scannedAt) => ({ schema: 'shellx-cut/doctor/1', scanned_at: scannedAt,
    os: 'windows', arch: 'x86_64', app_version: '0.6.116', essential_ok: status === 'ok',
    cards: ['ffmpeg', 'ffprobe'].map(id => ({ id, kind: 'tool', status, details: { runnable: status === 'ok' } })) })
  const project = name => ({ schema: 'shellx-cut/project/1', name,
    project_identity: { schema: 'shellx-cut/project-identity/1', origin_path_sha256: name, project_name: name },
    project_revision: `rev-${name}`, settings: { width: 1920, height: 1080, fps: 30 },
    assets: {}, tracks: [], markers: [], caption_styles: {}, checkpoints: [] })
  const receipt = renderId => ({ render_id: renderId, ts: '2026-10-10T10:00:00Z',
    output_path: `/tmp/exports/${renderId}.mp4`, output_hash: 'a'.repeat(64), duration_ms: 1000,
    preset: 'test', at_op: 'op_000001', checks: [{ name: 'output', pass: true }], pass: true })
  const reply = (route, result) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })

  await page.goto(new URL('/__app_lifecycle__', server.resolvedUrls.local[0]).href)
  await (await waitFor(projectRequests, 1)).abort('failed') // transport failure on first resync
  await page.waitForTimeout(50)
  await page.evaluate(() => window.__cutWs.onopen())
  await (await waitFor(projectRequests, 2)).fulfill(noProject)
  await (await waitFor(verifyRequests, 1)).fulfill({ status: 200, contentType: 'application/json', body: '{bad json' })
  const staleDoctor = await waitFor(doctorRequests, 1)
  await page.evaluate(report => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'doctor_updated', report }) }), doctor('missing', 'new-event'))
  await page.locator('[data-cut-wizard][data-cut-env-scanned-at="new-event"]').waitFor()
  await staleDoctor.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: doctor('ok', 'old-read') }) })
  await page.waitForTimeout(75)
  assert.equal(await page.locator('[data-cut-wizard]').getAttribute('data-cut-env-scanned-at'), 'new-event')

  await page.evaluate(() => window.__cutWs.onopen())
  await (await waitFor(projectRequests, 3)).fulfill(noProject)
  await (await waitFor(verifyRequests, 2)).abort('failed')
  await (await waitFor(doctorRequests, 2)).fulfill({ status: 200, contentType: 'application/json', body: '{bad json' })
  await page.waitForTimeout(75)
  await page.locator('[data-cut-wizard-dismiss]').click()

  await page.keyboard.press('Control+z')
  const oldUndo = await waitFor(navRequests, 1)
  await page.keyboard.press('Control+y') // queued behind the unresolved undo
  await page.evaluate(() => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'project_changed', open: false }) }))
  await (await waitFor(projectRequests, 4)).fulfill(noProject)
  await oldUndo.abort('failed')
  await page.waitForTimeout(75)
  assert.equal(navRequests.length, 1, 'old queued redo must be skipped after project switch')
  assert.deepEqual(await page.evaluate(() => window.__cutFeedback), [], 'obsolete action cannot show feedback')
  await page.keyboard.press('Control+y')
  await (await waitFor(navRequests, 2)).abort('failed')
  await page.waitForTimeout(75)
  assert.equal(navRequests.length, 2, 'settled history tail admits a later action immediately')
  assert.equal((await page.evaluate(() => window.__cutFeedback)).length, 1, 'current failed action has visible feedback')

  await page.evaluate(() => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'project_changed', open: true }) }))
  await reply(await waitFor(projectRequests, 5), project('Project A'))
  await page.locator('[data-cut-project]').filter({ hasText: 'Project A' }).waitFor()
  await page.evaluate(() => window.__cutWs.onopen())
  await reply(await waitFor(projectRequests, 6), project('Project A'))
  const oldVerify = await waitFor(verifyRequests, 3)
  await page.keyboard.press('Control+z')
  const projectAUndo = await waitFor(navRequests, 3)
  await page.keyboard.press('Control+y')

  await page.evaluate(() => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'project_changed', open: true }) }))
  const obsoleteSwitch = await waitFor(projectRequests, 7)
  await page.evaluate(() => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'project_changed', open: true }) }))
  await reply(obsoleteSwitch, project('Project Obsolete'))
  await reply(await waitFor(projectRequests, 8), project('Project B'))
  await page.locator('[data-cut-project]').filter({ hasText: 'Project B' }).waitFor()
  await projectAUndo.abort('failed')
  await reply(oldVerify, receipt('old-project-receipt'))
  await page.waitForTimeout(75)
  assert.match(await page.locator('[data-cut-project]').textContent(), /Project B/)
  assert.doesNotMatch(await page.locator('[data-cut-project]').textContent(), /Obsolete/)
  assert.equal(navRequests.length, 3, 'Project A queued redo cannot run in Project B')
  assert.equal((await page.evaluate(() => window.__cutFeedback)).length, 1, 'Project A failure cannot show feedback in Project B')

  await page.evaluate(() => window.__cutWs.onopen())
  await reply(await waitFor(projectRequests, 9), project('Project B'))
  await reply(await waitFor(verifyRequests, 4), receipt('new-project-receipt'))
  await reply(await waitFor(doctorRequests, 3), doctor('ok', 'recovered-read'))
  await page.keyboard.press('r')
  await page.locator('[data-cut-review-tab="receipts"]').click()
  await page.locator('[data-cut-receipt="new-project-receipt"]').waitFor()
  assert.equal(await page.locator('[data-cut-receipt="old-project-receipt"]').count(), 0)

  holdNextOps = true
  await page.evaluate(() => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'project_changed', open: true }) }))
  await reply(await waitFor(projectRequests, 10), project('Project C'))
  const staleHistory = await waitFor(opsRequests, 1)
  await page.evaluate(() => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'project_changed', open: true }) }))
  await reply(await waitFor(projectRequests, 11), project('Project D'))
  await page.locator('[data-cut-project]').filter({ hasText: 'Project D' }).waitFor()
  await reply(staleHistory, { ops: [{ op_id: 'op_000001', verb: 'old.project', ts: '2026-10-10T10:00:00Z', args: {} }], has_more: false })
  await page.locator('[data-cut-review-tab="ops"]').click()
  await page.waitForTimeout(75)
  assert.doesNotMatch(await page.locator('[data-cut-panel="review"]').textContent(), /old\.project/)

  await page.locator('[data-cut-env-chip]').click()
  await page.locator('[data-cut-settings-category="video-performance"]').click()
  for (const missing of ['ffmpeg', 'ffprobe']) {
    const ready = doctor('ok', `before-${missing}`)
    await page.evaluate(report => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'doctor_updated', report }) }), ready)
    await page.locator(`[data-cut-env-card="${missing}"][data-cut-env-status="ok"]`).waitFor()
    assert.match(await page.locator('[data-cut-env-chip]').textContent(), /Setup ready/)
    assert.equal(await page.locator(`[data-cut-env-download="${missing}"]`).count(), 0)
    if (process.env.CUT_DOCTOR_SHOTS) await page.screenshot({ path: `${process.env.CUT_DOCTOR_SHOTS}/before-${missing}.png` })
    const changed = { ...ready, essential_ok: false, cards: ready.cards.map(card => card.id === missing
      ? { ...card, status: 'missing', details: { runnable: false } } : card) }
    await page.evaluate(report => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'doctor_updated', report }) }), changed)
    await page.locator(`[data-cut-env-card="${missing}"][data-cut-env-status="missing"]`).waitFor()
    assert.match(await page.locator(`[data-cut-env-card="${missing}"]`).textContent(), /Needs setup/)
    assert.equal(await page.locator(`[data-cut-env-download="${missing}"]`).isVisible(), true)
    assert.match(await page.locator('[data-cut-env-chip]').textContent(), /Video setup needed/)
    await page.waitForFunction(id => window.__cutUiState?.overlays?.settings_cards?.some(card =>
      card.id === id && card.status === 'missing' && card.install_offered === true), missing)
    if (process.env.CUT_DOCTOR_SHOTS) await page.screenshot({ path: `${process.env.CUT_DOCTOR_SHOTS}/after-${missing}.png` })
    console.log(`PASS live Settings ${missing}: Ready -> Needs setup + Download; footer Video setup needed; ui.state missing/install_offered without restart`)
  }
  await page.locator('[data-cut-env-download="ffprobe"]').click()
  assert.deepEqual(await waitFor(downloadRequests, 1), { tool: 'ffmpeg', accept_license: true })
  await page.locator('[data-cut-environment-refresh]').click()
  const pendingDoctor = await waitFor(doctorRequests, 4)
  await page.evaluate(() => window.__cutWs.onopen())
  const pendingUnmount = await waitFor(projectRequests, 12)
  await page.evaluate(() => window.__cutUnmount())
  await reply(pendingUnmount, project('Project After Unmount'))
  await reply(pendingDoctor, doctor('missing', 'after-unmount'))
  await page.waitForTimeout(75)
  assert.equal(verifyRequests.length, 4, 'unmounted project pull cannot start receipt recovery')
  assert.deepEqual(await page.evaluate(() => window.__cutUnhandled), [])
  assert.deepEqual(pageErrors, [])
})
