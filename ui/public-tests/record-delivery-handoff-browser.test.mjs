import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'
const PROJECT_A = `sha256:${'a'.repeat(64)}`

const doctor = {
  ready: true, start_allowed: true, cards: [],
  monitors: [{ id: 'display-A', index: 1, primary: true, name: 'Primary display' }],
  windows: [], window_capture_supported: true,
  camera: { supported: false, detail: 'Camera unavailable', devices: [] },
  pause: { supported: true, detail: 'Pause available', incompatible: [] },
}

test('raw timeline feedback belongs to one capture and project', { timeout: 120_000 }, async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-record-add-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'record-add-fixture', configureServer(vite) {
      vite.middlewares.use('/__record_add__', (_request, response) => {
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
  for (const scenario of ['same project', 'other project', 'new recording', 'recording start', 'failed add', 'pending success', 'pending failure', 'pending project return', 'recovery banner']) {
    await t.test(scenario, async () => {
      const page = await browser.newPage()
      const errors = []
      page.on('pageerror', error => errors.push(String(error)))
      let take = 0
      let heldImport = null
      let rejectImport = scenario === 'failed add'
      let holdImport = scenario.startsWith('pending')
      let recovered = false
      let imported = false
      let stateReads = 0
      const reply = (route, value) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(value) })
      await page.route('**/api/verb/**', route => {
        const name = new URL(route.request().url()).pathname.split('/').at(-1)
        if (name === 'media.import' && holdImport) { heldImport = route; return }
        if (name === 'media.import' && rejectImport) return reply(route, { ok: false, error: { code: 'io', message: 'test import refused' } })
        if (name === 'screen_record.start') take += 1
        if (name === 'media.import') imported = true
        if (name === 'screen_record.recover') recovered = true
        if (name === 'project.state') stateReads += 1
        const result = name === 'screen_record.doctor' ? doctor
          : name === 'screen_record.start' ? { capture_id: `take-${take}`, pause: { enabled: false } }
            : name === 'screen_record.stop' ? { capture_id: `take-${take}`, raw_path: '/fixture/reused-raw.mp4' }
              : name === 'media.import' ? { asset_id: 'raw-asset', job_id: 'raw-job' }
                : name === 'project.state' ? { assets: imported ? { 'raw-asset': { probe: { kind: 'video', duration_ms: 1000 } } } : {}, tracks: [{ id: 'v1', kind: 'video', clips: imported ? [{ id: 'raw-clip', asset: 'raw-asset' }] : [] }] }
                  : name === 'screen_record.pending' ? { recordings: scenario === 'recovery banner' ? [{ capture_id: 'recovered-take', situation: 'interrupted', can_recover: true, message: 'Saved recording can be recovered.' }] : [] }
                    : name === 'screen_record.recover' ? { clip_id: 'recovered-clip' }
                      : name === 'screen_record.status' ? { capture_id: `take-${take}`, ready: true, terminal: false, since_first_screen_frame_ms: 0 } : {}
        return reply(route, { ok: true, result })
      })
      await page.addInitScript(() => localStorage.setItem('shellx-cut.recording-preset.v1', JSON.stringify({
        schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
        fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
        keys: false, raw: true, studio: { background: 'gradient' },
      })))
      const phase = value => page.waitForFunction(expected => document.querySelector('[data-test-phase]')?.textContent === expected, value)
      const note = () => page.locator('[data-test-export-note]').textContent()
      const record = async () => {
        await page.locator('[data-cut-action="record-start"]').click()
        await phase('recording')
        await page.locator('[data-test-stop]').click()
        await phase('done')
        await page.locator('[data-cut-rec-result="saved"]').waitFor()
      }
      try {
        await page.goto(new URL(`/__record_add__${scenario === 'recovery banner' ? '?recovery' : ''}`, server.resolvedUrls.local[0]).href)
        await record()
        assert.equal(await note(), '')
        if (scenario === 'recovery banner') {
          await page.locator('[data-cut-action="recording-recover"]').click()
          await page.waitForFunction(() => document.querySelector('[data-cut-recording-recovery-notice]')?.textContent.includes('added to the end of the timeline'))
          assert.equal(recovered, true)
          assert.equal(await note(), '', 'recovered capture must not mark the current raw take as added')
        } else {
          await page.locator('[data-cut-action="record-add-raw"]').click()
          await page.waitForFunction(() => document.querySelector('[data-test-export-note]')?.textContent !== '')
          if (holdImport) {
            for (let i = 0; !heldImport && i < 100; i += 1) await page.waitForTimeout(20)
            assert.ok(heldImport, 'held import must have started')
            assert.equal(await note(), 'Adding to the timeline…')
          } else {
            const expected = rejectImport ? 'add failed: test import refused' : 'Added to the timeline'
            await page.waitForFunction(value => document.querySelector('[data-test-export-note]')?.textContent === value, expected)
            assert.equal(await page.locator('[data-cut-rec-export-note]').textContent(), expected)
          }
          if (scenario === 'pending project return') {
            await page.locator('[data-test-project-b]').click()
            assert.equal(await note(), '')
            await page.locator('[data-test-project-a]').click()
            assert.equal(await note(), '')
            holdImport = false
            imported = true
            await reply(heldImport, { ok: true, result: { asset_id: 'raw-asset', job_id: 'raw-job' } })
            await page.waitForTimeout(100)
            assert.equal(await note(), '', 'returning to the same take cannot revive an invalidated add')
            heldImport = null
          }
          if (scenario === 'recording start') {
            await page.keyboard.press('F9')
            await phase('recording')
            assert.equal(await note(), '', 'a new recording clears the previous confirmation before Stop')
            await page.locator('[data-test-stop]').click()
            await phase('done')
          } else if (scenario === 'other project') {
            await page.locator('[data-test-project-b]').click()
            assert.equal(await note(), '', 'project switch clears another project confirmation immediately')
            await page.locator('[data-test-reset]').click()
          } else await page.locator('[data-cut-action="record-new-take"]').click()
          if (scenario !== 'recording start') {
            await phase('idle')
            if (scenario === 'new recording') assert.equal(await note(), '', 'New recording clears the previous confirmation immediately')
            await record()
          }
          assert.equal(await note(), '', 'next capture starts without the previous add status, even when raw path is reused')
          if (heldImport) {
            const readsBefore = stateReads
            holdImport = false
            imported = scenario === 'pending success'
            await reply(heldImport, imported ? { ok: true, result: { asset_id: 'raw-asset', job_id: 'raw-job' } }
              : { ok: false, error: { code: 'io', message: 'late failure' } })
            if (imported) for (let i = 0; stateReads === readsBefore && i < 100; i += 1) await page.waitForTimeout(20)
            await page.waitForTimeout(100)
            assert.equal(await note(), '', 'late completion cannot label the next capture')
            assert.equal(await page.locator('[data-cut-rec-export-note]').count(), 0)
          }
          rejectImport = false
          await page.locator('[data-cut-action="record-add-raw"]').click()
          await page.waitForFunction(() => document.querySelector('[data-test-export-note]')?.textContent === 'Added to the timeline')
        }
        assert.deepEqual(errors, [])
      } finally { await page.close() }
    })
  }
})

test('recording delivery survives conditional Record and keyed project workspace changes', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-record-delivery-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), {
      name: 'delivery-dialog-stub', enforce: 'pre',
      resolveId(source) { if (source === '@tauri-apps/plugin-dialog') return '\0delivery-dialog-stub' },
      load(id) { if (id === '\0delivery-dialog-stub') return 'export async function save() { return "/fixture/output/raw-copy.mp4" }' },
    }, {
      name: 'record-delivery-fixture', configureServer(vite) {
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
      },
    }], server: { host: '127.0.0.1', port: 0 },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  const exports = []
  const copies = []
  const statuses = []
  const cancellations = []
  const authorizations = []
  let holdAuthorization = false
  let take = 0
  await page.route('**/api/verb/**', route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'screen_record.export') { exports.push(route); return }
    if (name === 'screen_record.copy_raw') { copies.push(route); return }
    if (name === 'jobs.status') { statuses.push(route); return }
    if (name === 'jobs.cancel') { cancellations.push(route); return }
    if (name === 'project.get_output_dir' && holdAuthorization) { authorizations.push(route); return }
    if (name === 'screen_record.start') take += 1
    const result = name === 'screen_record.doctor' ? doctor
      : name === 'screen_record.start' ? { capture_id: `take-${take}`, pause: { enabled: false } }
        : name === 'screen_record.stop' ? { capture_id: `take-${take}`, raw_path: '/fixture/raw.mp4', source: '/fixture/source.mp4', plan: '/fixture/plan.json' }
          : name === 'screen_record.studio_event' ? { last_event: route.request().postDataJSON().event }
          : name === 'screen_record.polish' ? { clip_id: `clip-${take}` }
            : name === 'screen_record.status' ? { capture_id: `take-${take}`, ready: true, terminal: false, since_first_screen_frame_ms: 0 } : {}
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })
  })
  const waitFor = async (items, count) => {
    for (let i = 0; i < 120; i += 1) {
      if (items.length >= count) return items[count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected request ${count}, saw ${items.length}; browser errors: ${errors.join('; ')}`)
  }
  const reply = (route, result) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(result) })
  const record = (id, kind, state) => ({ job_id: id, kind, state, progress: state === 'done' ? 1 : 0 })
  await page.addInitScript(() => localStorage.setItem('shellx-cut.recording-preset.v1', JSON.stringify({
    schema: 'shellx-cut/recording-preset/1', source: { kind: 'display', monitorId: 'display-A' },
    fps: 30, durationMs: null, startCountdownSeconds: 0, audio: false, systemAudio: false,
    keys: false, raw: false, studio: { background: 'gradient' },
  })))
  await page.goto(new URL('/__record_handoff__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-panel="record"]').waitFor()
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  await page.locator('[data-test-stop]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'done', undefined, { timeout: 7_000 }).catch(async () => {
    assert.fail(`Stop did not finish: ${JSON.stringify(await page.locator('[data-test-phase]').textContent())}, ${JSON.stringify(await page.locator('[data-cut-panel="record"]').textContent()).slice(0, 1500)}, errors=${errors.join('; ')}`)
  })
  assert.equal(await page.locator('[data-test-result-project]').textContent(), PROJECT_A)
  await page.locator('[data-cut-action="record-export"]').click()
  const pendingAdmission = await waitFor(exports, 1)
  assert.equal(pendingAdmission.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await page.locator('[data-test-edit]').click()
  await page.locator('[data-test-library]').click()
  await reply(pendingAdmission, { ok: true, result: { job_id: 'export-1' } })
  await page.waitForFunction(() => document.querySelector('[data-test-export-job]')?.textContent === 'export-1')
  const firstStatus = await waitFor(statuses, 1)
  assert.equal(firstStatus.request().postDataJSON().job_id, 'export-1')
  assert.equal(firstStatus.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await page.waitForTimeout(650)
  assert.equal(statuses.length, 1, 'delivery owner keeps one status read while Record is unmounted')
  await page.locator('[data-test-record]').click()
  assert.equal(await page.locator('[data-cut-action="record-export"]').isDisabled(), true)
  await reply(firstStatus, { ok: true, result: record('export-1', 'screen_record_export', 'done') })
  await page.waitForFunction(() => document.querySelector('[data-test-export-note]')?.textContent?.includes('Saved MP4'))
  assert.equal(exports.length, 1, 'Record remount did not submit another export')

  await page.evaluate(() => { window.__TAURI__ = {
    core: { invoke: async () => ({ applied: true }) }, event: { listen: async () => () => {} },
  } })
  await page.locator('[data-cut-action="record-save-raw-copy"]').click()
  const pendingCopy = await waitFor(copies, 1)
  assert.equal(pendingCopy.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await page.locator('[data-test-edit]').click()
  await reply(pendingCopy, { ok: true, result: { job_id: 'copy-1' } })
  await page.waitForFunction(() => document.querySelector('[data-test-copy-job]')?.textContent === 'copy-1')
  const copyStatus = await waitFor(statuses, 2)
  assert.equal(copyStatus.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await reply(copyStatus, { ok: true, result: record('copy-1', 'screen_record_copy_raw', 'running') })
  await page.locator('[data-test-project-a-renamed]').click()
  assert.equal(await page.locator('[data-test-owns-result]').textContent(), 'true', 'rename with the same origin keeps the finished take')
  assert.equal(await page.locator('[data-test-copy-job]').textContent(), 'copy-1', 'rename retains admitted raw copy')
  await page.locator('[data-test-record]').click()
  await page.locator('[data-cut-action="record-copy-cancel"]').click({ timeout: 5_000 }).catch(async () => {
    assert.fail(`Copy cancel absent: body=${(await page.locator('body').textContent()).slice(0, 1200)}, errors=${errors.join('; ')}`)
  })
  const cancel = await waitFor(cancellations, 1)
  assert.equal(cancel.request().postDataJSON().job_id, 'copy-1')
  assert.equal(cancel.request().postDataJSON().expected_origin_path_sha256, PROJECT_A)
  await cancel.abort('failed')
  await page.waitForFunction(() => document.querySelector('[data-test-copy-note]')?.textContent?.includes('cancellation not confirmed'))
  await reply(await waitFor(statuses, 3), { ok: true, result: record('copy-1', 'screen_record_copy_raw', 'done') })
  await page.waitForFunction(() => document.querySelector('[data-test-copy-note]')?.textContent === 'Copy saved.')
  assert.equal(copies.length, 1)

  await page.locator('[data-cut-action="record-export"]').click()
  await reply(await waitFor(exports, 2), { ok: true, result: { job_id: 'export-2' } })
  const oldProjectStatus = await waitFor(statuses, 4)
  await page.locator('[data-test-project-a]').click()
  assert.equal(await page.locator('[data-test-owns-result]').textContent(), 'true', 'name restoration does not detach the take')
  assert.equal(await page.locator('[data-test-export-job]').textContent(), 'export-2', 'admitted export survives a same-origin rename')
  await reply(oldProjectStatus, { ok: true, result: record('export-2', 'screen_record_export', 'running') })
  const renamedStatus = await waitFor(statuses, 5)
  assert.equal(renamedStatus.request().postDataJSON().job_id, 'export-2', 'same-origin rename continues polling the exact job')
  await page.locator('[data-test-project-b]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-owns-result]')?.textContent === 'false')
  assert.equal(await page.locator('[data-cut-rec-result-view]').getAttribute('data-cut-rec-result-view'), 'false')
  await reply(renamedStatus, { ok: true, result: record('export-2', 'screen_record_export', 'done') })
  assert.equal(await page.locator('[data-test-export-job]').textContent(), 'export-2', 'B historical matching ID cannot finish A job')
  await page.waitForTimeout(650)
  assert.equal(statuses.length, 5, 'B does not poll A job')
  await page.locator('[data-test-project-a]').click()
  await reply(await waitFor(statuses, 6), { ok: true, result: record('export-2', 'screen_record_export', 'done') })
  await page.waitForFunction(() => document.querySelector('[data-test-export-job]')?.textContent === '')

  await page.locator('[data-cut-action="record-export"]').click()
  await reply(await waitFor(exports, 3), { ok: true, result: { job_id: 'export-3' } })
  const oldTakeStatus = await waitFor(statuses, 7)
  await page.locator('[data-test-reset]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'idle')
  assert.equal(await page.locator('[data-test-export-job]').textContent(), 'export-3', 'reset retains admitted old take')
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  await page.locator('[data-test-stop]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'done')
  assert.equal(await page.locator('[data-test-export-job]').textContent(), 'export-3', 'new take with reused source paths still retains old admitted ID')
  assert.equal(await page.locator('[data-cut-action="record-export"]').isDisabled(), true)
  await reply(oldTakeStatus, { ok: true, result: record('export-3', 'screen_record_export', 'done') })
  await page.waitForFunction(() => document.querySelector('[data-test-export-job]')?.textContent === '')
  assert.match(await page.locator('[data-test-export-note]').textContent(), /Previous recording export finished/, 'old take completion is not new take success')

  await page.evaluate(() => localStorage.setItem('cut.outputDir', '/fixture/output'))
  holdAuthorization = true
  await page.locator('[data-cut-action="record-export"]').click()
  const heldAuthorization = await waitFor(authorizations, 1)
  await page.locator('[data-test-project-b]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-owns-result]')?.textContent === 'false')
  await page.locator('[data-test-project-a]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-owns-result]')?.textContent === 'true')
  holdAuthorization = false
  await reply(heldAuthorization, { ok: true, result: {} })
  await page.waitForFunction(() => document.querySelector('[data-test-export-note]')?.textContent?.includes('Recording changed before export started'))
  assert.equal(exports.length, 3, 'held authorization cannot revive an old intent after A to B to A')
  await page.locator('[data-cut-action="record-export"]').click()
  await reply(await waitFor(exports, 4), { ok: true, result: { job_id: 17 } })
  await page.waitForFunction(() => document.querySelector('[data-test-export-note]')?.textContent?.includes('Could not confirm that exporting started'))
  await page.waitForFunction(() => !document.querySelector('[data-cut-action="record-export"]')?.disabled)
  assert.equal(await page.locator('[data-test-export-job]').textContent(), '', 'numeric ID is not admitted')
  await page.locator('[data-cut-action="record-export"]').click()
  await reply(await waitFor(exports, 5), { ok: true, result: { job_id: ' padded ' } })
  await page.waitForFunction(() => document.querySelector('[data-cut-action="record-export"]') && !document.querySelector('[data-cut-action="record-export"]').disabled)
  assert.equal(await page.locator('[data-test-export-job]').textContent(), '', 'padded ID is not normalized or admitted')
  await page.locator('[data-cut-action="record-export"]').click()
  await reply(await waitFor(exports, 6), { ok: true, result: { job_id: 'export-6' } })
  assert.equal((await waitFor(statuses, 8)).request().postDataJSON().job_id, 'export-6')
  await reply(statuses[7], { ok: true, result: record('export-6', 'screen_record_export', 'done') })
  await page.waitForFunction(() => document.querySelector('[data-test-export-job]')?.textContent === '')
  await page.locator('[data-cut-action="record-save-raw-copy"]').click()
  await reply(await waitFor(copies, 2), { ok: true, result: { job_id: 'copy-2' } })
  const oldTakeCopyStatus = await waitFor(statuses, 9)
  await page.locator('[data-test-reset]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'idle')
  await page.locator('[data-cut-action="record-start"]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'recording')
  await page.locator('[data-test-stop]').click()
  await page.waitForFunction(() => document.querySelector('[data-test-phase]')?.textContent === 'done')
  assert.equal(await page.locator('[data-test-copy-job]').textContent(), 'copy-2', 'new take with reused raw path retains old copy')
  assert.equal(await page.locator('[data-cut-action="record-save-raw-copy"]').isDisabled(), true)
  await reply(oldTakeCopyStatus, { ok: true, result: record('copy-2', 'screen_record_copy_raw', 'done') })
  await page.waitForFunction(() => document.querySelector('[data-test-copy-job]')?.textContent === '')
  assert.match(await page.locator('[data-test-copy-note]').textContent(), /Previous recording copy finished/, 'old copy is not labelled as new take save')
  holdAuthorization = true
  await page.locator('[data-cut-action="record-export"]').click()
  const unmountedAuthorization = await waitFor(authorizations, 2)
  await page.locator('[data-test-unmount-delivery]').click()
  await page.locator('[data-test-delivery]').waitFor({ state: 'detached' })
  holdAuthorization = false
  await reply(unmountedAuthorization, { ok: true, result: {} })
  await page.waitForTimeout(100)
  assert.equal(exports.length, 6, 'authorization released after owner unmount cannot submit')
  assert.deepEqual(errors, [])
})
