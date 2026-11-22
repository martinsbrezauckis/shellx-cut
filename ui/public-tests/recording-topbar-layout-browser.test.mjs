import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('every exit status of the recording top bar is shown whole beside reachable toolbar controls, in both themes', async t => {
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
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  await page.route('**/api/verb/**', route => route.fulfill({
    status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: { sequences: [] } }),
  }))
  const phases = {
    countdown: 'Cancel the countdown before returning to Edit.',
    starting: 'Starting the recording. Stay on the Record page.',
    recording: 'Stop the recording before returning to Edit.',
    finalizing: 'Finishing the recording. Stay on the Record page.',
    recovery: 'Check this recording before returning to Edit.',
  }
  // RECORDING_EXIT_REASON_MAX_CHARS in recordingWorkspaceAdmission.ts; the bar is only sized for this much text.
  const maxChars = 52
  const reason = page.locator('[data-cut-record-back-reason]')
  const url = phase => new URL(`/__recording_topbar__?phase=${phase}`, server.resolvedUrls.local[0]).href
  const setTheme = theme => page.evaluate(name => {
    // Dark is the :root default with no attribute (lib/themePref.ts).
    if (name === 'light') document.documentElement.setAttribute('data-theme', 'light')
    else document.documentElement.removeAttribute('data-theme')
  }, theme)
  // 1280x800 is the window the owner saw the status cut off in; 1100 is the narrowest supported window.
  for (const [phase, expected] of Object.entries(phases)) {
    await page.goto(url(phase))
    await reason.waitFor()
    assert.equal(await reason.textContent(), expected)
    assert.equal(await reason.getAttribute('title'), expected, 'full message stays available as the tooltip')
    assert.ok(expected.length <= maxChars, `${phase}: ${expected.length} characters is over the ${maxChars} the bar is sized for`)
    assert.equal(await page.locator('[data-cut-record-back-edit]').isDisabled(), true, 'capture exit guard remains enforced')
    const colors = {}
    for (const theme of ['dark', 'light']) {
      await setTheme(theme)
      for (const [width, height] of [[1984, 900], [1600, 900], [1280, 800], [1100, 800]]) {
        await page.setViewportSize({ width, height })
        const geometry = await page.evaluate(() => {
          const status = document.querySelector('[data-cut-record-back-reason]')
          const rect = status.getBoundingClientRect()
          const style = getComputedStyle(status)
          const controls = [...document.querySelectorAll('[data-cut-settings-btn], [data-cut-manual-link], [data-cut-theme-toggle]')]
          return {
            rect: { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom },
            overflow: style.overflowX, color: style.color, scrollWidth: status.scrollWidth, clientWidth: status.clientWidth,
            controls: controls.map(control => {
              const r = control.getBoundingClientRect()
              const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)
              return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, hit: control.contains(hit) }
            }),
          }
        })
        const where = `${phase} ${theme} ${width}x${height}`
        colors[theme] = geometry.color
        assert.ok(geometry.scrollWidth <= geometry.clientWidth, `${where}: status is cut off (${geometry.scrollWidth} > ${geometry.clientWidth})`)
        assert.equal(geometry.overflow, 'hidden', `${where}: a longer status still cannot paint over the controls`)
        assert.equal(geometry.controls.length, 3)
        for (const control of geometry.controls) {
          assert.ok(geometry.rect.right <= control.left, `${where}: status and control have separate horizontal slots`)
          assert.ok(control.right <= width, `${where}: toolbar control remains in viewport`)
          assert.equal(control.hit, true, `${where}: toolbar button wins its own hit test`)
        }
        if (process.env.CUT_TOPBAR_SCREENSHOT_DIR && (phase === 'finalizing' || phase === 'starting') && height === 800) {
          await page.screenshot({ path: join(process.env.CUT_TOPBAR_SCREENSHOT_DIR, `recording-topbar-${phase}-${theme}-${width}x${height}.png`), clip: { x: 0, y: 0, width, height: 56 } })
        }
      }
    }
    assert.notEqual(colors.dark, colors.light, `${phase}: both themes were really applied`)
  }
  await page.goto(url('finalizing'))
  await reason.waitFor()
  await setTheme('dark')
  await page.setViewportSize({ width: 1280, height: 800 })
  await page.locator('[data-cut-settings-btn]').click()
  await page.locator('[data-cut-manual-link]').click()
  assert.equal(await page.locator('body').getAttribute('data-settings-opened'), 'true')
  assert.equal(await page.locator('body').getAttribute('data-manual-opened'), 'true')
  const theme = await page.locator('[data-cut-theme-toggle]').getAttribute('data-cut-theme')
  await page.locator('[data-cut-theme-toggle]').click()
  assert.notEqual(await page.locator('[data-cut-theme-toggle]').getAttribute('data-cut-theme'), theme)
  assert.deepEqual(errors, [])
})
