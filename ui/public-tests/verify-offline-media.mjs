import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '../..')
const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6220'
const APP = process.env.SWEEP_APP || CUTD
const FIXTURE = process.env.OFFLINE_MEDIA_FIXTURE || resolve(REPO, 'testdata/talking_head.mp4')
const EVIDENCE_DIR = process.env.OFFLINE_MEDIA_EVIDENCE_DIR || ''
const TMP = mkdtempSync(join(tmpdir(), 'shellx-cut-offline-media-'))
const PROJECT = join(TMP, 'offline-media-rig.cutproj')
const SOURCE = join(TMP, 'offline-source.mp4')
const REPLACEMENT = join(TMP, 'offline-source-moved.mp4')
const checks = []

function check(name, pass, detail) {
  checks.push({ name, pass: !!pass, detail })
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}  ${detail}`)
}

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:offline-media' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(90_000),
  })
  return response.json()
}

async function projectState() {
  const response = await verb('project.state')
  if (!response.ok) throw new Error(response.error?.message || 'project.state failed')
  return response.result
}

async function waitForState(predicate, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await projectState()
    if (predicate(last)) return last
    await new Promise((resolveWait) => setTimeout(resolveWait, 150))
  }
  throw new Error(`project state condition timed out: ${JSON.stringify(last)?.slice(0, 600)}`)
}

async function waitForJobs(timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const response = await verb('jobs.list')
    const active = response.result?.jobs?.some((job) => job.state === 'queued' || job.state === 'running')
    if (!active) return
    await new Promise((resolveWait) => setTimeout(resolveWait, 200))
  }
  throw new Error('import jobs did not settle')
}

async function screenshot(page, name) {
  if (!EVIDENCE_DIR) return
  mkdirSync(EVIDENCE_DIR, { recursive: true })
  await page.screenshot({ path: join(EVIDENCE_DIR, `${name}.png`), fullPage: false })
}

let browser
let assetId = ''
try {
  if (!existsSync(FIXTURE)) throw new Error(`offline-media fixture missing: ${FIXTURE}`)
  copyFileSync(FIXTURE, SOURCE)
  copyFileSync(FIXTURE, REPLACEMENT)

  const created = await verb('project.create', {
    name: 'offline-media-rig',
    dir: PROJECT,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created.error)}`)
  const imported = await verb('media.import', {
    path: SOURCE,
    proxy: false,
    rationale: 'offline-media verifier seed',
  })
  assetId = imported.result?.asset_id || ''
  if (!imported.ok || !assetId) throw new Error(`media.import failed: ${JSON.stringify(imported.error)}`)
  await waitForJobs()

  let project = await projectState()
  let clip = project.tracks.flatMap((track) => track.clips || []).find((item) => item.asset === assetId)
  if (!clip) {
    const videoTrack = project.tracks.find((track) => track.kind === 'video')
    if (!videoTrack) throw new Error('project has no video track for offline fixture')
    const inserted = await verb('edit.insert', { asset: assetId, track: videoTrack.id, at_ms: 0 })
    if (!inserted.ok) throw new Error(`edit.insert failed: ${JSON.stringify(inserted.error)}`)
    project = await waitForState((state) => state.tracks.some((track) =>
      track.clips?.some((item) => item.asset === assetId)))
    clip = project.tracks.flatMap((track) => track.clips || []).find((item) => item.asset === assetId)
  }
  await verb('ui.playhead', { at_ms: 100 })

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
  const pageErrors = []
  page.on('pageerror', (error) => pageErrors.push(error.message))
  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
  await page.locator('[data-cut-left-tab="assets"]').click()
  const refresh = page.locator('[data-cut-media-health-refresh]').first()
  await refresh.waitFor({ state: 'visible', timeout: 15_000 })
  await refresh.click()
  await page.locator(`[data-cut-asset-card="${assetId}"]`).waitFor({ state: 'visible' })
  check('online baseline has no offline markers',
    await page.locator(`[data-cut-asset-offline="${assetId}"]`).count() === 0
      && await page.locator(`[data-cut-preview-offline="${assetId}"]`).count() === 0,
    `asset=${assetId}`)

  unlinkSync(SOURCE)
  await refresh.click()
  const assetOffline = page.locator(`[data-cut-asset-offline="${assetId}"]`)
  const timelineOffline = page.locator(`[data-cut-offline-asset="${assetId}"]`).first()
  const previewOffline = page.locator(`[data-cut-preview-offline="${assetId}"][data-cut-preview-offline-kind="base"]`)
  await Promise.all([
    assetOffline.waitFor({ state: 'visible', timeout: 15_000 }),
    timelineOffline.waitFor({ state: 'visible', timeout: 15_000 }),
    previewOffline.waitFor({ state: 'visible', timeout: 15_000 }),
  ])

  const assetText = (await assetOffline.textContent()) || ''
  const timelineText = (await timelineOffline.textContent()) || ''
  const previewText = (await previewOffline.textContent()) || ''
  check('Assets labels the missing source and exposes Relink',
    assetText.includes('offline')
      && await page.locator(`[data-cut-asset-relink="${assetId}"]`).isVisible(),
    assetText.trim().replace(/\s+/g, ' '))
  check('Timeline replaces media decoration with a labelled recovery action',
    timelineText.includes('Source missing')
      && await page.locator(`[data-cut-timeline-relink="${assetId}"]`).first().isVisible()
      && await timelineOffline.locator('[data-cut-clip-film], canvas').count() === 0,
    timelineText.trim().replace(/\s+/g, ' '))
  check('Preview replaces the broken frame with a labelled recovery action',
    previewText.includes('Source file is offline')
      && await page.locator(`[data-cut-preview-relink="${assetId}"]`).isVisible()
      && await page.locator('[data-cut-poster]').count() === 0,
    previewText.trim().replace(/\s+/g, ' '))
  check('offline UI is path-light',
    previewText.includes(basename(SOURCE))
      && !previewText.includes(TMP)
      && !timelineText.includes(TMP),
    `label=${basename(SOURCE)} tempPathHidden=${!previewText.includes(TMP) && !timelineText.includes(TMP)}`)
  await screenshot(page, 'offline-media-visible')

  const relinked = await verb('media.relink', {
    asset: assetId,
    path: REPLACEMENT,
    rationale: 'offline-media verifier relink',
  })
  check('media.relink accepts a same-content moved source', relinked.ok === true,
    relinked.ok ? basename(REPLACEMENT) : JSON.stringify(relinked.error))
  if (!relinked.ok) throw new Error('cannot verify recovery without a successful relink')
  await refresh.click()
  await Promise.all([
    assetOffline.waitFor({ state: 'detached', timeout: 15_000 }),
    timelineOffline.waitFor({ state: 'detached', timeout: 15_000 }),
    previewOffline.waitFor({ state: 'detached', timeout: 15_000 }),
  ])
  await page.locator('[data-cut-preview-surface]:not([data-cut-preview-surface="offline"])')
    .waitFor({ state: 'visible', timeout: 15_000 })
  check('relink clears every shared offline surface', true, 'Assets, Timeline, and Preview recovered')
  check('offline handling raises no page exceptions', pageErrors.length === 0,
    pageErrors.length ? pageErrors.join(' | ') : 'none')
  await screenshot(page, 'offline-media-relinked')

  if (EVIDENCE_DIR) {
    writeFileSync(join(EVIDENCE_DIR, 'receipt.json'), JSON.stringify({
      schema: 'shellx-cut/offline-media-verifier@1',
      app: APP,
      cutd: CUTD,
      fixture: basename(FIXTURE),
      sourceLabel: basename(SOURCE),
      replacementLabel: basename(REPLACEMENT),
      assetId,
      clipId: clip?.id || null,
      checks,
    }, null, 2))
  }
} finally {
  await browser?.close().catch(() => {})
  await verb('project.close').catch(() => {})
  await verb('project.forget', { path: PROJECT }).catch(() => {})
  rmSync(TMP, { recursive: true, force: true })
}

if (checks.some((entry) => !entry.pass)) process.exitCode = 1
