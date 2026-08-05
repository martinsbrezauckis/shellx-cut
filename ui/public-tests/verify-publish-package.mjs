import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6216'
const APP = process.env.SWEEP_APP || CUTD
const TMP = mkdtempSync(join(tmpdir(), 'cut-publish-package-'))
const PROJECT = join(TMP, 'publish-package.cutproj')
const MEDIA = join(TMP, 'source.mp4')

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:publish-package' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(120000),
  })
  return response.json()
}

async function waitJob(jobId) {
  for (let attempt = 0; attempt < 600; attempt += 1) {
    const status = await verb('jobs.status', { job_id: jobId })
    if (status.result?.state === 'done' || status.result?.state === 'failed') return status.result
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error(`job ${jobId} did not finish`)
}

function hashFile(path) {
  return `sha256:${createHash('sha256').update(readFileSync(path)).digest('hex')}`
}

let browser
try {
  const ffmpeg = process.env.FFMPEG_BIN || process.env.SHELLX_CUT_FFMPEG || 'ffmpeg'
  execFileSync(ffmpeg, [
    '-nostats', '-hide_banner', '-loglevel', 'error', '-y',
    '-f', 'lavfi', '-i', 'color=c=0x2563eb:s=320x180:r=30',
    '-t', '2', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', MEDIA,
  ])

  const created = await verb('project.create', {
    name: 'publish-package',
    dir: PROJECT,
    settings: { width: 320, height: 180, fps: 30 },
  })
  if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created.error)}`)
  const imported = await verb('media.import', { path: MEDIA, proxy: false })
  if (!imported.ok || !imported.result?.asset_id) throw new Error(`media.import failed: ${JSON.stringify(imported)}`)
  if (imported.result.job_id) await waitJob(imported.result.job_id)
  const state = await verb('project.state')
  const videoTrack = state.result?.tracks?.find((track) => track.kind === 'video')?.id
  if (!videoTrack) throw new Error('project has no video track')
  const inserted = await verb('edit.insert', { asset: imported.result.asset_id, track: videoTrack, at_ms: 0 })
  if (!inserted.ok) throw new Error(`edit.insert failed: ${JSON.stringify(inserted.error)}`)
  const savedBrand = await verb('project.brand', {
    brand: { colors: ['#ffffff'], aspect: '16:9' },
    rationale: 'focused stored-brand package verification',
  })
  if (!savedBrand.ok || savedBrand.result?.brand?.aspect !== '16:9') throw new Error(`project.brand failed: ${JSON.stringify(savedBrand)}`)
  const ops = await verb('project.ops')
  const sourceOp = ops.result?.ops?.at(-1)?.op_id

  const queued = await verb('render.bundle', {
    range_ms: [0, 1000],
    platforms: ['9:16'],
    preset: 'draft',
    rationale: 'focused publish-package verification',
  })
  if (!queued.ok || !queued.result?.job_id) throw new Error(`render.bundle failed: ${JSON.stringify(queued)}`)
  const job = await waitJob(queued.result.job_id)
  if (job.state !== 'done') throw new Error(`bundle job failed: ${JSON.stringify(job.error)}`)
  const result = job.result
  const manifest = JSON.parse(readFileSync(result.manifest_path, 'utf8'))
  const platform = result.platforms?.[0]
  const engineChecks = {
    terminal_status: job.completion === 'done_with_warnings' && result.status === 'blocked' && result.pass === false && result.issues?.some((issue) => issue.code === 'brand_check_failed') && result.warnings?.length > 0,
    manifest_exists: existsSync(result.manifest_path) && result.manifest_hash === hashFile(result.manifest_path),
    manifest_contract: manifest.schema === 'shellx-cut/publish-package/1' && manifest.bundle_id === result.bundle_id && manifest.source_op_id === sourceOp && manifest.status === result.status,
    video_hash: existsSync(platform?.path) && platform?.hash === hashFile(platform.path),
    thumbnail_hash: !platform?.thumb || (existsSync(platform.thumb) && platform.thumb_hash === hashFile(platform.thumb)),
    brand_bound: manifest.brand?.pass === result.brand?.pass && manifest.brand?.source === 'stored' && result.brand?.source === 'stored' && Array.isArray(manifest.brand?.platforms),
  }

  browser = await chromium.launch()
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  const browserErrors = []
  page.on('console', (message) => { if (message.type() === 'error') browserErrors.push(message.text()) })
  page.on('pageerror', (error) => browserErrors.push(error.message))
  await page.route('**/api/verb/clip.candidates', (route) => route.fulfill({
    contentType: 'application/json',
    body: JSON.stringify({
      ok: true,
      result: {
        candidates: [{
          asset: imported.result.asset_id,
          word_range: [0, 2],
          at_ms: 0,
          dur_ms: 1000,
          hook_score: 0.8,
          retention_score: 0.7,
          score: 0.76,
          reason: 'focused package UI fixture',
          transcript_excerpt: 'A focused publish package candidate.',
        }],
        count: 1,
        scoring: 'heuristic',
      },
    }),
  }))
  await page.route('**/api/verb/render.bundle', (route) => route.fulfill({
    contentType: 'application/json',
    body: JSON.stringify({ ok: true, result: { job_id: 'job_package_ui', bundle_id: result.bundle_id } }),
  }))
  await page.route('**/api/verb/jobs.status', (route) => route.fulfill({
    contentType: 'application/json',
    body: JSON.stringify({
      ok: true,
      result: {
        job_id: 'job_package_ui',
        kind: 'bundle',
        state: 'done',
        progress: 1,
        created_ts: '2026-07-11T00:00:00Z',
        updated_ts: '2026-07-11T00:00:01Z',
        result,
      },
    }),
  }))
  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  await page.locator('[data-cut-clips-btn]').click()
  await page.locator('[data-cut-clip-card]').waitFor()
  await page.locator('[data-cut-clip-make]').click()
  await page.locator('[data-cut-package-status="blocked"]').waitFor()
  const packageText = await page.locator('[data-cut-clip-bundle]').textContent()
  const manifestHref = await page.locator('[data-cut-bundle-manifest]').getAttribute('href')
  if (process.env.PUBLISH_PACKAGE_SCREENSHOT) {
    await page.screenshot({ path: process.env.PUBLISH_PACKAGE_SCREENSHOT, fullPage: true })
  }

  await page.locator('[data-cut-clips-close]').click()
  const openedReview = await verb('ui.open', { panel: 'review' })
  if (!openedReview.ok) throw new Error(`ui.open review failed: ${JSON.stringify(openedReview)}`)
  await page.locator('[data-cut-review-tab="qc"]').waitFor()
  await page.locator('[data-cut-review-tab="qc"]').click()
  await page.locator('[data-cut-qc-brand-status="saved"]').waitFor()
  await page.locator('[data-cut-qc-brand-editor]').evaluate((element) => { element.open = true })
  const storedAspect = await page.locator('[data-cut-qc-brand-aspect]').inputValue()
  await page.locator('[data-cut-qc-brand-fonts]').fill(' Inter, inter, Arial ')
  await page.locator('[data-cut-qc-brand-colors]').fill('#FFF, #000A')
  const brandSaveResponse = page.waitForResponse((response) => response.url().endsWith('/api/verb/project.brand') && response.request().method() === 'POST')
  await page.locator('[data-cut-action="qc-brand-save"]').click()
  const brandSaveEnvelope = await (await brandSaveResponse).json()
  const storedCheck = await verb('verify.brand')
  await page.locator('[data-cut-qc-card="brand"] [data-cut-qc-verdict]').waitFor()
  if (process.env.BRAND_KIT_SCREENSHOT) {
    await page.screenshot({ path: process.env.BRAND_KIT_SCREENSHOT, fullPage: true })
  }
  const rootOverflow = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth)
  const uiChecks = {
    blocked_visible: packageText?.includes('Package blocked') && packageText.includes('violate the active brand constraints'),
    manifest_visible: manifestHref?.includes('/api/export/') && manifestHref.endsWith('/manifest.json'),
    brand_editor_loaded: storedAspect === '16:9',
    brand_editor_saved: brandSaveEnvelope.ok && brandSaveEnvelope.result?.brand?.fonts?.join(',') === 'Inter,Arial' && brandSaveEnvelope.result?.brand?.colors?.join(',') === '#ffffff,#000000aa',
    stored_brand_checked: storedCheck.ok && storedCheck.result?.source === 'stored' && storedCheck.result?.brand?.aspect === '16:9',
    minimum_window: !rootOverflow,
    no_browser_errors: browserErrors.length === 0,
  }
  const checks = { ...engineChecks, ...uiChecks }
  for (const [name, pass] of Object.entries(checks)) console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}`)
  if (Object.values(checks).some((pass) => !pass)) {
    throw new Error(`publish-package checks failed: ${JSON.stringify({ checks, result, manifest })}`)
  }
} finally {
  await browser?.close()
  rmSync(TMP, { recursive: true, force: true })
}
