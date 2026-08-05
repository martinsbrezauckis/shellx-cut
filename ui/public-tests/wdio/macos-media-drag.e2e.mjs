import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

const ASSET_CLIP = process.env.SHELLX_CUT_WDIO_CLIP
const LIBRARY_CLIP = process.env.SHELLX_CUT_WDIO_LIBRARY_CLIP || ASSET_CLIP
const OUT_DIR = process.env.SHELLX_CUT_WDIO_OUT || join(tmpdir(), `shellx-cut-media-drag-${Date.now()}`)
const USE_NATIVE_INPUT = process.env.SHELLX_CUT_WDIO_NATIVE_INPUT === '1'
const NATIVE_DRAG = resolve(dirname(fileURLToPath(import.meta.url)), 'macos-native-drag.swift')
const execFileAsync = promisify(execFile)

if (!ASSET_CLIP) throw new Error('SHELLX_CUT_WDIO_CLIP must point to a real video clip on the Mac')

function sleep(ms) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, ms))
}

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

async function state() {
  const response = await verb('project.state', {})
  assert.equal(response.ok, true, `project.state failed: ${JSON.stringify(response.error ?? response)}`)
  return response.result
}

function clipCount(project) {
  return (project?.tracks || []).reduce((total, track) => total + (track.clips || []).length, 0)
}

async function waitForState(predicate, timeoutMs = 60000) {
  const started = Date.now()
  let current = null
  while (Date.now() - started < timeoutMs) {
    current = await state()
    if (predicate(current)) return current
    await sleep(350)
  }
  return null
}

async function waitForSelector(selector, timeoutMs = 30000) {
  await browser.waitUntil(
    () => pageAsync((value, done) => done({ ok: true, value: !!document.querySelector(value) }), selector),
    { timeout: timeoutMs, timeoutMsg: `missing selector ${selector}` },
  )
}

async function click(selector) {
  await waitForSelector(selector)
  await pageAsync((value, done) => {
    const element = document.querySelector(value)
    element.scrollIntoView({ block: 'center', inline: 'center' })
    element.click()
    done({ ok: true, value: true })
  }, selector)
}

async function waitForIdle(timeoutMs = 90000) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const jobs = await verb('jobs.list', {})
    const active = (jobs.result?.jobs || []).filter((job) => job.state === 'queued' || job.state === 'running')
    if (!active.length) return
    await sleep(500)
  }
  throw new Error('jobs did not become idle')
}

async function installDragLog() {
  await pageAsync((done) => {
    window.__cutMediaDragLog = []
    const record = (name) => (event) => {
      window.__cutMediaDragLog.push({
        name,
        target: event.target instanceof Element
          ? event.target.closest('[data-cut-asset-card],[data-cut-library-card]')?.getAttribute('data-cut-asset-card')
            || event.target.closest('[data-cut-library-card]')?.getAttribute('data-cut-library-card')
            || event.target.tagName
          : null,
      })
    }
    for (const name of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel', 'mousedown', 'mousemove', 'mouseup']) {
      window.addEventListener(name, record(name), true)
    }
    document.addEventListener('cut:asset-dragmove', record('asset-dragmove'))
    document.addEventListener('cut:asset-drop', record('asset-drop'))
    done({ ok: true, value: true })
  })
}

async function clearDragLog() {
  await pageAsync((done) => {
    window.__cutMediaDragLog = []
    done({ ok: true, value: true })
  })
}

async function dragLog() {
  return pageAsync((done) => done({ ok: true, value: window.__cutMediaDragLog || [] }))
}

async function nativeDrag(sourceSelector) {
  const points = await pageAsync((selector, done) => {
    const source = document.querySelector(selector)
    const target = document.querySelector('[data-cut-timeline-scroll]')
    if (!source || !target) {
      done({ ok: false, error: `drag endpoint missing: ${selector}` })
      return
    }
    source.scrollIntoView({ block: 'center', inline: 'center' })
    const sourceRect = source.getBoundingClientRect()
    const targetRect = target.getBoundingClientRect()
    done({
      ok: true,
      value: {
        sx: Math.round(sourceRect.left + sourceRect.width / 2),
        sy: Math.round(sourceRect.top + sourceRect.height / 2),
        ex: Math.round(targetRect.left + targetRect.width * 0.62),
        ey: Math.round(targetRect.top + Math.min(105, targetRect.height * 0.45)),
        screenX: window.screenX,
        screenY: window.screenY,
        outerWidth: window.outerWidth,
        outerHeight: window.outerHeight,
        innerWidth: window.innerWidth,
        innerHeight: window.innerHeight,
      },
    })
  }, sourceSelector)
  const insetX = Math.max(0, (points.outerWidth - points.innerWidth) / 2)
  const insetY = Math.max(0, points.outerHeight - points.innerHeight - insetX)
  const args = [
    Math.round(points.screenX + insetX + points.sx),
    Math.round(points.screenY + insetY + points.sy),
    Math.round(points.screenX + insetX + points.ex),
    Math.round(points.screenY + insetY + points.ey),
  ].map(String)
  if (USE_NATIVE_INPUT) {
    await execFileAsync('/usr/bin/swift', [NATIVE_DRAG, ...args], { timeout: 30000 })
    return
  }
  await browser.performActions([{
    type: 'pointer',
    id: 'media-drag-mouse',
    parameters: { pointerType: 'mouse' },
    actions: [
      { type: 'pointerMove', duration: 0, x: points.sx, y: points.sy, origin: 'viewport' },
      { type: 'pointerDown', button: 0 },
      { type: 'pointerMove', duration: 650, x: points.ex, y: points.ey, origin: 'viewport' },
      { type: 'pause', duration: 250 },
      { type: 'pointerUp', button: 0 },
    ],
  }])
  await browser.releaseActions()
}

async function syntheticPointerDrag(sourceSelector, cancel = false) {
  await pageAsync((selector, shouldCancel, done) => {
    const source = document.querySelector(selector)
    const target = document.querySelector('[data-cut-timeline-scroll]')
    if (!source || !target) {
      done({ ok: false, error: `pointer drag endpoint missing: ${selector}` })
      return
    }
    const sourceRect = source.getBoundingClientRect()
    const targetRect = target.getBoundingClientRect()
    const common = { bubbles: true, cancelable: true, pointerId: 77, pointerType: 'mouse', isPrimary: true, button: 0 }
    source.dispatchEvent(new PointerEvent('pointerdown', {
      ...common,
      clientX: sourceRect.left + sourceRect.width / 2,
      clientY: sourceRect.top + sourceRect.height / 2,
      buttons: 1,
    }))
    const end = {
      ...common,
      clientX: targetRect.left + targetRect.width * 0.62,
      clientY: targetRect.top + Math.min(105, targetRect.height * 0.45),
    }
    window.dispatchEvent(new PointerEvent('pointermove', { ...end, buttons: 1 }))
    window.dispatchEvent(new PointerEvent(shouldCancel ? 'pointercancel' : 'pointerup', { ...end, buttons: 0 }))
    done({ ok: true, value: true })
  }, sourceSelector, cancel)
}

describe('ShellX Cut macOS media placement', () => {
  it('drags Assets and explicitly inserts Library media in the native WKWebView', async () => {
    await mkdir(OUT_DIR, { recursive: true })
    await waitForSelector('[data-cut-panel="topbar"]')
    await waitForSelector('[data-cut-panel="timeline"]')
    await installDragLog()

    const projectName = `wdio-media-drag-${Date.now()}`
    const projectDir = join(OUT_DIR, `${projectName}.cutproj`)
    let libraryId = null
    try {
      const created = await verb('project.create', {
        name: projectName,
        dir: projectDir,
        settings: { width: 1280, height: 720, fps: 30 },
      })
      assert.equal(created.ok, true, `project.create failed: ${JSON.stringify(created.error ?? created)}`)
      const imported = await verb('media.import', {
        path: resolve(ASSET_CLIP),
        proxy: false,
        rationale: 'macOS native Assets drag seed',
      })
      assert.equal(imported.ok, true, `media.import failed: ${JSON.stringify(imported.error ?? imported)}`)
      await waitForIdle()
      const seeded = await waitForState((project) => clipCount(project) >= 2)
      assert.ok(seeded, 'initial linked A/V clip was auto-placed')
      const assetId = Object.keys(seeded.assets || {})[0]
      assert.ok(assetId, 'seeded asset id exists')

      await click('[data-cut-left-tab="assets"]')
      await waitForSelector(`[data-cut-asset-card="${assetId}"] .assets__icon, [data-cut-asset-card="${assetId}"] .assets__thumb`)
      const beforeAssetDrop = clipCount(await state())
      await nativeDrag(`[data-cut-asset-card="${assetId}"] .assets__icon, [data-cut-asset-card="${assetId}"] .assets__thumb`)
      const afterAssetDrop = await waitForState((project) => clipCount(project) > beforeAssetDrop, 30000)
      const assetEvents = await dragLog()
      assert.ok(afterAssetDrop, `Assets drag did not add clips; events=${JSON.stringify(assetEvents)}`)
      assert.equal(assetEvents.some((event) => event.name === 'pointerdown' || event.name === 'mousedown'), true, JSON.stringify(assetEvents))
      assert.equal(assetEvents.some((event) => event.name === 'pointercancel'), false, JSON.stringify(assetEvents))
      assert.equal(assetEvents.some((event) => event.name === 'asset-drop'), true, JSON.stringify(assetEvents))

      await clearDragLog()
      const beforeCancelledPointer = clipCount(await state())
      await syntheticPointerDrag(`[data-cut-asset-card="${assetId}"] .assets__icon, [data-cut-asset-card="${assetId}"] .assets__thumb`, true)
      await sleep(500)
      const cancelledPointerEvents = await dragLog()
      assert.equal(clipCount(await state()), beforeCancelledPointer, `pointercancel inserted media: ${JSON.stringify(cancelledPointerEvents)}`)
      assert.equal(cancelledPointerEvents.some((event) => event.name === 'pointercancel'), true, JSON.stringify(cancelledPointerEvents))
      assert.equal(cancelledPointerEvents.some((event) => event.name === 'asset-drop'), false, JSON.stringify(cancelledPointerEvents))

      await clearDragLog()
      const beforePointerDrop = clipCount(await state())
      await syntheticPointerDrag(`[data-cut-asset-card="${assetId}"] .assets__icon, [data-cut-asset-card="${assetId}"] .assets__thumb`)
      const afterPointerDrop = await waitForState((project) => clipCount(project) > beforePointerDrop, 30000)
      const pointerEvents = await dragLog()
      assert.ok(afterPointerDrop, `pointer drop did not add clips: ${JSON.stringify(pointerEvents)}`)
      assert.equal(pointerEvents.some((event) => event.name === 'pointerdown'), true, JSON.stringify(pointerEvents))
      assert.equal(pointerEvents.some((event) => event.name === 'asset-drop'), true, JSON.stringify(pointerEvents))

      const added = await verb('library.add', {
        path: resolve(LIBRARY_CLIP),
        name: `wdio-mac-library-insert-${Date.now()}`,
      })
      assert.equal(added.ok, true, `library.add failed: ${JSON.stringify(added.error ?? added)}`)
      libraryId = added.result?.item?.id
      assert.ok(libraryId, 'library item id exists')
      await click('[data-cut-library-btn]')
      await waitForSelector('[data-cut-library-workspace]')
      await waitForSelector(`[data-cut-library-card="${libraryId}"] .lb-thumb-img, [data-cut-library-card="${libraryId}"] .lb-thumb-glyph`)
      const beforeLibraryInsert = clipCount(await state())
      await click(`[data-cut-library-insert="${libraryId}"]`)
      const afterLibraryInsert = await waitForState((project) => clipCount(project) > beforeLibraryInsert, 90000)
      assert.ok(afterLibraryInsert, 'Library workspace Insert at playhead did not add clips')
    } finally {
      if (libraryId) await verb('library.remove', { id: libraryId }).catch(() => {})
      await verb('project.close', {}).catch(() => {})
      await verb('project.delete', { path: projectDir }).catch(() => {})
      await verb('project.forget', { path: projectDir }).catch(() => {})
    }
  })
})
