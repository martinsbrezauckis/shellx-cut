import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer as createTcpServer } from 'node:net'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

for (const eventBeforeAppRead of [false, true]) {
test(`mounted App settles failed reconnect work and fences older Doctor reads behind events (${eventBeforeAppRead ? 'panel starts first' : 'App starts first'})`, async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-app-lifecycle-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), {
      name: 'observe-app-doctor-start', enforce: 'pre',
      transform(source, id) {
        if (!id.split('?')[0].endsWith('/src/App.tsx')) return null
        return source.replace('const refreshDoctor = useCallback(async (refresh = true) => {',
          'const refreshDoctor = useCallback(async (refresh = true) => { (window as any).__appDoctorStarted = true;')
      },
    }, { name: 'app-lifecycle-fixture', configureServer(vite) {
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

  const leftPanelImports = []
  if (!eventBeforeAppRead) {
    await page.route(/\/src\/panels\/LeftPanel(?:\/index\.tsx)?(?:\?.*)?$/, route => leftPanelImports.push(route))
  }
  await page.goto(new URL('/__app_lifecycle__', server.resolvedUrls.local[0]).href)
  await (await waitFor(projectRequests, 1)).abort('failed') // transport failure on first resync
  await page.waitForTimeout(50)
  await page.evaluate(() => window.__cutWs.onopen())
  await (await waitFor(projectRequests, 2)).fulfill(noProject)
  const initialVerify = await waitFor(verifyRequests, 1)
  if (!eventBeforeAppRead) {
    await initialVerify.fulfill({ status: 200, contentType: 'application/json', body: '{bad json' })
    await page.waitForFunction(() => window.__appDoctorStarted === true)
    await (await waitFor(leftPanelImports, 1)).continue()
  }
  const staleDoctor = await waitFor(doctorRequests, 1)
  await page.evaluate(report => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'doctor_updated', report }) }), doctor('missing', 'new-event'))
  await page.locator('[data-cut-wizard][data-cut-env-scanned-at="new-event"]').waitFor()
  let doctorOffset = 0
  if (eventBeforeAppRead) {
    await initialVerify.fulfill({ status: 200, contentType: 'application/json', body: '{bad json' })
    await page.waitForFunction(() => window.__appDoctorStarted === true)
    await reply(await waitFor(doctorRequests, 2), doctor('missing', 'new-event'))
    doctorOffset = 1
  }
  await staleDoctor.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: doctor('ok', 'old-read') }) })
  await page.waitForTimeout(75)
  assert.equal(await page.locator('[data-cut-wizard]').getAttribute('data-cut-env-scanned-at'), 'new-event')

  await page.evaluate(() => window.__cutWs.onopen())
  await (await waitFor(projectRequests, 3)).fulfill(noProject)
  await (await waitFor(verifyRequests, 2)).abort('failed')
  await (await waitFor(doctorRequests, 2 + doctorOffset)).fulfill({ status: 200, contentType: 'application/json', body: '{bad json' })
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
  await reply(await waitFor(doctorRequests, 3 + doctorOffset), doctor('ok', 'recovered-read'))
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
  const pendingDoctor = await waitFor(doctorRequests, 4 + doctorOffset)
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

}

// Hold the child read before App can begin its reconnect refresh. This is the
// same order regardless of which lazy panel finishes importing first.
test('shared setup reads keep newer reports and leave post-event refreshes fresh', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-doctor-order-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'doctor-order-fixture', configureServer(vite) {
      vite.middlewares.use('/__doctor_order__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}; window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/app-lifecycle-browser.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } })
  await page.addInitScript(() => {
    window.WebSocket = class {
      static OPEN = 1
      readyState = 1
      constructor() { window.__cutWs = this; queueMicrotask(() => this.onopen?.()) }
      send() {}
      close() {}
    }
    window.__scanTimes = []
    new MutationObserver(() => {
      const stamp = document.querySelector('[data-cut-wizard]')?.getAttribute('data-cut-env-scanned-at')
      if (stamp) window.__scanTimes.push(stamp)
    }).observe(document, { subtree: true, attributes: true, childList: true })
  })
  const projects = [], reads = [], verifies = []
  const reply = (route, result) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })
  const noProject = route => route.fulfill({ status: 200, contentType: 'application/json', body: '{"ok":false,"error":{"code":"no_project"}}' })
  await page.route('**/api/verb/**', route => {
    const verb = new URL(route.request().url()).pathname.split('/').pop()
    if (verb === 'project.state') projects.push(route)
    else if (verb === 'system.doctor') reads.push(route)
    else if (verb === 'verify.checks') verifies.push(route)
    else void noProject(route)
  })
  const waitFor = async (items, count) => {
    for (let i = 0; i < 200 && items.length < count; i += 1) await page.waitForTimeout(25)
    assert.ok(items.length >= count, `expected request ${count}, saw ${items.length}`)
    return items[count - 1]
  }
  const report = (ready, stamp) => ({ schema: 'shellx-cut/doctor/1', scanned_at: stamp,
    os: 'linux', arch: 'x86_64', app_version: '0.6.116', essential_ok: ready,
    cards: ['ffmpeg', 'ffprobe'].map(id => ({ id, kind: 'tool', status: ready ? 'ok' : 'missing', details: { runnable: ready } })) })
  const older = report(true, '2026-11-20T20:00:00.000Z')
  const newer = report(false, '2026-11-20T20:00:02.000Z')
  const emit = value => page.evaluate(report => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'doctor_updated', report }) }), value)
  await page.goto(new URL('/__doctor_order__', server.resolvedUrls.local[0]).href)
  await waitFor(projects, 1)
  const held = await waitFor(reads, 1)
  await emit(newer)
  await page.locator(`[data-cut-wizard][data-cut-env-scanned-at="${newer.scanned_at}"]`).waitFor()
  if (process.env.CUT_DOCTOR_SHOTS) await page.screenshot({ path: `${process.env.CUT_DOCTOR_SHOTS}/shared-before-old.png` })
  await noProject(projects[0])
  await noProject(await waitFor(verifies, 1))
  await page.waitForTimeout(100) // App reaches its refresh while the child read is held.
  await reply(held, older)
  await page.waitForTimeout(100)
  if (process.env.CUT_DOCTOR_SHOTS) await page.screenshot({ path: `${process.env.CUT_DOCTOR_SHOTS}/shared-after-old.png` })
  assert.match(await page.locator('[data-cut-env-chip]').textContent(), /Video setup needed/)
  assert.equal(await page.locator('[data-cut-wizard]').getAttribute('data-cut-env-scanned-at'), newer.scanned_at)
  assert.deepEqual([...new Set(await page.evaluate(() => window.__scanTimes))], [newer.scanned_at], 'scan time never goes backwards')
  await reply(await waitFor(reads, 2), newer)
  await page.waitForTimeout(75)
  await emit(older)
  await page.waitForTimeout(75)
  assert.equal(await page.locator('[data-cut-wizard]').getAttribute('data-cut-env-scanned-at'), newer.scanned_at, 'older events cannot replace newer reports either')

  // Every consumer uses this same client. Simultaneous asks still share one
  // HTTP read; an event between its start and reply fences all its borrowers.
  await page.evaluate(async () => {
    const { fetchDoctor } = await import('/src/lib/doctor.ts')
    window.__sharedReplies = Promise.all([fetchDoctor(), fetchDoctor(), fetchDoctor()])
  })
  const shared = await waitFor(reads, 3)
  await page.waitForTimeout(50)
  assert.equal(reads.length, 3, 'three simultaneous askers make one HTTP request')
  const newest = report(false, '2026-11-20T20:00:04.000Z')
  await emit(newest)
  await reply(shared, older)
  assert.deepEqual(await page.evaluate(async () => (await window.__sharedReplies).map(r => r.scanned_at)), Array(3).fill(newest.scanned_at), 'all shared consumers receive the current event report')
  await page.evaluate(async () => {
    const { fetchDoctor } = await import('/src/lib/doctor.ts')
    window.__sharedScanReplies = Promise.all([fetchDoctor(true), fetchDoctor(true), fetchDoctor()])
  })
  const scan = await waitFor(reads, 4)
  await page.waitForTimeout(50)
  assert.equal(reads.length, 4, 'two forced scans and a joining read make one request')
  assert.deepEqual(scan.request().postDataJSON(), { refresh: true })
  await reply(scan, older)
  assert.deepEqual(await page.evaluate(async () => (await window.__sharedScanReplies).map(r => r.scanned_at)), Array(3).fill(newest.scanned_at), 'older scan timestamps cannot replace the displayed report')
  const precise = report(false, '2026-11-20T20:00:04.123456790+00:00')
  await emit(precise)
  await emit(report(true, '2026-11-20T22:00:04.123456789+02:00'))
  await page.waitForTimeout(75)
  assert.match(await page.locator('[data-cut-env-chip]').textContent(), /Video setup needed/)
  assert.equal(await page.locator('[data-cut-wizard]').getAttribute('data-cut-env-scanned-at'), precise.scanned_at, 'sub-millisecond older events cannot reverse the scan time')
  console.log('doctor order: held panel read fenced; post-event refresh fresh; older event fenced; simultaneous askers=1; shared borrowers=3 fenced')
})

test('project switch completion preserves later workspace and panel choices', { timeout: 120_000 }, async t => {
  const portProbe = createTcpServer()
  await new Promise(resolveListen => portProbe.listen(0, '127.0.0.1', resolveListen))
  const port = portProbe.address().port
  await new Promise(resolveClose => portProbe.close(resolveClose))
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-project-navigation-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir,
    server: { host: '127.0.0.1', port, strictPort: true },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1440, height: 980 } })
  page.setDefaultTimeout(5_000)
  const pageErrors = []
  page.on('pageerror', error => pageErrors.push(String(error)))
  await page.addInitScript(() => {
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
  let nextHeldReply
  let projectNumber = 0
  let hold = false
  const project = () => ({ schema: 'shellx-cut/project/1', name: `Navigation ${projectNumber}`,
    project_identity: { schema: 'shellx-cut/project-identity/1', origin_path_sha256: String(projectNumber), project_name: `Navigation ${projectNumber}` },
    project_revision: `revision-${projectNumber}`, settings: { width: 1920, height: 1080, fps: 30 },
    assets: {}, tracks: [], markers: [], caption_styles: {}, checkpoints: [] })
  await page.route('**/api/verb/**', async route => {
    const verb = new URL(route.request().url()).pathname.split('/').pop()
    let result
    if (verb === 'project.state') result = project()
    else if (verb === 'project.ops') result = { ops: [], has_more: false }
    else if (verb === 'system.doctor') result = { schema: 'shellx-cut/doctor/1', scanned_at: 'navigation-fixture',
      os: 'windows', arch: 'x86_64', app_version: '0.6.116', essential_ok: true, cards: [] }
    const body = JSON.stringify(result ? { ok: true, result } : { ok: false })
    const reply = () => route.fulfill({ status: 200, contentType: 'application/json', body })
    if (verb === 'project.state' && hold) {
      hold = false
      nextHeldReply(reply)
    } else await reply()
  })
  await page.goto(server.resolvedUrls.local[0])
  await page.locator('[data-cut-project]').filter({ hasText: 'Navigation 0' }).waitFor()
  const mode = () => page.locator('[data-cut-app-root]').getAttribute('data-cut-workspace-mode')
  const switchProject = async (held = true) => {
    projectNumber += 1
    hold = held
    const pending = held ? new Promise(resolveReply => { nextHeldReply = resolveReply }) : null
    await page.evaluate(() => window.__cutWs.onmessage({ data: JSON.stringify({ type: 'project_changed', open: true }) }))
    return pending ? await Promise.race([pending, new Promise((_, reject) => {
      const timer = setTimeout(() => reject(new Error('project.state was not requested')), 5_000)
      timer.unref()
    })]) : null
  }
  const finish = async reply => {
    await reply()
    await page.locator('[data-cut-project]').filter({ hasText: `Navigation ${projectNumber}` }).waitFor()
  }
  const failures = []
  const scenarios = [
    ['Record tab', async () => {
      await page.locator('[data-cut-mode="record"]').click()
      await page.locator('[data-cut-panel="record"]').waitFor()
    }, 'record'],
    ['Library button', () => page.locator('[data-cut-library-btn]').click(), 'library'],
    ['Edit tab', () => page.locator('[data-cut-mode="edit"]').click(), 'edit'],
    ['Edit after Record', async () => {
      await page.locator('[data-cut-mode="record"]').click()
      await page.locator('[data-cut-mode="edit"]').click()
    }, 'edit'],
    ['Projects toggle', () => page.locator('[data-cut-projects-btn]').click(), 'edit'],
    ['Assets panel', () => page.locator('[data-cut-left-tab="assets"]').click(), 'edit'],
    ['Transcript panel', () => page.locator('[data-cut-left-tab="transcript"]').click(), 'edit'],
    ['Library source reveal', () => page.evaluate(() => document.dispatchEvent(new CustomEvent('cut:reveal-registered-source', {
      detail: { assetId: 'source', destination: 'library' },
    }))), 'library'],
    ['keyboard Review', () => page.keyboard.press('r'), 'edit'],
  ]
  for (const [name, navigate, expectedMode] of scenarios) {
    await page.evaluate(() => document.dispatchEvent(new CustomEvent('cut:open-ui-surface', { detail: { id: 'transcript' } })))
    const reply = await switchProject()
    try {
      await navigate()
      await page.waitForFunction(expected => window.__cutUiState?.active_workspace === expected, expectedMode)
      const before = await page.evaluate(() => window.__cutUiState.left)
      if (process.env.CUT_NAV_SHOTS && name === 'Record tab') await page.screenshot({ path: `${process.env.CUT_NAV_SHOTS}/before-reply.png` })
      await finish(reply)
      if (process.env.CUT_NAV_SHOTS && name === 'Record tab') await page.screenshot({ path: `${process.env.CUT_NAV_SHOTS}/after-reply.png` })
      assert.equal(await mode(), expectedMode, name)
      const after = await page.evaluate(() => window.__cutUiState.left)
      for (const field of ['active_tab', 'collapsed']) assert.equal(after[field], before[field], `${name}: ${field}`)
      console.log(`PASS held reply preserves ${name}`)
    } catch (error) { failures.push(`${name}: ${error.message}`); console.log(`FAIL ${failures.at(-1)}`); await reply().catch(() => {}) }
  }
  await page.locator('[data-cut-mode="record"]').click()
  await finish(await switchProject())
  assert.equal(await mode(), 'edit', 'without a later choice a project switch still opens Edit')
  console.log('PASS no later navigation opens Edit')
  for (let i = 0; i < 10; i += 1) {
    await switchProject(false)
    await page.locator('[data-cut-mode="record"]').click()
    await page.locator('[data-cut-project]').filter({ hasText: `Navigation ${projectNumber}` }).waitFor()
    assert.equal(await mode(), 'record', `fast navigation ${i + 1}`)
    await page.locator('[data-cut-mode="edit"]').click()
  }
  console.log('PASS ten fast UI navigations without a held reply')
  assert.deepEqual(pageErrors, [])
  assert.deepEqual(failures, [])
})
