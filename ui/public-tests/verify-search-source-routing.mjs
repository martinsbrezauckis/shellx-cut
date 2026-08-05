import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '../..')
const BASE_CLIP = resolve(REPO, 'testdata/talking_head.mp4')
const USED_CLIP = resolve(REPO, 'testdata/insert_clip.mp4')
const UNUSED_CLIP = resolve(REPO, 'testdata/silent_screen.mp4')
const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6213'
const APP = process.env.SWEEP_APP || CUTD
const temp = mkdtempSync(join(tmpdir(), 'cut-search-routing-'))
const projectDir = join(temp, 'search-routing-rig.cutproj')
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

async function state() {
  const envelope = await verb('project.state')
  if (!envelope.ok) throw new Error(envelope.error?.message || 'project.state failed')
  return envelope.result
}

async function waitForProject(predicate, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const project = await state()
    if (predicate(project)) return project
    await new Promise((resolveWait) => setTimeout(resolveWait, 120))
  }
  throw new Error('timed out waiting for project state')
}

async function importAsset(path) {
  const imported = await verb('media.import', { path, proxy: false })
  if (!imported.ok) throw new Error(imported.error?.message || `media.import failed: ${path}`)
  const project = await waitForProject((candidate) => Object.values(candidate.assets || {}).some((asset) =>
    asset.path === path && (asset.probe?.duration_ms || 0) > 0,
  ))
  const assetId = Object.entries(project.assets).find(([, asset]) => asset.path === path)?.[0]
  if (!assetId) throw new Error(`asset was not registered: ${path}`)
  return assetId
}

let browser
try {
  const created = await verb('project.create', {
    name: 'search-routing-rig',
    dir: projectDir,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  if (!created.ok) throw new Error(created.error?.message || 'project.create failed')

  await importAsset(BASE_CLIP)
  await waitForProject((project) => project.tracks.some((track) => track.kind === 'audio' && track.clips?.some((clip) => clip.asset)))
  const usedAsset = await importAsset(USED_CLIP)
  const unusedAsset = await importAsset(UNUSED_CLIP)
  const beforeInsert = await state()
  const videoTrack = beforeInsert.tracks.find((track) => track.kind === 'video')?.id
  const audioTrack = beforeInsert.tracks.find((track) => track.kind === 'audio')?.id
  if (!videoTrack || !audioTrack) throw new Error('base video/audio tracks were not created')
  const videoInsert = await verb('edit.insert', {
    asset: usedAsset,
    track: videoTrack,
    at_ms: 12_000,
    src_range_ms: [2000, 5500],
    ripple: true,
    rationale: 'search-routing rig video',
  })
  const audioInsert = await verb('edit.insert', {
    asset: usedAsset,
    track: audioTrack,
    at_ms: 12_000,
    src_range_ms: [2000, 5500],
    ripple: false,
    rationale: 'search-routing rig linked audio',
  })
  if (!videoInsert.ok || !audioInsert.ok) throw new Error('could not seed aligned ranged clips')

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  let searchArgs = null
  await page.route('**/api/verb/media.index_status', (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({
      ok: true,
      result: {
        count: 2,
        assets: [
          { asset: usedAsset, indexed_frames: 10, dim: 4, model: 'rig' },
          { asset: unusedAsset, indexed_frames: 4, dim: 4, model: 'rig' },
        ],
      },
      warnings: [],
    }),
  }))
  await page.route('**/api/verb/media.search', async (route) => {
    searchArgs = route.request().postDataJSON()
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        ok: true,
        result: {
          count: 2,
          hits: [
            { asset: usedAsset, start_ms: 2500, end_ms: 3500, peak_ms: 3000, score: 0.91 },
            { asset: unusedAsset, start_ms: 500, end_ms: 1500, peak_ms: 1000, score: 0.82 },
          ],
        },
        warnings: [],
      }),
    })
  })

  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
  await page.locator('[data-cut-left-tab="find"]').click()
  await page.locator('[data-cut-find-tab="find-moment"]').click()
  await page.locator('[data-cut-search-query]').fill('red scene')
  await page.locator('[data-cut-search-go]').click()
  await page.locator('[data-cut-search-hit="1"]').waitFor()
  check('Find moment sends the requested text query', searchArgs?.query === 'red scene', `query=${searchArgs?.query}`)

  const usedText = await page.locator('[data-cut-search-hit="0"]').textContent()
  const unusedText = await page.locator('[data-cut-search-hit="1"]').textContent()
  check(
    'search results distinguish source time from timeline placement',
    usedText.includes('insert_clip.mp4') && usedText.includes('source 3.0s') && usedText.includes('timeline 13.0s')
      && unusedText.includes('silent_screen.mp4') && unusedText.includes('not on timeline'),
    `used=${usedText.replace(/\s+/g, ' ').trim()} unused=${unusedText.replace(/\s+/g, ' ').trim()}`,
  )

  let playheadArgs = null
  const onRequest = (request) => {
    if (request.url().includes('/api/verb/ui.playhead')) playheadArgs = request.postDataJSON()
  }
  page.on('request', onRequest)
  const jumpButton = page.locator('[data-cut-search-jump="0"]')
  await jumpButton.click({ timeout: 3000 })
  await page.waitForTimeout(150)
  page.off('request', onRequest)
  check('timeline action jumps to the mapped occurrence', playheadArgs?.at_ms === 13_000, `at_ms=${playheadArgs?.at_ms}`)

  const unusedTimelineAction = await page.locator('[data-cut-search-jump="1"]').count()
  await page.locator('[data-cut-search-source="1"]').click()
  const sourceDialog = page.locator(`[data-cut-source-monitor="${unusedAsset}"]`)
  await sourceDialog.waitFor()
  await page.locator('[data-cut-source-current]').filter({ hasText: '0:01.000' }).waitFor()
  check(
    'unused hit opens its exact source moment instead of a false timeline jump',
    unusedTimelineAction === 0 && await sourceDialog.isVisible(),
    `timelineActions=${unusedTimelineAction} sourceCurrent=${await page.locator('[data-cut-source-current]').textContent()}`,
  )

  const layout = await page.evaluate(() => ({
    rootOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
    resultOverflow: [...document.querySelectorAll('[data-cut-search-hit]')]
      .some((row) => row.scrollWidth > row.clientWidth),
    dialogOverflow: [...document.querySelectorAll('[data-cut-source-monitor]')]
      .some((dialog) => dialog.scrollWidth > dialog.clientWidth),
  }))
  check(
    'search routing fits the supported minimum window',
    layout.rootOverflow === 0 && !layout.resultOverflow && !layout.dialogOverflow,
    `rootOverflow=${layout.rootOverflow} resultOverflow=${layout.resultOverflow} dialogOverflow=${layout.dialogOverflow}`,
  )
  await page.keyboard.press('Escape')
} finally {
  await browser?.close().catch(() => {})
  await verb('project.close').catch(() => {})
  await verb('project.forget', { path: projectDir }).catch(() => {})
  rmSync(temp, { recursive: true, force: true })
}

if (checks.some((entry) => !entry.pass)) process.exitCode = 1
