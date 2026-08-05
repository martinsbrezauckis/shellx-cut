import assert from 'node:assert/strict'
import { mkdir, writeFile } from 'node:fs/promises'
import { basename, extname, join, resolve } from 'node:path'
import { tmpdir } from 'node:os'

const VIDEO_PATH = process.env.SHELLX_CUT_WDIO_CLIP
const IMAGE_PATH = process.env.SHELLX_CUT_WDIO_IMAGE
const OUT_DIR = process.env.SHELLX_CUT_WDIO_OUT || join(tmpdir(), `shellx-cut-drop-to-create-${Date.now()}`)
const DROP_CASE = process.env.SHELLX_CUT_WDIO_DROP_CASE || 'both'

if (!VIDEO_PATH) throw new Error('SHELLX_CUT_WDIO_CLIP must point to a real video')
if (!IMAGE_PATH) throw new Error('SHELLX_CUT_WDIO_IMAGE must point to a real still image')
if (!['video', 'image', 'both'].includes(DROP_CASE)) {
  throw new Error('SHELLX_CUT_WDIO_DROP_CASE must be video, image, or both')
}

const sleep = (ms) => new Promise((resolveDelay) => setTimeout(resolveDelay, ms))

async function pageAsync(fn, ...args) {
  const result = await browser.executeAsync(fn, ...args)
  if (!result?.ok) throw new Error(result?.error || 'page eval failed')
  return result.value
}

async function verb(name, args = {}) {
  return pageAsync((verbName, verbArgs, done) => {
    fetch(`/api/verb/${verbName}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(verbArgs),
    })
      .then((response) => response.json())
      .then((value) => done({ ok: true, value }))
      .catch((error) => done({ ok: false, error: String(error?.stack || error?.message || error) }))
  }, name, args)
}

async function waitForSelector(selector, timeoutMs = 30000) {
  await browser.waitUntil(
    () => pageAsync((value, done) => done({ ok: true, value: !!document.querySelector(value) }), selector),
    { timeout: timeoutMs, timeoutMsg: `missing selector ${selector}` },
  )
}

async function currentState() {
  const response = await verb('project.state', {})
  return response.ok ? response.result : null
}

async function waitForProject(predicate, timeoutMs = 90000) {
  const started = Date.now()
  let current = null
  while (Date.now() - started < timeoutMs) {
    current = await currentState()
    if (current && predicate(current)) return current
    await sleep(350)
  }
  return null
}

async function waitForUiProject(name, timeoutMs = 30000) {
  await browser.waitUntil(
    () => pageAsync((expected, done) => {
      const text = document.querySelector('[data-cut-project]')?.textContent || ''
      done({ ok: true, value: text.includes(`${expected}.cutproj`) })
    }, name),
    { timeout: timeoutMs, timeoutMsg: `UI did not finish switching to ${name}.cutproj` },
  )
}

async function emitTauriDropEvent(name, path) {
  await pageAsync((eventName, mediaPath, done) => {
    const events = window.__TAURI__?.event
    const label = window.__TAURI_INTERNALS__?.metadata?.currentWebview?.label
    if (!events?.emitTo || !label) {
      done({ ok: false, error: 'Tauri Webview-targeted event.emitTo is unavailable' })
      return
    }
    const payload = { paths: [mediaPath], position: { x: 420, y: 420 } }
    Promise.resolve(events.emitTo({ kind: 'Webview', label }, eventName, payload))
      .then(() => done({ ok: true, value: true }))
      .catch((error) => done({ ok: false, error: String(error?.stack || error?.message || error) }))
  }, name, resolve(path))
}

async function hoverNativeDrop(path) {
  await emitTauriDropEvent('tauri://drag-enter', path)
  await emitTauriDropEvent('tauri://drag-over', path)
}

async function waitForNativeDropHover(path, timeoutMs = 15000) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await hoverNativeDrop(path)
    const visible = await pageAsync((done) => {
      done({ ok: true, value: !!document.querySelector('[data-cut-dropzone="over"]') })
    })
    if (visible) return
    await sleep(250)
  }
  throw new Error('native drag-enter/drag-over did not reach the mounted DropZone listener')
}

async function commitNativeDrop(path) {
  await emitTauriDropEvent('tauri://drag-drop', path)
}

async function cleanupProject(name) {
  let listed = null
  try {
    listed = await verb('project.list', { sort: 'recent' })
  } catch {
    return
  }
  const entry = listed.result?.projects?.find((project) => project.name === name)
  let didClose = false
  const started = Date.now()
  while (Date.now() - started < 90000) {
    const closed = await verb('project.close', {}).catch(() => null)
    if (closed?.ok) {
      didClose = true
      break
    }
    if (closed?.error?.code !== 'job_cancel_pending') return
    await sleep(500)
  }
  if (!didClose) return
  if (entry?.path) {
    await verb('project.delete', { path: entry.path }).catch(() => {})
    await verb('project.forget', { path: entry.path }).catch(() => {})
  }
}

async function resetToEmptyProjectSurface() {
  await verb('project.close', {}).catch(() => {})
  await browser.refresh()
  await waitForSelector('[data-cut-left-tab="projects"]')
  await waitForSelector('[data-cut-panel="projects"]')
  await browser.waitUntil(
    () => pageAsync((done) => {
      const tab = document.querySelector('[data-cut-left-tab="projects"]')
      done({ ok: true, value: tab?.getAttribute('aria-selected') === 'true' })
    }),
    { timeout: 30000, timeoutMsg: 'Projects did not become the empty-app default tab' },
  )
  await browser.waitUntil(
    () => pageAsync((done) => {
      const text = document.querySelector('[data-cut-project]')?.textContent || ''
      done({ ok: true, value: /no project/i.test(text) })
    }),
    { timeout: 30000, timeoutMsg: 'UI did not settle on the no-project state' },
  )
}

function projectName(path) {
  const file = basename(path)
  return file.slice(0, Math.max(0, file.length - extname(file).length))
}

function clipRows(project) {
  return (project?.tracks || []).flatMap((track) =>
    (track.clips || []).map((clip) => ({ ...clip, trackKind: track.kind })),
  )
}

function clipTimelineDurationMs(clip) {
  const sourceStart = Number(clip.src_in_ms)
  const sourceEnd = Number(clip.src_out_ms)
  const speed = Math.abs(Number(clip.speed) || 1)
  if (!Number.isFinite(sourceStart) || !Number.isFinite(sourceEnd) || speed <= 0) return null
  return Math.round(Math.abs(sourceEnd - sourceStart) / speed)
}

describe('ShellX Cut no-project native media drop', () => {
  it('opens Projects first and creates a populated project from video or image', async () => {
    await mkdir(OUT_DIR, { recursive: true })
    const checks = []
    const createdNames = []
    try {
      await resetToEmptyProjectSurface()
      checks.push({
        id: 'projects-first',
        pass: true,
        detail: 'empty native app renders Projects as the selected first tab',
      })

      if (DROP_CASE !== 'image') {
        const videoName = projectName(VIDEO_PATH)
        createdNames.push(videoName)
        await waitForNativeDropHover(VIDEO_PATH)
        const videoHint = await pageAsync((done) => {
          done({ ok: true, value: document.querySelector('[data-cut-dropzone="over"]')?.textContent || '' })
        })
        assert.match(videoHint, /start a project/i)
        await commitNativeDrop(VIDEO_PATH)
        const videoProject = await waitForProject((project) =>
          project.name === videoName
          && Object.keys(project.assets || {}).length >= 1
          && clipRows(project).some((clip) => clip.trackKind === 'video'),
        )
        assert.ok(videoProject, 'video drop did not create and populate a project')
        await waitForUiProject(videoName)
        const videoAsset = Object.values(videoProject.assets || {})[0]
        const videoProbe = videoAsset?.probe || {}
        if (Number(videoProbe.width) > 0 && Number(videoProbe.height) > 0) {
          assert.equal(videoProject.settings.width, Number(videoProbe.width), 'first video width was not adopted')
          assert.equal(videoProject.settings.height, Number(videoProbe.height), 'first video height was not adopted')
        }
        if (Number(videoProbe.fps) > 0) {
          assert.ok(
            Math.abs(videoProject.settings.fps - Number(videoProbe.fps)) < 0.02,
            'first video frame rate was not adopted',
          )
        }
        await browser.saveScreenshot(join(OUT_DIR, 'video-drop-created-project.png'))
        checks.push({
          id: 'video-drop-create',
          pass: true,
          detail: 'Tauri drag-drop bridge created a named project, imported video, and placed its video clip',
        })

        if (DROP_CASE === 'both') {
          await cleanupProject(videoName)
          await resetToEmptyProjectSurface()
        }
      }

      if (DROP_CASE !== 'video') {
        const imageName = projectName(IMAGE_PATH)
        createdNames.push(imageName)
        await waitForNativeDropHover(IMAGE_PATH)
        await commitNativeDrop(IMAGE_PATH)
        const imageProject = await waitForProject((project) =>
          project.name === imageName
          && Object.keys(project.assets || {}).length >= 1
          && clipRows(project).some((clip) =>
            clip.trackKind === 'video'
            && clipTimelineDurationMs(clip) === 5_000,
          ),
        )
        assert.ok(imageProject, 'image drop did not create a project with a five-second timeline clip')
        await waitForUiProject(imageName)
        await browser.saveScreenshot(join(OUT_DIR, 'image-drop-created-project.png'))
        checks.push({
          id: 'image-drop-create',
          pass: true,
          detail: 'Tauri drag-drop bridge created a named project and placed the still for five seconds',
        })
      }

      const receipt = {
        schema: 'shellx-cut/native-drop-to-create-candidate/1',
        candidate_only: true,
        installed_app: false,
        case: DROP_CASE,
        gesture: 'webdriver-test-only-tauri-event-bridge',
        limitation: 'The final installed three-host gate must still perform a real OS file drag.',
        checks,
      }
      await writeFile(
        join(OUT_DIR, 'drop-to-create-receipt.json'),
        `${JSON.stringify(receipt, null, 2)}\n`,
        'utf8',
      )
    } catch (error) {
      const diagnostic = {
        schema: 'shellx-cut/native-drop-to-create-failure/1',
        error: String(error?.stack || error?.message || error),
        state_envelope: await verb('project.state', {}).catch((stateError) => ({
          diagnostic_error: String(stateError?.message || stateError),
        })),
        state: await currentState().catch((stateError) => ({
          diagnostic_error: String(stateError?.message || stateError),
        })),
        projects: await verb('project.list', { sort: 'recent' }).catch((listError) => ({
          diagnostic_error: String(listError?.message || listError),
        })),
        overlay: await pageAsync((done) => {
          const element = document.querySelector('[data-cut-dropzone]')
          done({
            ok: true,
            value: element
              ? {
                  kind: element.getAttribute('data-cut-dropzone'),
                  text: element.textContent || '',
                }
              : null,
          })
        }).catch(() => null),
      }
      await writeFile(
        join(OUT_DIR, 'drop-to-create-failure.json'),
        `${JSON.stringify(diagnostic, null, 2)}\n`,
        'utf8',
      )
      await browser.saveScreenshot(join(OUT_DIR, 'drop-to-create-failure.png')).catch(() => {})
      throw error
    } finally {
      for (const name of createdNames.reverse()) await cleanupProject(name).catch(() => {})
    }
  })
})
