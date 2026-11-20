import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

async function openFixture(t, { reattach = null } = {}) {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-voiceover-doctor-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'voiceover-doctor-fixture', configureServer(vite) {
      vite.middlewares.use('/__voiceover_doctor__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/voiceover-doctor-lifecycle-browser.tsx"></script>`)
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
  await page.addInitScript(identity => {
    window.__unhandled = []
    window.addEventListener('unhandledrejection', event => window.__unhandled.push(String(event.reason)))
    if (identity) window.sessionStorage.setItem('shellx-cut/voiceover-reattach/1', JSON.stringify(identity))
  }, reattach)
  const doctors = []
  await page.route('**/api/verb/screen_record.doctor', route => { doctors.push(route) })
  const next = async count => {
    for (let i = 0; i < 160; i += 1) {
      if (doctors.length >= count) return doctors[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected Doctor request ${count}; page errors: ${errors.join('; ')}`)
  }
  await page.goto(new URL('/__voiceover_doctor__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-voiceover-track]').waitFor()
  return { page, errors, doctors, next }
}

// The Doctor read starts a native screen-capture probe on macOS. Every audio
// track header renders this control, so mounting it (opening a project,
// importing media) must not read Doctor; opening the voiceover controls may.
test('voiceover control reads Doctor only when the person opens it', async t => {
  const { page, errors, doctors, next } = await openFixture(t)
  await page.waitForTimeout(400)
  assert.equal(doctors.length, 0, 'mounting the voiceover control must not read Doctor')
  assert.equal(await page.locator('[data-cut-voiceover-control]').getAttribute('data-cut-voiceover-control'), 'unchecked')

  await page.evaluate(() => window.voiceoverDoctorUnmount())
  await page.locator('[data-test-unmounted]').waitFor({ state: 'attached' })
  await page.evaluate(() => window.voiceoverDoctorMount())
  await page.locator('[data-cut-voiceover-track]').waitFor()
  await page.waitForTimeout(200)
  assert.equal(doctors.length, 0, 'remounting the voiceover control must not read Doctor')

  await page.locator('[data-cut-action="voiceover-controls"]').click()
  await (await next(1)).abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-cut-voiceover-control]')?.getAttribute('data-cut-voiceover-control') === 'unavailable')
  await page.waitForTimeout(200)
  assert.equal(doctors.length, 1, 'opening the voiceover controls reads Doctor once')
  assert.deepEqual(errors, [])
})

// Voiceover records only the microphone: opening its controls reads only
// microphone readiness, starting a take reads nothing about the screen, and a
// missing microphone permission is named with where to turn it on.
test('voiceover reads only the microphone and names a missing permission', async t => {
  const { page, errors, doctors, next } = await openFixture(t)
  const refusal = {
    code: 'not_found',
    message: 'microphone permission is needed to record voiceover',
    cause: 'macOS has not given ShellX Cut access to the microphone',
    suggested_action: 'Turn on ShellX Cut in System Settings > Privacy & Security > Microphone, then press Record voiceover again',
  }
  let starts = 0
  await page.route('**/api/verb/voiceover.start', route => {
    starts += 1
    void route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: false, error: refusal }) })
  })
  const readiness = (startAllowed, blocked) => ({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: {
    mode: 'microphone', start_allowed: startAllowed, microphone_access: startAllowed ? 'granted' : 'denied',
    microphone_blocked: blocked, microphones: [], microphone_selection: { mode: 'system_default', status: 'ready' },
  } }) })

  await page.locator('[data-cut-action="voiceover-controls"]').click()
  const opened = await next(1)
  assert.deepEqual(opened.request().postDataJSON(), { mode: 'microphone' }, 'opening voiceover reads only the microphone')
  await opened.fulfill(readiness(true, null))
  await page.waitForFunction(() => document.querySelector('[data-cut-voiceover-control]')?.getAttribute('data-cut-voiceover-control') === 'ready')
  await page.locator('[data-cut-action="voiceover-record"]').click()
  await page.waitForFunction(() => document.querySelector('[data-cut-voiceover-status]')?.getAttribute('data-cut-voiceover-status') === 'error')
  const shown = await page.locator('[data-cut-voiceover-status]').textContent()
  assert.match(shown, /Microphone permission is needed to record voiceover/)
  assert.match(shown, /Privacy & Security > Microphone/)
  assert.equal(starts, 1)
  assert.equal(doctors.length, 1, 'starting a take reads no screen capture state')

  await page.evaluate(() => window.voiceoverDoctorUnmount())
  await page.locator('[data-test-unmounted]').waitFor({ state: 'attached' })
  await page.evaluate(() => { window.sessionStorage.clear(); window.voiceoverDoctorMount() })
  await page.locator('[data-cut-action="voiceover-controls"]').click()
  const denied = await next(2)
  assert.deepEqual(denied.request().postDataJSON(), { mode: 'microphone' })
  await denied.fulfill(readiness(false, { message: refusal.message, suggested_action: refusal.suggested_action }))
  await page.waitForFunction(() => document.querySelector('[data-cut-voiceover-status]')?.textContent?.includes('Privacy & Security > Microphone'))
  assert.equal(await page.locator('[data-cut-action="voiceover-record"]').isDisabled(), true)
  assert.deepEqual(errors, [])
})

test('a pending voiceover reattach still reads Doctor on mount', async t => {
  const { page, errors, next } = await openFixture(t, {
    reattach: { request_id: 'voiceover-reattach', expected_revision: 'revision-1', audio_track: 'audio-1', start_ms: 0, out_ms: null },
  })
  await (await next(1)).abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-cut-voiceover-control]')?.getAttribute('data-cut-voiceover-control') === 'unavailable')
  assert.deepEqual(errors, [])
})

test('voiceover Doctor rejection closes admission and remains settled after unmount', async t => {
  const { page, errors, next } = await openFixture(t)
  await page.locator('[data-cut-action="voiceover-controls"]').click()
  await (await next(1)).abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-cut-voiceover-control]')?.getAttribute('data-cut-voiceover-control') === 'unavailable')
  assert.equal(await page.locator('[data-cut-action="voiceover-record"]').isDisabled(), true)

  await page.evaluate(() => window.voiceoverDoctorUnmount())
  await page.locator('[data-test-unmounted]').waitFor({ state: 'attached' })
  await page.evaluate(() => window.voiceoverDoctorMount())
  await page.locator('[data-cut-voiceover-track]').waitFor()
  await page.locator('[data-cut-action="voiceover-controls"]').click()
  const pending = await next(2)
  await page.evaluate(() => window.voiceoverDoctorUnmount())
  await page.locator('[data-test-unmounted]').waitFor({ state: 'attached' })
  await pending.abort('failed')
  await page.waitForTimeout(100)
  assert.deepEqual(errors, [])
  assert.deepEqual(await page.evaluate(() => window.__unhandled), [])
})

// On Windows a privacy switch that is off, or a take that fails after it
// started, must say what is wrong and what to turn on, never a generic failure.
test('voiceover names the Windows microphone block and a failed take reason', async t => {
  const { page, errors, next } = await openFixture(t)
  const windowsBlock = {
    reason: 'windows_desktop_apps_microphone_off',
    message: 'Windows is blocking microphone access for desktop apps',
    suggested_action: 'Turn on Settings > Privacy & security > Microphone > Let desktop apps access your microphone, then try again',
  }
  const readiness = (startAllowed, blocked) => ({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: {
    mode: 'microphone', start_allowed: startAllowed, microphone_access: startAllowed ? 'granted' : 'denied',
    microphone_blocked: blocked, microphones: [], microphone_selection: { mode: 'system_default', status: 'ready' },
  } }) })

  await page.locator('[data-cut-action="voiceover-controls"]').click()
  await (await next(1)).fulfill(readiness(false, windowsBlock))
  await page.waitForFunction(() => document.querySelector('[data-cut-voiceover-status]')?.textContent?.includes('Let desktop apps access your microphone'))
  const blocked = await page.locator('[data-cut-voiceover-status]').textContent()
  assert.match(blocked, /Windows is blocking microphone access for desktop apps/)
  assert.equal(await page.locator('[data-cut-action="voiceover-record"]').isDisabled(), true)

  await page.evaluate(() => window.voiceoverDoctorUnmount())
  await page.locator('[data-test-unmounted]').waitFor({ state: 'attached' })
  await page.evaluate(() => { window.sessionStorage.clear(); window.voiceoverDoctorMount() })
  await page.locator('[data-cut-action="voiceover-controls"]').click()
  await (await next(2)).fulfill(readiness(true, null))
  await page.waitForFunction(() => document.querySelector('[data-cut-voiceover-control]')?.getAttribute('data-cut-voiceover-control') === 'ready')
  const failedTake = {
    schema: 'shellx-cut/voiceover-timeline/1', phase: 'finished', request_id: 'voiceover-failed', accepted_revision: 'revision-1',
    audio_track: 'audio-1', start_ms: 0, out_ms: null, terminal: 'failed', direct_monitoring: 'off',
    owner_claim: { session_id: 'AAAAAAAAAAAAAAAAAAAAAA', capability: 'c'.repeat(43) },
    failure: { ...windowsBlock, detail: 'mic config: Access is denied. (0x80070005)' },
  }
  const answer = route => { void route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: failedTake }) }) }
  await page.route('**/api/verb/voiceover.start', answer)
  await page.route('**/api/verb/voiceover.tick', answer)
  await page.locator('[data-cut-action="voiceover-record"]').click()
  await page.waitForFunction(() => document.querySelector('[data-cut-voiceover-status]')?.getAttribute('data-cut-voiceover-status') === 'error')
  const failed = await page.locator('[data-cut-voiceover-status]').textContent()
  assert.match(failed, /^Voiceover failed\. Windows is blocking microphone access for desktop apps/)
  assert.match(failed, /Let desktop apps access your microphone/)
  assert.doesNotMatch(failed, /private recovery state/)
  assert.deepEqual(errors, [])
})
