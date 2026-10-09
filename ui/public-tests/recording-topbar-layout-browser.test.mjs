import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('finishing recording status stays bounded beside reachable toolbar controls', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-recording-topbar-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'recording-topbar-layout-fixture', configureServer(vite) {
      vite.middlewares.use('/__recording_topbar__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/recording-topbar-layout.tsx"></script>`)
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
  await page.route('**/api/verb/**', route => route.fulfill({
    status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: { sequences: [] } }),
  }))
  await page.goto(new URL('/__recording_topbar__', server.resolvedUrls.local[0]).href)
  const reason = page.locator('[data-cut-record-back-reason]')
  await reason.waitFor()
  const fullReason = 'Finishing the recording. Stay in Recording Studio until it completes.'
  assert.equal(await reason.textContent(), fullReason)
  assert.equal(await reason.getAttribute('title'), fullReason, 'full message remains available when visually truncated')
  assert.equal(await page.locator('[data-cut-record-back-edit]').isDisabled(), true, 'capture exit guard remains enforced')
  for (const width of [1984, 1600, 1280, 1100]) {
    await page.setViewportSize({ width, height: 900 })
    const geometry = await page.evaluate(() => {
      const status = document.querySelector('[data-cut-record-back-reason]')
      const rect = status.getBoundingClientRect()
      const style = getComputedStyle(status)
      const controls = [...document.querySelectorAll('[data-cut-settings-btn], [data-cut-manual-link], [data-cut-theme-toggle]')]
      return {
        rect: { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom },
        overflow: style.overflowX, ellipsis: style.textOverflow, truncated: status.scrollWidth > status.clientWidth,
        controls: controls.map(control => {
          const r = control.getBoundingClientRect()
          const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)
          return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, hit: control.contains(hit) }
        }),
      }
    })
    assert.equal(geometry.overflow, 'hidden', `${width}: overflowing status cannot paint over controls`)
    assert.equal(geometry.ellipsis, 'ellipsis')
    assert.equal(geometry.truncated, true, `${width}: long finishing message is visibly bounded`)
    assert.equal(geometry.controls.length, 3)
    for (const control of geometry.controls) {
      assert.ok(geometry.rect.right <= control.left, `${width}: status and control have separate horizontal slots`)
      assert.ok(control.right <= width, `${width}: toolbar control remains in viewport`)
      assert.equal(control.hit, true, `${width}: toolbar button wins its own hit test`)
    }
    if (process.env.CUT_TOPBAR_SCREENSHOT_DIR) {
      await page.locator('[data-cut-recording-chrome]').screenshot({ path: join(process.env.CUT_TOPBAR_SCREENSHOT_DIR, `recording-topbar-${width}.png`) })
    }
  }
  await page.locator('[data-cut-settings-btn]').click()
  await page.locator('[data-cut-manual-link]').click()
  assert.equal(await page.locator('body').getAttribute('data-settings-opened'), 'true')
  assert.equal(await page.locator('body').getAttribute('data-manual-opened'), 'true')
  const theme = await page.locator('[data-cut-theme-toggle]').getAttribute('data-cut-theme')
  await page.locator('[data-cut-theme-toggle]').click()
  assert.notEqual(await page.locator('[data-cut-theme-toggle]').getAttribute('data-cut-theme'), theme)
  assert.deepEqual(errors, [])
})
