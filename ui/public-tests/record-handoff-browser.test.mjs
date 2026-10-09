import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

const doctor = {
  ready: true, start_allowed: true, cards: [],
  monitors: [{ id: 'display-A', index: 1, primary: true, name: 'Primary display' }],
  windows: [], window_capture_supported: true,
  camera: { supported: false, detail: 'Camera unavailable', devices: [] },
  pause: { supported: true, detail: 'Pause available', incompatible: [] },
}

async function fixture(t, preset = null, initialDoctor = doctor, holdPolish = false) {
  let currentDoctor = initialDoctor
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-record-handoff-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'record-handoff-fixture', configureServer(vite) {
      vite.middlewares.use('/__record_handoff__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/record-handoff.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const errors = []
  const starts = []
  const pauseActions = []
  const pendingPolishes = []
  page.on('pageerror', error => errors.push(String(error)))
  if (preset) await page.addInitScript(value => localStorage.setItem('shellx-cut.recording-preset.v1', JSON.stringify(value)), preset)
  await page.route('**/api/verb/**', async route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'screen_record.start') starts.push(route.request().postDataJSON())
    if (name === 'screen_record.pause' || name === 'screen_record.resume') pauseActions.push(name)
    if (holdPolish && name === 'screen_record.polish') { pendingPolishes.push(route); return }
    const result = name === 'screen_record.doctor' ? currentDoctor
      : name === 'screen_record.start' ? { capture_id: 'capture-fixture', pause: { enabled: Boolean(starts.at(-1)?.pause) } }
        : holdPolish && name === 'screen_record.stop' ? { capture_id: 'capture-fixture', raw_path: '/fixture/raw.mp4', source: '/fixture/source.mp4', plan: '/fixture/plan.json' }
          : holdPolish && name === 'screen_record.studio_event' ? { last_event: route.request().postDataJSON().event }
        : name === 'screen_record.status' ? { capture_id: 'capture-fixture', terminal: false }
          : name === 'screen_record.pause' ? { action: 'pause', saved: true, state: 'paused', logical_media_time_ms: 200 }
            : name === 'screen_record.resume' ? { action: 'resume', saved: true, state: 'recording', logical_media_time_ms: 200 }
          : {}
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })
  })
  await page.goto(new URL('/__record_handoff__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-panel="record"]').waitFor({ timeout: 10_000 }).catch(() => {
    throw new Error(`Record did not mount: ${errors.join('; ')}`)
  })
  return { page, starts, pauseActions, pendingPolishes, errors, setDoctor: next => { currentDoctor = next } }
}

test('New recording waits for polish to finish before resetting the saved take', async t => {
  const preset = {
    schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
    fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
    keys: false, raw: false, studio: { background: 'gradient' },
  }
  const { page, starts, pendingPolishes, errors } = await fixture(t, preset, doctor, true)
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  await page.locator('[data-test-stop]').click()
  await page.locator('[data-cut-rec-result-view="true"]').waitFor()
  assert.equal(await page.locator('[data-test-phase]').textContent(), 'finalizing')
  const newTake = page.locator('[data-cut-action="record-new-take"]')
  assert.equal(await newTake.isDisabled(), true, 'New recording cannot discard a take while polish still owns it')
  await page.locator('[data-cut-rec-result="processing"]').waitFor()
  await page.getByRole('heading', { name: 'Finishing your recording', exact: true }).waitFor()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'finalizing')
  assert.equal(pendingPolishes.length, 1)
  await pendingPolishes[0].fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: { clip_id: 'clip-fixture' } }) })
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'done')
  assert.equal(await newTake.isEnabled(), true)
  await newTake.click()
  await page.locator('[data-cut-record-phase="idle"][data-cut-rec-result-view="false"]').waitFor()
  await page.locator('[data-cut-action="record-start"]').waitFor()
  assert.equal(starts.length, 1, 'resetting a result does not begin another capture')
  assert.deepEqual(errors, [])
})

test('Record draft survives Edit and reaches background F9 and Record remount', async t => {
  const { page, starts, errors } = await fixture(t)
  await page.locator('[data-cut-rec-mode="raw"]').click()
  await page.locator('[data-cut-rec-system-audio-toggle-input]').check()
  await page.locator('[data-cut-rec-audio-toggle-input]').uncheck()
  await page.locator('[data-cut-rec-settings-tab="quality"]').click()
  await page.locator('[data-cut-rec-fps="60"]').click()
  await page.locator('[data-test-edit]').click()
  await page.locator('[data-test-record]').click()
  await page.locator('[data-cut-rec-mode="raw"][aria-pressed="true"]').waitFor()
  assert.equal(await page.locator('[data-cut-rec-mode="raw"]').getAttribute('aria-pressed'), 'true')
  assert.equal(await page.locator('[data-cut-rec-system-audio-toggle-input]').isChecked(), true)
  assert.equal(await page.locator('[data-cut-rec-audio-toggle-input]').isChecked(), false)
  assert.equal(await page.locator('[data-cut-rec-fps="60"]').getAttribute('aria-pressed'), 'true')
  await page.locator('[data-test-edit]').click()
  await page.keyboard.press('F9')
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  assert.equal(starts.length, 1)
  assert.equal(starts[0].system_audio, true)
  assert.equal(starts[0].audio, false)
  assert.equal(starts[0].fps, 60)
  assert.equal(starts[0].monitor_id, 'display-A')
  assert.equal(starts[0].expected_project_identity.project_name, 'Record handoff fixture')
  assert.deepEqual(errors, [])
})

test('an incomplete Window choice stays selected and refuses F9 after Edit handoff', async t => {
  const { page, starts, errors } = await fixture(t)
  await page.locator('[data-cut-rec-source-kind-button="window"]').click()
  await page.locator('[data-cut-rec-window-no-selection]').waitFor()
  await page.locator('[data-test-edit]').click()
  await page.keyboard.press('F9')
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'error')
  assert.equal(starts.length, 0)
  await page.locator('[data-test-record]').click()
  await page.locator('[data-cut-rec-source-kind="window"]').waitFor()
  assert.equal(await page.locator('[data-cut-rec-source="window"]').inputValue(), '')
  assert.deepEqual(errors, [])
})

test('an unapplied frame rate draft refuses F9 instead of using the previous rate', async t => {
  const { page, starts, errors } = await fixture(t)
  await page.locator('[data-cut-rec-settings-tab="quality"]').click()
  await page.locator('[data-cut-action="record-fps-advanced-toggle"]').click()
  await page.locator('[data-cut-rec-fps-custom-input]').fill('999')
  await page.locator('[data-test-edit]').click()
  await page.keyboard.press('F9')
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'error')
  assert.equal(starts.length, 0)
  await page.locator('[data-test-record]').click()
  assert.equal(await page.locator('[data-cut-rec-fps-custom-input]').inputValue(), '999')
  assert.deepEqual(errors, [])
})

test('pause admission from Edit F9 provides live controls after Record mounts', async t => {
  const preset = {
    schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
    fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
    keys: false, raw: false, pause: { mode: 'enabled' }, studio: { background: 'gradient' },
  }
  const { page, starts, errors } = await fixture(t, preset)
  await page.locator('[data-test-edit]').click()
  await page.keyboard.press('F9')
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  assert.deepEqual(starts[0].pause, { mode: 'enabled' })
  await page.locator('[data-test-record]').click()
  await page.locator('[data-cut-action="record-pause-resume"]').waitFor()
  assert.equal(await page.locator('[data-cut-rec-live-pause-unavailable]').count(), 0)
  await page.locator('[data-cut-action="record-pause-resume"]').click()
  await page.locator('[data-cut-rec-pause-state="paused"]').waitFor()
  await page.locator('[data-cut-action="record-pause-resume"]').click()
  await page.locator('[data-cut-rec-pause-state="recording"]').waitFor()
  assert.deepEqual(errors, [])
})

test('a durable Pause acknowledgement survives Edit and Record remount', async t => {
  const preset = {
    schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
    fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
    keys: false, raw: false, pause: { mode: 'enabled' }, studio: { background: 'gradient' },
  }
  const { page, starts, pauseActions, errors } = await fixture(t, preset)
  await page.locator('[data-test-edit]').click()
  await page.keyboard.press('F9')
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  await page.locator('[data-test-record]').click()
  await page.locator('[data-cut-action="record-pause-resume"]').click()
  await page.locator('[data-cut-rec-pause-state="paused"]').waitFor()
  assert.deepEqual(pauseActions, ['screen_record.pause'])
  await page.locator('[data-test-edit]').click()
  await page.locator('[data-test-record]').click()
  await page.locator('[data-cut-rec-pause-state="paused"]').waitFor()
  assert.match(await page.locator('[data-cut-action="record-pause-resume"]').textContent(), /Resume/)
  await page.locator('[data-cut-action="record-pause-resume"]').click()
  await page.locator('[data-cut-rec-pause-state="recording"]').waitFor()
  assert.deepEqual(pauseActions, ['screen_record.pause', 'screen_record.resume'])
  assert.deepEqual(starts[0].pause, { mode: 'enabled' })
  assert.deepEqual(errors, [])
})

const linuxCamera = (devices) => ({
  ...doctor,
  camera: { supported: true, detail: 'Choose a camera at Start.', devices },
})

const cameraDevice = (id, state = 'permission_required') => ({ id, label: `Camera ${id}`, state, detail: state })

test('Linux Doctor can support camera with zero devices while screen-only Start omits camera_id', async t => {
  const { page, starts, errors } = await fixture(t, null, linuxCamera([]))
  await page.locator('[data-cut-studio-camera-available="true"]').waitFor()
  await page.locator('[data-cut-studio-camera-unavailable]').waitFor()
  assert.equal(await page.locator('[data-cut-rec-camera-toggle]').isDisabled(), true)
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  assert.equal(starts.length, 1)
  assert.equal(Object.hasOwn(starts[0], 'camera_id'), false)
  assert.deepEqual(errors, [])
})

test('Linux camera selection sends the current exact ID and refuses busy, denied, and stale choices', async t => {
  const oldId = 'linux-camera-old'
  const newId = 'linux-camera-current'
  const { page, starts, errors, setDoctor } = await fixture(t, null, linuxCamera([cameraDevice(oldId)]))
  await page.locator('[data-cut-rec-camera-toggle]').check()
  assert.equal(await page.locator('[data-cut-rec-camera-device]').inputValue(), oldId)
  for (const state of ['busy', 'permission_denied']) {
    setDoctor(linuxCamera([cameraDevice(oldId, state)]))
    await page.locator('[data-cut-action="record-source-refresh"]').click()
    await page.locator(`[data-cut-studio-camera-status="${state}"]`).waitFor()
    assert.equal(await page.locator('[data-cut-action="record-start"]').isDisabled(), true)
    assert.equal(starts.length, 0)
  }
  setDoctor(linuxCamera([cameraDevice(newId)]))
  await page.locator('[data-cut-action="record-source-refresh"]').click()
  await page.locator('[data-cut-studio-camera-status="missing"]').waitFor()
  assert.equal(await page.locator('[data-cut-rec-camera-device]').inputValue(), '')
  assert.equal(await page.locator('[data-cut-action="record-start"]').isDisabled(), true)
  await page.locator('[data-cut-rec-camera-device]').selectOption(newId)
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  assert.equal(starts.length, 1)
  assert.equal(starts[0].camera_id, newId)
  assert.deepEqual(errors, [])
})

for (const mode of ['raw', 'pause']) {
  test(`Linux camera is omitted when ${mode} capture is selected`, async t => {
    const { page, starts, errors } = await fixture(t, null, linuxCamera([cameraDevice('linux-camera-current')]))
    await page.locator('[data-cut-rec-camera-toggle]').check()
    if (mode === 'raw') await page.locator('[data-cut-rec-mode="raw"]').click()
    else {
      await page.locator('[data-cut-rec-settings-tab="timing"]').click()
      await page.locator('[data-cut-action="record-pause-enable"]').check()
    }
    await page.locator('[data-cut-action="record-start"]').click()
    await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
    assert.equal(starts.length, 1)
    assert.equal(Object.hasOwn(starts[0], 'camera_id'), false)
    assert.deepEqual(errors, [])
  })
}
