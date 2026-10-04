// Manual UI asset contract — public Cut imagery is a literal capture of the
// production-built UI, never an invented layout, Vite page, or mock transport.
//
// Default mode is a hermetic docs/asset check. With --write-current this file
// starts the current-source release cutd in a disposable loopback workspace,
// uses Cut's embedded self-authored First edit sample for a generic project,
// and replaces the PNGs only after real UI/engine readiness assertions pass.
// Build first: npm --prefix ui run build && cargo build --release -p server \
//   --bin cutd --manifest-path app/Cargo.toml

import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync } from 'node:fs'
import { copyFile, mkdtemp, rename, rm } from 'node:fs/promises'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { createRequire } from 'node:module'
import { basename, dirname, join, resolve } from 'node:path'
const root = resolve(new URL('../..', import.meta.url).pathname)
const manual = resolve(root, 'docs/public/site/manual/cut/index.html')
const editorAsset = resolve(root, 'docs/public/site/manual/assets/cut/cut-main-editor-current.png')
const recordAsset = resolve(root, 'docs/public/site/manual/assets/cut/cut-recording-studio-current.png')
const appPackage = resolve(root, 'ui/package.json')
const desktopConfig = resolve(root, 'app/desktop/src-tauri/tauri.conf.json')
const uiDist = resolve(root, 'ui/dist')
const cutd = resolve(root, 'app/target/release/cutd')
const writeCurrent = process.argv.includes('--write-current')
const captureName = 'Product walkthrough'

function text(path) {
  return readFileSync(path, 'utf8')
}

function pngSize(path) {
  const bytes = readFileSync(path)
  assert.deepEqual(
    [...bytes.subarray(0, 8)],
    [137, 80, 78, 71, 13, 10, 26, 10],
    `${path} is a PNG`,
  )
  assert.equal(bytes.toString('ascii', 12, 16), 'IHDR', `${path} has an IHDR header`)
  return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20), bytes: bytes.length }
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

function wait(ms) {
  return new Promise((resolveWait) => setTimeout(resolveWait, ms))
}

async function waitFor(check, message, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let last
  while (Date.now() < deadline) {
    try {
      last = await check()
      if (last) return last
    } catch (error) {
      last = error instanceof Error ? error.message : String(error)
    }
    await wait(200)
  }
  throw new Error(`${message}; last observed: ${typeof last === 'string' ? last : JSON.stringify(last)}`)
}

async function reserveLoopbackPort() {
  const server = createServer()
  await new Promise((resolveListen, rejectListen) => {
    server.once('error', rejectListen)
    server.listen(0, '127.0.0.1', resolveListen)
  })
  const address = server.address()
  assert.ok(address && typeof address !== 'string', 'reserved loopback port has numeric address')
  const { port } = address
  await new Promise((resolveClose) => server.close(resolveClose))
  return port
}

async function postVerb(base, name, args = {}) {
  const response = await fetch(`${base}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(20_000),
  })
  assert.equal(response.ok, true, `${name} REST request succeeded (${response.status})`)
  const body = await response.json()
  assert.equal(body.ok, true, `${name} succeeded: ${JSON.stringify(body.error ?? body)}`)
  return body.result
}

function waitForExit(child) {
  if (child.exitCode !== null) return Promise.resolve()
  return new Promise((resolveExit) => child.once('exit', resolveExit))
}

async function stopAndWait(child) {
  if (child.exitCode !== null) return true
  if (!child.killed) child.kill('SIGTERM')
  const stoppedAfterTerm = await Promise.race([
    waitForExit(child).then(() => true),
    wait(5_000).then(() => false),
  ])
  if (stoppedAfterTerm) return true
  if (child.exitCode === null) child.kill('SIGKILL')
  return Promise.race([
    waitForExit(child).then(() => true),
    wait(5_000).then(() => false),
  ])
}

async function stageAsset(source, destination, stagedPaths) {
  const staged = join(dirname(destination), `.${basename(destination)}.manual-next-${process.pid}.tmp`)
  stagedPaths.push(staged)
  await copyFile(source, staged)
  return staged
}

async function publishStagedAsset(staged, destination, stagedPaths) {
  await rename(staged, destination)
  stagedPaths.splice(stagedPaths.indexOf(staged), 1)
}

function productionPageEvidence(state, appVersion) {
  assert.equal(state.connection, 'open', 'UI is connected to the real cutd events socket')
  assert.equal(state.version, `cut-ui v${appVersion}`, 'production status bar shows the exact current UI version')
  assert.equal(state.hasProductionAssets, true, 'page is served from the built /assets bundle')
  assert.equal(state.hasSourceModule, false, 'page is not Vite source-module output')
  assert.equal(state.hasDevelopmentLabel, false, 'page does not carry a development label')
  assert.equal(state.hasDisconnectedLabel, false, 'page does not carry a disconnected label')
  assert.equal(state.hasCheckingSetupLabel, false, 'page does not carry a transient Checking setup label')
}

async function captureCurrentUi(appVersion) {
  assert.equal(existsSync(cutd), true, `current-source release binary is built: ${cutd}`)
  assert.equal(existsSync(resolve(uiDist, 'index.html')), true, `current-source production UI is built: ${uiDist}`)

  const runDir = await mkdtemp(join(tmpdir(), 'shellx-cut-manual-ui-'))
  const projectRoot = join(runDir, 'projects')
  const projectDir = join(projectRoot, `${captureName}.cutproj`)
  const tempEditor = join(runDir, basename(editorAsset))
  const tempRecord = join(runDir, basename(recordAsset))
  const port = await reserveLoopbackPort()
  const base = `http://127.0.0.1:${port}`
  const serverLog = []
  const stagedPaths = []
  const child = spawn(cutd, ['serve', '--addr', `127.0.0.1:${port}`, '--ui-dist', uiDist], {
    cwd: root,
    env: {
      ...process.env,
      SHELLX_CUT_HOME: join(runDir, 'home'),
      SHELLX_CUT_PROJECTS_DIR: projectRoot,
      SHELLX_CUT_NO_HWENC: '1',
    },
    stdio: ['ignore', 'ignore', 'pipe'],
  })
  child.stderr.on('data', (chunk) => serverLog.push(chunk.toString()))

  let browser
  try {
    await waitFor(async () => {
      const response = await fetch(`${base}/api/verbs`, { signal: AbortSignal.timeout(1_000) })
      return response.ok
    }, 'temporary source-built cutd did not become ready')

    const doctor = await postVerb(base, 'system.doctor')
    assert.equal(doctor.essential_ok, true, 'temporary capture engine has essential editing tools ready')

    const created = await postVerb(base, 'project.create', {
      name: captureName,
      dir: projectDir,
      settings: { width: 1920, height: 1080, fps: 30, audio_rate: 48_000 },
      starter: 'first-edit',
    })
    assert.equal(basename(created.starter_asset_path), 'first-edit-sample.mp4', 'generic fixture uses Cut’s bundled First edit sample')
    await postVerb(base, 'media.import', {
      path: created.starter_asset_path,
      proxy: false,
      rationale: 'deterministic public documentation fixture',
    })
    const project = await waitFor(async () => {
      const state = await postVerb(base, 'project.state')
      return state.tracks?.some((track) => track.clips?.length > 0) ? state : null
    }, 'bundled generic sample did not reach the temporary timeline')
    assert.equal(project.name, captureName, 'temporary editor project is generic')

    const require = createRequire(appPackage)
    const { chromium } = require('playwright')
    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage({ viewport: { width: 2160, height: 1350 }, deviceScaleFactor: 1 })
    const pageErrors = []
    page.on('pageerror', (error) => pageErrors.push(error.stack || error.message))
    await page.goto(base, { waitUntil: 'domcontentloaded' })
    await page.waitForSelector('[data-cut-panel="topbar"]')
    await page.waitForFunction(() => document.querySelector('[data-cut-connection]')?.getAttribute('data-cut-connection') === 'open')
    await page.waitForFunction((name) => document.querySelector('[data-cut-project]')?.textContent?.includes(name) === true, captureName)
    await page.getByRole('tab', { name: /^Assets/ }).click()
    await page.waitForSelector('[data-cut-panel="assets"]')
    await page.waitForSelector('[data-cut-timeline-toolbar]')
    await page.locator('[data-cut-action="expand-rail"]').click()
    await page.waitForSelector('[data-cut-right-tabs]')
    await page.locator('[data-cut-rail-pin]').click()
    await page.waitForSelector('[data-cut-rail-pinned="true"]')
    await page.waitForFunction(() => !document.body.innerText.includes('Loading review'))
    assert.deepEqual(pageErrors, [], `editor capture has no page errors: ${pageErrors.join('\n')}`)
    const editorState = await page.evaluate(() => {
      const body = document.body.innerText.toLowerCase()
      const resourceUrls = [...document.querySelectorAll('script[src], link[href]')]
        .map((element) => element.getAttribute('src') ?? element.getAttribute('href') ?? '')
      return {
        connection: document.querySelector('[data-cut-connection]')?.getAttribute('data-cut-connection') ?? '',
        version: [...document.querySelectorAll('body *')].map((element) => element.textContent?.trim()).find((value) => /^cut-ui v\d+\.\d+\.\d+$/.test(value ?? '')) ?? '',
        hasProductionAssets: resourceUrls.some((url) => url.startsWith('/assets/')),
        hasSourceModule: resourceUrls.some((url) => url.startsWith('/src/')),
        hasDevelopmentLabel: body.includes('development'),
        hasDisconnectedLabel: body.includes('disconnected'),
        hasCheckingSetupLabel: body.includes('checking setup'),
        project: document.querySelector('[data-cut-project]')?.textContent?.trim() ?? '',
        timeline: Boolean(document.querySelector('[data-cut-timeline-toolbar]')),
        assets: Boolean(document.querySelector('[data-cut-panel="assets"]')),
      }
    })
    productionPageEvidence(editorState, appVersion)
    assert.match(editorState.project, new RegExp(captureName), 'editor shows the generic project name')
    assert.equal(editorState.timeline, true, 'editor capture has the real timeline toolbar')
    assert.equal(editorState.assets, true, 'editor capture has the real Assets panel')
    await page.screenshot({ path: tempEditor })

    await page.getByRole('tab', { name: 'Record' }).click()
    await page.setViewportSize({ width: 1920, height: 1080 })
    await page.waitForSelector('[data-cut-panel="record"]')
    await page.waitForFunction((name) => document.querySelector('[data-cut-project]')?.textContent?.includes(name) === true, captureName)

    const recordDoctor = await postVerb(base, 'screen_record.doctor')
    const screenCapture = recordDoctor.cards?.find((card) => card.name === 'screen_capture')
    assert.equal(
      screenCapture?.status === 'ok' || (screenCapture?.status === 'unknown' && recordDoctor.start_allowed === true),
      true,
      'recording host either proved capture delivery or exposes the deliberate Linux portal start path',
    )
    assert.equal(typeof recordDoctor.camera?.supported, 'boolean', 'Doctor reports the current build camera capability')
    assert.ok(Array.isArray(recordDoctor.camera?.devices), 'Doctor reports current camera choices')
    if (!recordDoctor.camera.supported || recordDoctor.camera.devices.length === 0) {
      await page.waitForSelector('[data-cut-studio-camera-unavailable]')
    }
    await page.waitForFunction(() => {
      const body = document.body.innerText.toLowerCase()
      const start = document.querySelector('[data-cut-action="record-start"]')
      return !body.includes('checking setup') && start && !start.hasAttribute('disabled')
    })
    const recordState = await page.evaluate(() => {
      const body = document.body.innerText.toLowerCase()
      const resourceUrls = [...document.querySelectorAll('script[src], link[href]')]
        .map((element) => element.getAttribute('src') ?? element.getAttribute('href') ?? '')
      const start = document.querySelector('[data-cut-action="record-start"]')
      return {
        connection: document.querySelector('[data-cut-connection]')?.getAttribute('data-cut-connection') ?? '',
        version: [...document.querySelectorAll('body *')].map((element) => element.textContent?.trim()).find((value) => /^cut-ui v\d+\.\d+\.\d+$/.test(value ?? '')) ?? '',
        hasProductionAssets: resourceUrls.some((url) => url.startsWith('/assets/')),
        hasSourceModule: resourceUrls.some((url) => url.startsWith('/src/')),
        hasDevelopmentLabel: body.includes('development'),
        hasDisconnectedLabel: body.includes('disconnected'),
        hasCheckingSetupLabel: body.includes('checking setup'),
        cameraSupported: document.querySelector('[data-cut-studio-camera-status]')?.getAttribute('data-cut-studio-camera-available') === 'true',
        cameraUnavailable: Boolean(document.querySelector('[data-cut-studio-camera-unavailable]')),
        timelineDeferred: document.querySelector('.app__main')?.getAttribute('data-cut-record-timeline-deferred') ?? '',
        timelinePresent: Boolean(document.querySelector('.app__timeline')),
        startDisabled: start?.hasAttribute('disabled') ?? true,
      }
    })
    productionPageEvidence(recordState, appVersion)
    assert.equal(recordState.cameraSupported, recordDoctor.camera.supported, 'Record reflects current Doctor camera capability')
    if (!recordDoctor.camera.supported || recordDoctor.camera.devices.length === 0) {
      assert.equal(recordState.cameraUnavailable, true, 'Record explains missing camera choices')
    }
    assert.equal(recordState.timelineDeferred, 'true', 'empty Record workspace defers the editor timeline')
    assert.equal(recordState.timelinePresent, false, 'empty Record workspace does not show an editor timeline beneath it')
    assert.equal(recordState.startDisabled, false, 'ready recording host exposes an enabled Start recording action')
    assert.deepEqual(pageErrors, [], `record capture has no page errors: ${pageErrors.join('\n')}`)
    await page.screenshot({ path: tempRecord })

    // Both captures are complete and validated before either final filename is
    // touched. Stage both same-directory siblings before publishing either;
    // each replacement is an atomic rename, though two names cannot form one
    // filesystem transaction.
    const stagedEditor = await stageAsset(tempEditor, editorAsset, stagedPaths)
    const stagedRecord = await stageAsset(tempRecord, recordAsset, stagedPaths)
    await publishStagedAsset(stagedEditor, editorAsset, stagedPaths)
    await publishStagedAsset(stagedRecord, recordAsset, stagedPaths)
    return { editor: editorState, record: recordState, port }
  } catch (error) {
    const logTail = serverLog.join('').trim().split('\n').slice(-12).join('\n')
    throw new Error(`${error instanceof Error ? error.message : String(error)}${logTail ? `\ncutd log tail:\n${logTail}` : ''}`, { cause: error })
  } finally {
    if (browser) await browser.close()
    const stopped = await stopAndWait(child)
    if (!stopped) process.stderr.write('manual-ui-assets: owned temporary cutd did not exit after SIGKILL grace\n')
    await Promise.all(stagedPaths.map((path) => rm(path, { force: true })))
    await rm(runDir, { recursive: true, force: true })
  }
}

const html = text(manual)
const appVersion = JSON.parse(text(appPackage)).version
const desktopVersion = JSON.parse(text(desktopConfig)).version
const manualVersion = html.match(/data-app-version="([^"]+)"[^>]*>([^<]+)</)

assert.equal(appVersion, desktopVersion, 'UI and desktop shell use the same release version')
assert.ok(manualVersion, 'manual has a machine-readable visible release marker')
assert.equal(manualVersion[1], appVersion, 'manual data-app-version matches the current UI release')
assert.equal(manualVersion[2].trim(), appVersion, 'manual visibly shows the current UI release')

assert.match(html, /cut-main-editor-current\.png/, 'manual has an editor image path')
assert.match(html, /cut-recording-studio-current\.png/, 'manual has a Record image path')

for (const path of [editorAsset, recordAsset]) {
  assert.equal(existsSync(path), true, `${path} exists`)
}
const captureEvidence = writeCurrent ? await captureCurrentUi(appVersion) : null
const editorPng = pngSize(editorAsset)
const recordPng = pngSize(recordAsset)
assert.deepEqual({ width: editorPng.width, height: editorPng.height }, { width: 2160, height: 1350 }, 'editor capture keeps its documented desktop viewport')
assert.deepEqual({ width: recordPng.width, height: recordPng.height }, { width: 1920, height: 1080 }, 'Record capture keeps its documented desktop viewport')
assert.ok(editorPng.bytes > 50_000, 'editor capture is a nontrivial raster image')
assert.ok(recordPng.bytes > 50_000, 'Record capture is a nontrivial raster image')

console.log(JSON.stringify({
  result: 'PASS',
  editor: { ...editorPng, sha256: sha256(editorAsset) },
  record: { ...recordPng, sha256: sha256(recordAsset) },
  ...(captureEvidence ? { captureEvidence } : {}),
}, null, 2))
