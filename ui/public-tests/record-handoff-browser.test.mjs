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

// `override(name)` may return a whole engine reply for one verb (for example an
// engine refusal) instead of the default fixture result.
async function fixture(t, preset = null, initialDoctor = doctor, holdPolish = false, override = null, search = '') {
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
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const errors = []
  const starts = []
  const pauseActions = []
  let pauseState = 'recording'
  const pendingPolishes = []
  page.on('pageerror', error => errors.push(String(error)))
  if (preset) await page.addInitScript(value => localStorage.setItem('shellx-cut.recording-preset.v1', JSON.stringify(value)), preset)
  await page.route('**/api/verb/**', async route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'screen_record.start') starts.push(route.request().postDataJSON())
    if (name === 'screen_record.pause' || name === 'screen_record.resume') pauseActions.push(name)
    if (name === 'screen_record.pause') pauseState = 'paused'
    if (name === 'screen_record.resume') pauseState = 'recording'
    if (holdPolish && name === 'screen_record.polish') { pendingPolishes.push(route); return }
    const custom = override?.(name)
    if (custom) { await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(custom) }); return }
    const result = name === 'screen_record.doctor' ? currentDoctor
      : name === 'screen_record.start' ? { capture_id: 'capture-fixture', pause: { enabled: Boolean(starts.at(-1)?.pause) } }
        : holdPolish && name === 'screen_record.stop' ? { capture_id: 'capture-fixture', raw_path: '/fixture/raw.mp4', source: '/fixture/source.mp4', plan: '/fixture/plan.json' }
          : holdPolish && name === 'screen_record.studio_event' ? { last_event: route.request().postDataJSON().event }
        : name === 'screen_record.status' ? { capture_id: 'capture-fixture', ready: true, terminal: false, since_first_screen_frame_ms: 0,
          pause: { enabled: Boolean(starts.at(-1)?.pause), state: pauseState, reason: 'Pause was not enabled for this take.' } }
          : name === 'screen_record.pause' ? { action: 'pause', saved: true, state: 'paused', logical_media_time_ms: 200 }
            : name === 'screen_record.resume' ? { action: 'resume', saved: true, state: 'recording', logical_media_time_ms: 200 }
          : {}
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })
  })
  await page.goto(new URL(`/__record_handoff__${search}`, server.resolvedUrls.local[0]).href)
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

test('a completed unsupported Doctor clears a saved Pause choice after the pending probe', async t => {
  const preset = {
    schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
    fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
    keys: false, raw: false, pause: { mode: 'enabled' }, studio: { background: 'gradient' },
  }
  const unavailable = { ...doctor, pause: { supported: false, detail: 'Pause intentionally unavailable', incompatible: [] } }
  const { page, starts, errors } = await fixture(t, preset, unavailable)
  await page.getByText('Pause intentionally unavailable', { exact: true }).waitFor({ state: 'attached' })
  assert.equal(await page.locator('[data-cut-action="record-pause-enable"]').isChecked(), false)
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  assert.equal(starts.length, 1)
  assert.equal(Object.hasOwn(starts[0], 'pause'), false)
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

for (const initialState of ['recording', 'paused']) {
  test(`an externally started ${initialState} take reads pause from engine status after reload`, async t => {
    let state = initialState
    const { page, starts, errors } = await fixture(t, null, doctor, false, name => {
      if (name === 'screen_record.active') return { ok: true, result: {
        active: true, kind: 'recording', capture_id: 'capture-external', ready: true, since_first_screen_frame_ms: 2500,
      } }
      if (name === 'screen_record.status') return { ok: true, result: {
        capture_id: 'capture-external', ready: true, terminal: false,
        pause: { enabled: true, state, reason: null },
      } }
      if (name === 'screen_record.pause' || name === 'screen_record.resume') {
        state = name === 'screen_record.pause' ? 'paused' : 'recording'
        return { ok: true, result: { capture_id: 'capture-external', action: state === 'paused' ? 'pause' : 'resume',
          saved: true, state, logical_media_time_ms: 200 } }
      }
    })
    const assertState = async () => {
      await page.locator(`[data-cut-rec-pause-state="${state}"]`).waitFor({ timeout: 5000 })
      assert.match(await page.locator('[data-cut-action="record-pause-resume"]').textContent(), state === 'paused' ? /Resume/ : /Pause/)
      assert.equal(await page.locator('[data-cut-rec-live-pause-unavailable]').count(), 0)
    }
    await assertState()
    await page.reload()
    await assertState()
    if (process.env.CUT_PAUSE_SHOT_DIR) await page.locator('[data-cut-rec-live-controls]').screenshot({
      path: `${process.env.CUT_PAUSE_SHOT_DIR}/external-${initialState}-after-reload.png`,
    })
    await page.locator('[data-cut-action="record-pause-resume"]').click()
    await assertState()
    // A control issued by another client must also reach the page through status.
    state = initialState
    await assertState()
    assert.equal(starts.length, 0, 'this page never started the running take')
    assert.deepEqual(errors, [])
  })
}

test('an ordinary external take displays its exact engine pause reason on an unsupported platform', async t => {
  const reason = 'Pause and resume are unavailable on this platform. Stop remains available to finalize an ordinary recording.'
  const unsupported = { ...doctor, pause: { supported: false, detail: 'Setup capability differs from this take.', incompatible: [] } }
  const { page, starts, errors } = await fixture(t, null, unsupported, false, name => {
    if (name === 'screen_record.active') return { ok: true, result: {
      active: true, kind: 'recording', capture_id: 'capture-ordinary', ready: true,
    } }
    if (name === 'screen_record.status') return { ok: true, result: {
      capture_id: 'capture-ordinary', ready: true, terminal: false, pause: { enabled: false, state: 'unavailable', reason },
    } }
  })
  const unavailable = page.locator('[data-cut-rec-live-pause-unavailable]')
  await page.waitForFunction(text => document.querySelector('[data-cut-rec-live-pause-unavailable]')?.textContent.includes(text), reason, { timeout: 5000 })
  assert.equal(await unavailable.locator('small').textContent(), reason)
  assert.equal(await page.locator('[data-cut-action="record-pause-resume"]').count(), 0)
  assert.equal(await page.locator('[data-cut-action="record-stop"]').isEnabled(), true)
  if (process.env.CUT_PAUSE_SHOT_DIR) await unavailable.screenshot({ path: `${process.env.CUT_PAUSE_SHOT_DIR}/unsupported-reason.png` })
  assert.equal(starts.length, 0)
  assert.deepEqual(errors, [])
})

test('running status overrides this page\'s Start form and recovers after reconnect', async t => {
  let connected = false
  let unreachable = false
  const { page, errors } = await fixture(t, null, doctor, false, name => {
    if (name === 'screen_record.status' && unreachable) return { ok: false, error: { code: 'unavailable', message: 'Connection interrupted.' } }
    if (name === 'screen_record.status' && connected) return { ok: true, result: {
      capture_id: 'capture-fixture', ready: true, terminal: false, pause: { enabled: true, state: 'paused', reason: null },
    } }
  })
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  unreachable = true
  await page.waitForResponse(async response => response.url().includes('screen_record.status') && !(await response.json()).ok)
  unreachable = false
  connected = true
  await page.locator('[data-cut-rec-pause-state="paused"]').waitFor({ timeout: 5000 })
  assert.match(await page.locator('[data-cut-action="record-pause-resume"]').textContent(), /Resume/)
  assert.deepEqual(errors, [])
})

for (const separate of [true, false]) {
  test(`own camera track polish ${separate ? 'separate' : 'baked'}`, async t => {
    const preset = {
      schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
      fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
      keys: false, raw: false, cameraId: 'cam', cameraSeparate: separate,
      studio: { background: 'gradient', camera: { x: 0.7, y: 0.7, size: 0.22, shape: 'circle' } },
    }
    const { page, pendingPolishes } = await fixture(t, preset, linuxCamera([cameraDevice('cam', 'ready')]), true)
    await page.setViewportSize({ width: 1600, height: 1100 })
    const toggle = page.locator('[data-cut-rec-camera-separate]')
    await toggle.waitFor()
    assert.equal(await toggle.isChecked(), separate)
    await page.locator('[data-cut-rec-camera-toggle]').uncheck()
    assert.equal(await toggle.isDisabled(), true)
    await page.getByText('Turn on Camera to edit it later.', { exact: true }).waitFor()
    await page.locator('[data-cut-rec-camera-toggle]').check()
    assert.equal(await toggle.isEnabled(), true)
    if (process.env.CUT_CAMERA_SHOT_DIR && separate) {
      for (const theme of ['dark', 'light']) {
        await page.evaluate(theme => document.documentElement.setAttribute('data-theme', theme), theme)
        await page.locator('.rec-studio-controls__camera').screenshot({ path: `${process.env.CUT_CAMERA_SHOT_DIR}/camera-${theme}.png` })
      }
    }
    await page.locator('[data-cut-action="record-start"]').click()
    await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
    await page.locator('[data-test-stop]').click()
    await page.waitForFunction(() => document.querySelector('[data-cut-rec-result-view]'))
    for (let i = 0; i < 100 && !pendingPolishes.length; i++) await page.waitForTimeout(25)
    assert.equal(pendingPolishes[0].request().postDataJSON().camera, separate ? 'separate' : 'baked')
    await pendingPolishes[0].fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: { clip_id: 'clip-fixture' } }) })
    await page.getByRole('button', { name: 'Export as recorded', exact: true }).waitFor()
    await page.getByText('Edits you make later in Edit are not in this file. Export from Edit to include them.', { exact: true }).waitFor()
    if (process.env.CUT_CAMERA_SHOT_DIR && separate) {
      for (const theme of ['dark', 'light']) {
        await page.evaluate(theme => document.documentElement.setAttribute('data-theme', theme), theme)
        await page.locator('[data-cut-rec-result-view]').screenshot({ path: `${process.env.CUT_CAMERA_SHOT_DIR}/result-${theme}.png` })
      }
    }
  })
}

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

const quickPreset = {
  schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
  fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
  keys: false, raw: true, studio: { background: 'gradient' },
}

test('a refused Start shows the way forward the engine gives', async t => {
  const refusal = {
    ok: false,
    error: {
      code: 'not_found',
      message: 'the selected window is no longer available',
      cause: 'window windows-hwnd-v1:00000000deadbeef:1 is not in the current screen_record.doctor window list',
      suggested_action: 'reopen the source picker and choose the window again',
    },
  }
  const { page, errors } = await fixture(t, quickPreset, doctor, false, name => (name === 'screen_record.start' ? refusal : null))
  await page.locator('[data-cut-action="record-start"]').click()
  const shown = page.locator('[data-cut-rec-error]')
  await shown.waitFor()
  assert.equal(
    await shown.textContent(),
    'Could not start recording: The selected window is no longer available. Reopen the source picker and choose the window again.',
  )
  assert.deepEqual(errors, [])
})

test('an early Stop says nothing was recorded and how to record, with no recovery hint', async t => {
  let stopped = false
  const nothingRecorded = {
    ok: false,
    error: {
      code: 'capture',
      message: 'nothing was recorded',
      cause: 'Stop came before the first screen frame was captured, so there is no take to save',
      suggested_action: 'start the recording again and wait until the timer is running before you stop',
    },
  }
  const { page, errors } = await fixture(t, quickPreset, doctor, false, name => {
    if (name === 'screen_record.stop') { stopped = true; return nothingRecorded }
    if (name === 'screen_record.status' && stopped) {
      return { ok: false, error: { code: 'not_found', message: 'screen recording is not active in this cutd process' } }
    }
    return null
  })
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  await page.locator('[data-test-stop]').click()
  const shown = page.locator('[data-cut-rec-error]')
  await shown.waitFor()
  const text = await shown.textContent()
  assert.equal(
    text,
    'Nothing was recorded: Stop came before the first screen frame was captured. Start the recording again and wait until the timer is running before you stop.',
  )
  assert.doesNotMatch(text, /Recovery|recover/i, 'there is nothing to recover')
  assert.deepEqual(errors, [])
})

// The environment report's operating system reaches Record through the fixture's `os` query value. Only
// macOS asks for Audio Capture permission, so only macOS may read that sentence under Test system audio.
for (const [os, search] of [['linux', '?os=linux'], ['windows', '?os=windows'], ['not yet known', '']]) {
  test(`the system audio hint has no macOS sentence when the system is ${os}`, async t => {
    const { page, errors } = await fixture(t, null, doctor, false, null, search)
    const hint = page.locator('[data-cut-rec-system-audio-probe] .rec__audio-probe-status')
    await hint.waitFor()
    assert.equal(await hint.textContent(), 'Play a short sound, then test.')
    assert.deepEqual(errors, [])
  })
}

test('the system audio hint keeps the Audio Capture permission sentence on macOS', async t => {
  const { page, errors } = await fixture(t, null, doctor, false, null, '?os=macos')
  const hint = page.locator('[data-cut-rec-system-audio-probe] .rec__audio-probe-status')
  await hint.waitFor()
  assert.equal(await hint.textContent(), 'Play a short sound, then test. macOS may ask for Audio Capture permission.')
  assert.deepEqual(errors, [])
})

// The readiness card says what it will do before Check: only macOS may ask for screen access, so only macOS
// reads that clause. Windows, Linux and a system that is not known yet read the sentence without it.
{
  const notChecked = { ...doctor, ready: false, start_allowed: true, screen_check: 'not_checked' }
  const base = 'ShellX Cut checks it when you start recording. Check now to see the result first'
  for (const [os, search] of [['linux', '?os=linux'], ['windows', '?os=windows'], ['not yet known', '']]) {
    test(`the screen capture not-checked sentence has no macOS clause when the system is ${os}`, async t => {
      const { page, errors } = await fixture(t, null, notChecked, false, null, search)
      const detail = page.locator('[data-cut-rec-readiness-detail]')
      await page.locator('[data-cut-rec-readiness-label]').getByText('Screen capture not checked yet').waitFor()
      assert.equal(await detail.innerText(), `${base}.`)
      assert.deepEqual(errors, [])
    })
  }
  test('the screen capture not-checked sentence keeps the macOS clause on macOS', async t => {
    const { page, errors } = await fixture(t, null, notChecked, false, null, '?os=macos')
    await page.locator('[data-cut-rec-readiness-label]').getByText('Screen capture not checked yet').waitFor()
    assert.equal(await page.locator('[data-cut-rec-readiness-detail]').innerText(), `${base}; macOS may ask for screen access.`)
    assert.deepEqual(errors, [])
  })
}

// The live box under the preview while a take is paused. The recorder's recorded time stands still during a pause
// (its acknowledgement carries `logical_media_time_ms`); a wall-clock count kept running and, after Resume, stayed ahead.
test('a paused take: the live box says Paused with a pause mark and its timer stands still; Resume counts on from the recorded time', async t => {
  const preset = {
    schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
    fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
    keys: false, raw: false, pause: { mode: 'enabled' }, studio: { background: 'gradient' },
  }
  const RECORDED_MS = 47_021
  const { page, errors } = await fixture(t, preset, doctor, false, name => {
    if (name === 'screen_record.pause') return { ok: true, result: { action: 'pause', saved: true, state: 'paused', logical_media_time_ms: RECORDED_MS } }
    if (name === 'screen_record.resume') return { ok: true, result: { action: 'resume', saved: true, state: 'recording', logical_media_time_ms: RECORDED_MS } }
    return null
  })
  const hud = page.locator('[data-cut-rec-hud]')
  const timer = page.locator('[data-cut-rec-elapsed]')
  const seconds = async () => { const [m, s] = (await timer.textContent()).split(':').map(Number); return m * 60 + s }
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-elapsed]')?.textContent !== '0:00')
  assert.match(await hud.locator('small').textContent(), /^Recording · /)
  assert.equal(await hud.locator('.rec__hud-dot').count(), 1, 'a running take shows the recording dot')
  assert.equal(await hud.locator('.rec__hud-pause').count(), 0)

  await page.locator('[data-cut-action="record-pause-resume"]').click()
  await page.locator('[data-cut-rec-pause-state="paused"]').waitFor()
  assert.match(await hud.locator('small').textContent(), /^Paused · /, 'the word says it')
  assert.equal(await hud.locator('.rec__hud-dot').count(), 0, 'no red recording dot while paused')
  const mark = hud.locator('.rec__hud-pause')
  await mark.waitFor()
  const markLook = await mark.evaluate(element => {
    const style = getComputedStyle(element)
    const [r, g, b] = style.borderLeftColor.match(/\d+(\.\d+)?/g).map(Number)
    return { box: element.getBoundingClientRect().width, borderLeft: parseFloat(style.borderLeftWidth), r, g, b, animation: style.animationName }
  })
  assert.ok(markLook.box >= 8 && markLook.borderLeft >= 2, `the pause mark is visible ${JSON.stringify(markLook)}`)
  assert.ok(markLook.r - Math.max(markLook.g, markLook.b) < 60, `the pause mark is not red ${JSON.stringify(markLook)}`)
  assert.equal(markLook.animation, 'none', 'the pause mark does not blink')
  const boxColours = await hud.evaluate(element => { const s = getComputedStyle(element); return [s.backgroundColor, s.borderTopColor, s.color] })
  for (const colour of boxColours) {
    const [r, g, b] = colour.match(/\d+(\.\d+)?/g).map(Number)
    assert.ok(r - Math.max(g, b) < 60, `the paused box is not red-toned: ${colour}`)
  }
  assert.equal(await hud.getAttribute('data-cut-rec-hud') !== null, true, 'machine-readable attribute kept')
  assert.equal(await timer.getAttribute('data-cut-rec-elapsed') !== null, true, 'machine-readable attribute kept')

  // The time shown is the recorder's recorded time and is the same at two instants during the pause.
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-elapsed]')?.textContent === '0:47', null, { timeout: 3000 })
  const firstLook = await timer.textContent()
  await page.waitForTimeout(2600)
  const secondLook = await timer.textContent()
  assert.equal(firstLook, '0:47')
  assert.equal(secondLook, firstLook, 'the timer does not run while paused')
  assert.match(await page.locator('[data-cut-rec-pause-status]').textContent(), /^Paused at 0:47\./, 'the line and the timer say the same time')

  await page.locator('[data-cut-action="record-pause-resume"]').click()
  await page.locator('[data-cut-rec-pause-state="recording"]').waitFor()
  assert.match(await hud.locator('small').textContent(), /^Recording · /)
  assert.equal(await hud.locator('.rec__hud-dot').count(), 1)
  await page.waitForTimeout(2300)
  const after = await seconds()
  assert.ok(after >= 48 && after <= 50, `counts on from 0:47 after Resume, not from the wall-clock start: ${await timer.textContent()}`)
  assert.deepEqual(errors, [])
})

// With a length limit the HUD shows "Ns left" in place of the timer. The recorder stops a take at its start plus
// the limit on the wall clock and a pause does not move that deadline, so the seconds left must run on through a
// pause (they follow the recorder, not the recorded time that stands still).
test('a paused take with a length limit: the seconds left keep counting down through the pause', async t => {
  const preset = {
    schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
    fps: 30, durationMs: 30_000, startCountdownSeconds: 0, audio: false, systemAudio: false,
    keys: false, raw: false, pause: { mode: 'enabled' }, studio: { background: 'gradient' },
  }
  const { page, starts, errors } = await fixture(t, preset, doctor, false, name => {
    if (name === 'screen_record.pause') return { ok: true, result: { action: 'pause', saved: true, state: 'paused', logical_media_time_ms: 1_500 } }
    if (name === 'screen_record.resume') return { ok: true, result: { action: 'resume', saved: true, state: 'recording', logical_media_time_ms: 1_500 } }
    return null
  })
  const hud = page.locator('[data-cut-rec-hud]')
  const left = async () => {
    const text = await page.locator('[data-cut-rec-elapsed]').textContent()
    assert.match(text, /^\d+s left$/)
    return Number.parseInt(text, 10)
  }
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  assert.equal(starts[0].duration_ms, 30_000, 'the limit goes to the recorder, which enforces it')
  await page.waitForTimeout(1300)
  const beforePause = await left()
  await page.locator('[data-cut-action="record-pause-resume"]').click()
  await page.locator('[data-cut-rec-pause-state="paused"]').waitFor()
  assert.match(await hud.locator('small').textContent(), /^Paused · /)
  await page.waitForTimeout(3300)
  const duringPause = await left()
  assert.ok(beforePause - duringPause >= 3, `the limit keeps running while paused: ${beforePause}s left before, ${duringPause}s left 3.3 s into the pause`)
  await page.locator('[data-cut-action="record-pause-resume"]').click()
  await page.locator('[data-cut-rec-pause-state="recording"]').waitFor()
  await page.waitForTimeout(1300)
  const afterResume = await left()
  assert.ok(duringPause - afterResume >= 1, `and after Resume: ${duringPause}s -> ${afterResume}s`)
  assert.deepEqual(errors, [])
})

test('a paused take says the time in plain words and the preview says it is paused', async t => {
  const preset = {
    schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
    fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
    keys: false, raw: false, pause: { mode: 'enabled' }, studio: { background: 'gradient' },
  }
  const { page, errors } = await fixture(t, preset, doctor, false, name => {
    if (name === 'screen_record.live_frame') return { ok: true, result: {
      capture_id: 'capture-fixture', state: 'awaiting_frame', reason: null, generation: 0, frame: null, frame_age_ms: null,
    } }
    if (name === 'screen_record.pause') return { ok: true, result: { action: 'pause', saved: true, state: 'paused', logical_media_time_ms: 47_021 } }
    return null
  })
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  const preview = page.locator('[data-cut-rec-preview-unavailable]')
  const status = page.locator('[data-cut-rec-pause-status]')
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-preview-unavailable]')?.textContent?.includes('Waiting for the first recorded frame'))
  assert.equal(await page.locator('[data-cut-rec-preview-phase]').textContent(), 'REC')

  await page.locator('[data-cut-action="record-pause-resume"]').click()
  await page.locator('[data-cut-rec-pause-state="paused"]').waitFor()
  const pausedLine = 'Paused at 0:47. Resume to continue recording.'
  assert.equal(await status.textContent(), pausedLine)
  // Two status polls pass (one per second); the line that says when stays.
  await page.waitForTimeout(2300)
  const later = await status.textContent()
  assert.equal(later, pausedLine)
  assert.doesNotMatch(later, /journal|\bms\b/)
  assert.equal(await page.locator('[data-cut-rec-preview-unavailable] strong').textContent(), 'Recording paused')
  assert.equal(await page.locator('[data-cut-rec-preview-unavailable] span').textContent(), 'Resume to continue recording.')
  assert.doesNotMatch(await preview.textContent(), /Waiting|first recorded frame/)
  assert.equal(await page.locator('[data-cut-rec-preview-phase]').textContent(), 'Paused')
  assert.equal(await page.locator('[data-cut-rec-preview-phase]').getAttribute('data-cut-rec-preview-phase'), 'recording', 'the machine-readable phase is unchanged')
  assert.equal(await page.locator('[data-cut-rec-native-preview-state]').getAttribute('data-cut-rec-native-preview-state'), 'awaiting_frame', 'the engine preview state is unchanged')
  if (process.env.CUT_PAUSE_SHOT_DIR) await page.locator('[data-cut-studio-preview]').screenshot({ path: `${process.env.CUT_PAUSE_SHOT_DIR}/paused-preview.png` })
  if (process.env.CUT_PAUSE_SHOT_DIR) await page.locator('[data-cut-rec-live-controls]').screenshot({ path: `${process.env.CUT_PAUSE_SHOT_DIR}/paused-live-controls.png` })

  await page.locator('[data-cut-action="record-pause-resume"]').click()
  await page.locator('[data-cut-rec-pause-state="recording"]').waitFor()
  await page.waitForFunction(() => document.querySelector('[data-cut-rec-preview-unavailable]')?.textContent?.includes('Waiting for the first recorded frame'))
  assert.equal(await page.locator('[data-cut-rec-preview-phase]').textContent(), 'REC')
  assert.deepEqual(errors, [])
})
