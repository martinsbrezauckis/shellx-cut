import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

async function openFixture(t, readiness) {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-record-mic-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'record-mic-fixture', configureServer(vite) {
      vite.middlewares.use('/__record_mic__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/record-mic-input-browser.tsx"></script>`)
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
  const requests = []
  await page.route('**/api/verb/screen_record.doctor', route => {
    requests.push(route.request().postDataJSON())
    void route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: readiness(route.request().postDataJSON()) }) })
  })
  await page.goto(new URL('/__record_mic__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-rec-mic-status]').waitFor()
  return { page, errors, requests }
}

const windowsBlock = {
  reason: 'windows_desktop_apps_microphone_off',
  message: 'Windows is blocking microphone access for desktop apps',
  suggested_action: 'Turn on Settings > Privacy & security > Microphone > Let desktop apps access your microphone, then try again',
}

const base = { mode: 'microphone', microphones: [], microphone_selection: { mode: 'system_default', status: 'ready' } }

// A Windows privacy switch that is off is shown as soon as the microphone
// control is on, before the person presses Test.
test('Record microphone control names a Windows block before any test', async t => {
  const { page, errors, requests } = await openFixture(t, () => ({
    ...base, start_allowed: false, microphone_access: 'denied', microphone_blocked: windowsBlock,
  }))
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-mic-status]')?.getAttribute('data-cut-rec-mic-status') === 'blocked')
  const shown = await page.locator('[data-cut-rec-mic-status]').textContent()
  assert.match(shown, /Windows is blocking microphone access for desktop apps/)
  assert.match(shown, /Let desktop apps access your microphone/)
  assert.equal(await page.locator('[data-cut-rec-mic-status]').getAttribute('data-cut-rec-mic-blocked'), 'windows_desktop_apps_microphone_off')
  assert.deepEqual(requests, [{ mode: 'microphone' }], 'no test ran and no screen read happened')
  assert.deepEqual(errors, [])
})

test('Record microphone test that cannot open the device says why', async t => {
  const { page, errors } = await openFixture(t, request => request.warm_mic
    ? { ...base, start_allowed: true, microphone_access: 'granted', microphone_blocked: null, mic_warm: {
      live: false, supported: true, reason: 'microphone_in_use',
      message: 'The microphone is in use by another recording or app. Stop the other recording or close the app that uses the microphone, then try again',
    } }
    : { ...base, start_allowed: true, microphone_access: 'granted', microphone_blocked: null })
  await page.locator('[data-cut-action="record-mic-test"]').click()
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-mic-status]')?.textContent?.includes('in use by another recording'))
  const shown = await page.locator('[data-cut-rec-mic-status]').textContent()
  assert.match(shown, /Stop the other recording/)
  assert.doesNotMatch(shown, /No microphone signal arrived/)
  assert.deepEqual(errors, [])
})
