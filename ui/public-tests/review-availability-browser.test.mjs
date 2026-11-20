import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('mounted Review disables failed availability, recovers, and ignores old project and unmounted work', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-review-availability-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'review-availability-fixture', configureServer(vite) {
      vite.middlewares.use('/__review_availability__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/review-availability-browser.tsx"></script>`)
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
  await page.addInitScript(() => {
    window.__reviewUnhandled = []
    window.addEventListener('unhandledrejection', event => window.__reviewUnhandled.push(String(event.reason)))
  })
  const requests = []
  await page.route('**/api/verb/project.ops', route => { requests.push(route) })
  const nextRequest = async count => {
    for (let i = 0; i < 120; i += 1) {
      if (requests.length >= count) return requests[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected request ${count}, saw ${requests.length}; ${errors.join('; ')}`)
  }
  const reply = (route, undo, redo) => route.fulfill({ status: 200, contentType: 'application/json',
    body: JSON.stringify({ ok: true, result: { undo_available: undo, redo_available: redo } }) })
  const refresh = async () => {
    await page.locator('[data-cut-review-tab="receipts"]').click()
    await page.locator('[data-cut-review-tab="ops"]').click()
  }
  const undo = page.locator('[data-cut-undo-bar] [data-cut-action="undo"]')
  const redo = page.locator('[data-cut-undo-bar] [data-cut-action="redo"]')

  await page.goto(new URL('/__review_availability__', server.resolvedUrls.local[0]).href)
  await reply(await nextRequest(1), true, true)
  await undo.waitFor({ state: 'visible' })
  await page.waitForFunction(() => !document.querySelector('[data-cut-action="undo"]')?.disabled)
  assert.equal(await redo.isEnabled(), true)

  await refresh()
  await (await nextRequest(3)).abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-cut-action="undo"]')?.title === 'Undo availability unavailable')
  assert.equal(await undo.isDisabled(), true)
  assert.equal(await redo.isDisabled(), true)
  await reply(requests[1], true, true) // Older same-project read cannot re-enable either button.
  await page.waitForTimeout(30)
  assert.equal(await undo.isDisabled(), true)

  await refresh()
  await (await nextRequest(5)).fulfill({ status: 200, contentType: 'application/json', body: '{bad json' })
  await page.waitForFunction(() => document.querySelector('[data-cut-undo-tip]')?.textContent?.includes('availability unavailable'))
  await reply(requests[3], true, true)
  await refresh()
  await reply(await nextRequest(7), true, false)
  await page.waitForFunction(() => !document.querySelector('[data-cut-action="undo"]')?.disabled)
  assert.equal(await redo.isDisabled(), true)
  await reply(requests[5], false, true)
  await page.waitForTimeout(30)
  assert.equal(await undo.isEnabled(), true, 'older response cannot overwrite recovery')

  await refresh()
  const oldProject = await nextRequest(9)
  await page.locator('[data-test-switch]').click()
  assert.equal(await page.locator('[data-test-origin]').textContent(), 'origin-B')
  assert.equal(await undo.isDisabled(), true, 'same-name project switch clears old availability')
  await reply(await nextRequest(10), false, true)
  await page.waitForFunction(() => !document.querySelector('[data-cut-action="redo"]')?.disabled)
  await reply(oldProject, true, false)
  await page.waitForTimeout(30)
  assert.equal(await undo.isDisabled(), true, 'old origin response cannot change new origin')

  await redo.click()
  const beforeUnmount = requests.length
  await page.locator('[data-test-unmount]').click()
  await page.waitForTimeout(450)
  assert.equal(requests.length, beforeUnmount, 'delayed action refresh is canceled on unmount')
  assert.deepEqual(await page.evaluate(() => window.__reviewUnhandled), [])
  assert.deepEqual(errors, [])
})
