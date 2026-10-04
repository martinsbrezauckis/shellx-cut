import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('mounted Export preserves every successful interchange warning at desktop and narrow widths', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-interchange-warning-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false, plugins: [react(), {
      name: 'interchange-warning-fixture',
      configureServer(vite) {
        vite.middlewares.use('/__interchange_warning__', (_request, response) => {
          response.setHeader('content-type', 'text/html')
          response.end(`<div id="root"></div><script type="module">
            import RefreshRuntime from '/@react-refresh'
            RefreshRuntime.injectIntoGlobalHook(window)
            window.$RefreshReg$ = () => {}
            window.$RefreshSig$ = () => type => type
            window.__vite_plugin_react_preamble_installed__ = true
          </script><script type="module" src="/public-tests/fixtures/interchange-export-warnings-browser.tsx"></script>`)
        })
      },
    }], server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  let exportCalls = 0
  const pendingExports = new Map()
  const warningReply = {
    ok: true,
    result: { path: '/private/output/timeline.fcpxml', format: 'fcpxml' },
    warnings: [
      { code: 'richness_dropped', message: 'Muted range from 0.2s to 0.8s is not represented in this XML.' },
      { code: 'captions_not_in_xml', message: 'Caption track must be exported separately.' },
    ],
  }
  await page.route('**/api/verb/**', async route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    let reply = { ok: true, result: name === 'jobs.list' ? { jobs: [] } : {} }
    if (name === 'export.xml') {
      exportCalls += 1
      if (exportCalls === 5 || exportCalls === 7 || exportCalls === 8) {
        pendingExports.set(exportCalls, route)
        return
      }
      reply = exportCalls === 1 || exportCalls === 3 ? warningReply
        : exportCalls === 2 ? { ok: true, result: { path: '/private/output/clean.fcpxml', format: 'fcpxml' } }
          : exportCalls === 6 ? { ok: true, result: { path: '/private/output/newer-clean.fcpxml', format: 'fcpxml' } }
            : { ok: false, error: { code: 'conflict', message: 'Nothing to export' } }
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(reply) })
  })
  await page.goto(new URL('/__interchange_warning__', server.resolvedUrls.local[0]).href)
  const exportXml = async () => {
    const requested = page.waitForRequest(item => new URL(item.url()).pathname.endsWith('/api/verb/export.xml'))
    await page.locator('[data-cut-export-btn]').click()
    await page.locator('[data-cut-export-option="fcpxml"]').click()
    const request = await requested
    const response = await request.response()
    assert.ok(response, 'the exact Export submission received a response')
    return response.json()
  }
  const waitPending = async call => {
    for (let i = 0; i < 100; i += 1) {
      if (pendingExports.has(call)) return pendingExports.get(call)
      await page.waitForTimeout(25)
    }
    assert.fail(`deferred Export request ${call} did not arrive`)
  }
  const settleReact = () => page.evaluate(() => new Promise(resolve =>
    requestAnimationFrame(() => requestAnimationFrame(resolve))))

  assert.equal((await exportXml()).ok, true)
  const notice = page.locator('[data-cut-export-warnings]')
  await notice.waitFor({ state: 'visible' })
  assert.match(await notice.textContent(), /Exported with warnings/)
  assert.equal(await page.locator('[data-cut-export-warning-file]').textContent(), 'Saved timeline.fcpxml')
  assert.deepEqual(await page.locator('[data-cut-export-warning]').allTextContents(), warningReply.warnings.map(warning => warning.message))
  assert.doesNotMatch(await notice.textContent(), /\/private\/output/)
  await page.waitForTimeout(5_250)
  assert.equal(await notice.isVisible(), true, 'the full list outlives the five-second flash')
  await page.setViewportSize({ width: 1100, height: 800 })
  assert.equal(await notice.isVisible(), true, 'warning notice remains visible at the two-row topbar width')
  const box = await notice.boundingBox()
  assert.ok(box && box.x >= 0 && box.x + box.width <= 1100, 'warning card fits the narrow viewport')

  assert.equal((await exportXml()).ok, true)
  await notice.waitFor({ state: 'detached' })
  assert.equal(await notice.count(), 0, 'a clean successful export clears prior warnings')
  assert.equal((await exportXml()).ok, true)
  await notice.waitFor({ state: 'visible' })
  await page.evaluate(() => window.reviseFixtureProject())
  assert.equal(await notice.isVisible(), true, 'ordinary project revision retains the warning list')
  assert.equal((await exportXml()).ok, false)
  await notice.waitFor({ state: 'detached' })
  assert.equal(await notice.count(), 0, 'a failed export does not leave stale success warnings')

  const oldExport = exportXml()
  const oldRoute = await waitPending(5)
  assert.equal((await exportXml()).ok, true)
  await oldRoute.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(warningReply) })
  assert.equal((await oldExport).ok, true)
  await settleReact()
  assert.equal(await notice.count(), 0, 'late older warning cannot replace a newer clean export')

  const switchedExport = exportXml()
  const switchedRoute = await waitPending(7)
  await page.evaluate(() => window.switchFixtureProject())
  await page.waitForFunction(() => window.fixtureProjectName === 'other-project')
  await switchedRoute.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(warningReply) })
  assert.equal((await switchedExport).ok, true)
  await settleReact()
  assert.equal(await notice.count(), 0, 'an old project reply cannot display warnings in the new project')

  const unmountedExport = exportXml()
  const unmountedRoute = await waitPending(8)
  await page.evaluate(() => window.unmountFixture())
  await unmountedRoute.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(warningReply) })
  assert.equal((await unmountedExport).ok, true)
  await settleReact()
  assert.equal(await notice.count(), 0, 'an unmounted TopBar ignores its late export response')
  assert.deepEqual(errors, [])
})
