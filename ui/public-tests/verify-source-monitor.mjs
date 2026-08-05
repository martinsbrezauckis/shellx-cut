import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '../..')
const BASE_CLIP = resolve(REPO, 'testdata/talking_head.mp4')
const SOURCE_CLIP = resolve(REPO, 'testdata/insert_clip.mp4')
const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6212'
const APP = process.env.SWEEP_APP || CUTD
const temp = mkdtempSync(join(tmpdir(), 'cut-source-monitor-'))
const projectDir = join(temp, 'source-monitor-rig.cutproj')
const checks = []

function check(name, pass, detail) {
  checks.push({ name, pass, detail })
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}  ${detail}`)
}

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:ui' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(30_000),
  })
  return response.json()
}

async function projectState() {
  const envelope = await verb('project.state')
  if (!envelope.ok) throw new Error(envelope.error?.message || 'project.state failed')
  return envelope.result
}

async function waitForProject(predicate, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const project = await projectState()
    if (predicate(project)) return project
    await new Promise((resolveWait) => setTimeout(resolveWait, 120))
  }
  throw new Error('timed out waiting for project state')
}

function mediaDuration(clip) {
  if (clip.kind === 'gap') return clip.duration_ms || 0
  const sourceDuration = Math.max(0, (clip.src_out_ms || 0) - (clip.src_in_ms || 0))
  const speed = Number.isFinite(clip.speed) && clip.speed > 0 ? clip.speed : 1
  return Math.round(sourceDuration / speed)
}

function clipStart(track, clipId) {
  let cursor = 0
  for (const clip of track.clips || []) {
    const duration = mediaDuration(clip)
    cursor = Math.max(0, cursor - Math.min(clip.xfade_in_ms || 0, duration))
    if (clip.id === clipId) return cursor
    cursor += duration
  }
  return null
}

let browser
try {
  const created = await verb('project.create', {
    name: 'source-monitor-rig',
    dir: projectDir,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  if (!created.ok) throw new Error(created.error?.message || 'project.create failed')

  const firstImport = await verb('media.import', { path: BASE_CLIP, proxy: false })
  if (!firstImport.ok) throw new Error(firstImport.error?.message || 'first media.import failed')
  await waitForProject((project) => project.tracks.some((track) => track.kind === 'audio' && track.clips?.some((clip) => clip.asset)))

  const secondImport = await verb('media.import', { path: SOURCE_CLIP, proxy: false })
  if (!secondImport.ok) throw new Error(secondImport.error?.message || 'second media.import failed')
  const ready = await waitForProject((project) => Object.values(project.assets || {}).some((asset) =>
    asset.path === SOURCE_CLIP && (asset.probe?.duration_ms || 0) > 0,
  ))
  const sourceAsset = Object.entries(ready.assets).find(([, asset]) => asset.path === SOURCE_CLIP)?.[0]
  if (!sourceAsset) throw new Error('source asset was not registered')
  const initiallyUsed = ready.tracks.some((track) => track.clips?.some((clip) => clip.asset === sourceAsset))

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
  await page.locator('[data-cut-left-tab="assets"]').click()
  const openButton = page.locator(`[data-cut-source-monitor-open="${sourceAsset}"]`)
  await openButton.waitFor()
  check('unused timed asset exposes Source monitor', !initiallyUsed && await openButton.isVisible(), `asset=${sourceAsset} initiallyUsed=${initiallyUsed}`)

  await verb('ui.playhead', { at_ms: 12_000 })
  await page.waitForTimeout(200)
  await openButton.click()
  const dialog = page.locator(`[data-cut-source-monitor="${sourceAsset}"]`)
  await dialog.waitFor()
  const media = dialog.locator('video')
  await page.waitForFunction((assetId) => {
    const element = document.querySelector(`[data-cut-source-monitor="${assetId}"] video`)
    return element instanceof HTMLVideoElement && Number.isFinite(element.duration) && element.duration > 0
  }, sourceAsset)
  const mediaContract = await media.evaluate((element) => ({
    controls: element.controls,
    path: new URL(element.currentSrc || element.src).pathname,
    duration: element.duration,
  }))
  check(
    'monitor streams original source with transport',
    mediaContract.controls && mediaContract.path === `/api/source/${sourceAsset}` && mediaContract.duration >= 9.9,
    `controls=${mediaContract.controls} path=${mediaContract.path} duration=${mediaContract.duration.toFixed(3)}s`,
  )

  await media.evaluate((element) => {
    element.currentTime = 2
    element.dispatchEvent(new Event('timeupdate', { bubbles: true }))
  })
  await page.locator('[data-cut-source-current]').filter({ hasText: '0:02.000' }).waitFor()
  await page.locator('[data-cut-source-mark-in]').click()
  await media.evaluate((element) => {
    element.currentTime = 5.5
    element.dispatchEvent(new Event('timeupdate', { bubbles: true }))
  })
  await page.locator('[data-cut-source-current]').filter({ hasText: '0:05.500' }).waitFor()
  await page.locator('[data-cut-source-mark-out]').click()
  check(
    'mark In and Out preserve the selected source range',
    (await page.locator('[data-cut-source-in]').textContent()) === '0:02.000'
      && (await page.locator('[data-cut-source-out]').textContent()) === '0:05.500',
    `in=${await page.locator('[data-cut-source-in]').textContent()} out=${await page.locator('[data-cut-source-out]').textContent()}`,
  )

  const insertRequests = []
  const onRequest = (request) => {
    if (request.url().includes('/api/verb/edit.insert')) insertRequests.push(request.postDataJSON())
  }
  page.on('request', onRequest)
  await page.locator('[data-cut-source-insert]').click()
  const inserted = await waitForProject((project) => {
    const clips = project.tracks.flatMap((track) => track.clips || [])
      .filter((clip) => clip.asset === sourceAsset && clip.src_in_ms === 2000 && clip.src_out_ms === 5500)
    return clips.length === 2
  })
  page.off('request', onRequest)

  const videoTrack = inserted.tracks.find((track) => track.kind === 'video' && track.clips?.some((clip) => clip.asset === sourceAsset && clip.src_in_ms === 2000))
  const audioTrack = inserted.tracks.find((track) => track.kind === 'audio' && track.clips?.some((clip) => clip.asset === sourceAsset && clip.src_in_ms === 2000))
  const videoClip = videoTrack?.clips.find((clip) => clip.asset === sourceAsset && clip.src_in_ms === 2000)
  const audioClip = audioTrack?.clips.find((clip) => clip.asset === sourceAsset && clip.src_in_ms === 2000)
  const videoStart = videoTrack && videoClip ? clipStart(videoTrack, videoClip.id) : null
  const audioStart = audioTrack && audioClip ? clipStart(audioTrack, audioClip.id) : null
  const rangedRequests = insertRequests.filter((request) => JSON.stringify(request.src_range_ms) === JSON.stringify([2000, 5500]))
  check(
    'range insert creates aligned linked video and audio',
    rangedRequests.length === 2 && videoStart === 12_000 && audioStart === 12_000,
    `requests=${insertRequests.length} ranged=${rangedRequests.length} videoStart=${videoStart} audioStart=${audioStart}`,
  )

  await page.locator('[data-cut-source-note]').filter({ hasText: 'Inserted 0:03.500 at 0:12.000' }).waitFor()
  const layout = await page.evaluate((assetId) => {
    const root = document.documentElement
    const dialogElement = document.querySelector(`[data-cut-source-monitor="${assetId}"]`)
    const rect = dialogElement?.getBoundingClientRect()
    return {
      rootOverflow: root.scrollWidth - root.clientWidth,
      dialogOverflow: dialogElement ? dialogElement.scrollWidth - dialogElement.clientWidth : -1,
      insideViewport: !!rect && rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight,
    }
  }, sourceAsset)
  check(
    'Source monitor fits the supported minimum window',
    layout.rootOverflow === 0 && layout.dialogOverflow === 0 && layout.insideViewport,
    `rootOverflow=${layout.rootOverflow} dialogOverflow=${layout.dialogOverflow} inside=${layout.insideViewport}`,
  )

  await page.keyboard.press('Escape')
  await dialog.waitFor({ state: 'detached' })
  check('Escape closes Source monitor', true, 'dialog detached')
} finally {
  await browser?.close().catch(() => {})
  await verb('project.close').catch(() => {})
  await verb('project.forget', { path: projectDir }).catch(() => {})
  rmSync(temp, { recursive: true, force: true })
}

if (checks.some((entry) => !entry.pass)) process.exitCode = 1
