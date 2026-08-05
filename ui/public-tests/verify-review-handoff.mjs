import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6216'
const APP = process.env.SWEEP_APP || CUTD
const TMP = mkdtempSync(join(tmpdir(), 'cut-review-handoff-'))
const PROJECT = join(TMP, 'review-handoff.cutproj')
const MEDIA = join(TMP, 'source.mp4')
const FEEDBACK = join(TMP, 'feedback.json')
const TAMPERED = join(TMP, 'feedback-tampered.json')

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:review-handoff' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(120_000),
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

function expect(name, pass, detail = '') {
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}${detail ? `  ${detail}` : ''}`)
  if (!pass) throw new Error(`${name} failed${detail ? `: ${detail}` : ''}`)
}

let browser
try {
  const ffmpeg = process.env.FFMPEG_BIN || process.env.SHELLX_CUT_FFMPEG || 'ffmpeg'
  execFileSync(ffmpeg, [
    '-nostats', '-hide_banner', '-loglevel', 'error', '-y',
    '-f', 'lavfi', '-i', 'testsrc2=s=320x180:r=30',
    '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000',
    '-t', '2', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-shortest', MEDIA,
  ])

  const created = await verb('project.create', {
    name: 'review-handoff',
    dir: PROJECT,
    settings: { width: 320, height: 180, fps: 30 },
  })
  if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created.error)}`)
  const imported = await verb('media.import', { path: MEDIA, proxy: false })
  if (!imported.ok) throw new Error(`media.import failed: ${JSON.stringify(imported.error)}`)
  if (imported.result?.job_id) await waitJob(imported.result.job_id)
  const initialState = await verb('project.state')
  const videoTrack = initialState.result?.tracks?.find((track) => track.kind === 'video')
  if (!videoTrack) throw new Error('project has no video track')
  if (!videoTrack.clips?.length) {
    const inserted = await verb('edit.insert', { asset: imported.result.asset_id, track: videoTrack.id, at_ms: 0 })
    if (!inserted.ok) throw new Error(`edit.insert failed: ${JSON.stringify(inserted.error)}`)
  }
  const contextComment = await verb('comment.add', { at_ms: 200, text: 'Existing editor context', author: 'editor' })
  if (!contextComment.ok) throw new Error(`comment.add failed: ${JSON.stringify(contextComment.error)}`)

  const rendered = await verb('render.final', {
    preset: 'draft',
    rationale: 'focused review handoff verification',
  })
  if (!rendered.ok || !rendered.result?.job_id) throw new Error(`render.final failed: ${JSON.stringify(rendered)}`)
  const renderJob = await waitJob(rendered.result.job_id)
  if (renderJob.state !== 'done') throw new Error(`render job failed: ${JSON.stringify(renderJob.error)}`)
  const outputPreference = await verb('project.set_output_dir', { dir: TMP })
  expect('Session output preference is active for the routing edge case', outputPreference.ok && outputPreference.result?.dir)

  browser = await chromium.launch()
  const appPage = await browser.newPage({ viewport: { width: 1100, height: 680 }, acceptDownloads: true })
  const browserErrors = []
  appPage.on('console', (message) => { if (message.type() === 'error') browserErrors.push(message.text()) })
  appPage.on('pageerror', (error) => browserErrors.push(error.message))
  await appPage.goto(APP, { waitUntil: 'domcontentloaded' })
  await appPage.locator('[data-cut-comments-btn]').click()
  await appPage.locator('[data-cut-panel="comments"]').waitFor()
  const exportResponse = appPage.waitForResponse((response) => response.url().endsWith('/api/verb/comment.export'))
  await appPage.locator('[data-cut-action="comment-export-review"]').click()
  const exportEnvelope = await (await exportResponse).json()
  expect('Comments control exports a package', exportEnvelope.ok, JSON.stringify(exportEnvelope.error ?? {}))
  const packageLink = appPage.locator('[data-cut-review-package]')
  await packageLink.waitFor()
  const packageHref = await packageLink.getAttribute('href')
  expect('Comments rail exposes the review page link', packageHref?.includes('/api/export/review_'), packageHref ?? 'missing')
  expect('Import feedback control is visible', await appPage.locator('[data-cut-action="comment-import-feedback"]').isVisible())

  const result = exportEnvelope.result
  expect('Default review package stays in served project exports', result.path.startsWith(join(PROJECT, 'exports')), result.path)
  const manifest = JSON.parse(readFileSync(result.manifest_path, 'utf8'))
  expect('Package files exist', existsSync(result.path) && existsSync(result.manifest_path) && existsSync(result.media_path))
  expect('Copied media matches the receipt hash', hashFile(result.media_path) === result.render_hash, `${hashFile(result.media_path)} vs ${result.render_hash}`)
  expect('Manifest binds exact render state', manifest.schema === 'shellx-cut/review-package/1' && manifest.source_op_id === result.source_op_id && manifest.render.render_id === result.render_id && manifest.render.output_hash === result.render_hash)
  expect('Manifest carries existing review context', manifest.comments.some((comment) => comment.text === 'Existing editor context'))

  const servedHtml = await fetch(new URL(packageHref, APP))
  const html = await servedHtml.text()
  expect('Review HTML is served with the correct type', servedHtml.headers.get('content-type')?.startsWith('text/html'))
  expect('Standalone reviewer has a no-network CSP', html.includes("default-src 'none'") && html.includes("connect-src 'none'"))
  const servedCsp = servedHtml.headers.get('content-security-policy') ?? ''
  expect('Served reviewer hash-pins its script', servedCsp.includes("script-src 'sha256-") && servedCsp.includes("connect-src 'none'") && !servedCsp.includes("script-src 'unsafe-inline'"), servedCsp)
  const manifestRel = result.manifest_path.split(/[/\\]exports[/\\]/).at(-1)
  const servedManifest = await fetch(`${CUTD}/api/export/${manifestRel.split(/[/\\]/).map(encodeURIComponent).join('/')}`)
  expect('Manifest is served as JSON', servedManifest.headers.get('content-type')?.startsWith('application/json'))

  const reviewPage = await browser.newPage({ viewport: { width: 1180, height: 760 }, acceptDownloads: true })
  const reviewErrors = []
  reviewPage.on('console', (message) => { if (message.type() === 'error') reviewErrors.push(message.text()) })
  reviewPage.on('pageerror', (error) => reviewErrors.push(error.message))
  await reviewPage.goto(new URL(packageHref, APP).href, { waitUntil: 'domcontentloaded' })
  await reviewPage.locator('#video').waitFor()
  await reviewPage.locator('#video').evaluate((video) => { video.currentTime = 0.45 })
  await reviewPage.locator('#note').fill('Tighten the first beat.')
  await reviewPage.locator('#author').fill('Reviewer')
  await reviewPage.locator('#add').click()
  await reviewPage.locator('#video').evaluate((video) => { video.currentTime = 1.1 })
  await reviewPage.locator('#note').fill('Hold on the result a little longer.')
  await reviewPage.locator('#add').click()
  await reviewPage.locator('#download:not([disabled])').waitFor({ timeout: 5000 }).catch(() => {})
  expect('Reviewer enables feedback download after notes', await reviewPage.locator('#download').isEnabled(), reviewErrors.join(' | '))
  const downloadPromise = reviewPage.waitForEvent('download')
  await reviewPage.locator('#download').click()
  const download = await downloadPromise
  await download.saveAs(FEEDBACK)
  const feedback = JSON.parse(readFileSync(FEEDBACK, 'utf8'))
  expect('Standalone reviewer downloads two timecoded notes', feedback.comments.length === 2 && feedback.comments[0].at_ms >= 400 && feedback.comments[1].at_ms >= 1000)
  expect('Feedback stays bound to the reviewed bytes', feedback.schema === 'shellx-cut/review-feedback/1' && feedback.source_op_id === result.source_op_id && feedback.render_id === result.render_id && feedback.render_hash === result.render_hash)
  expect('Standalone reviewer has no browser errors', reviewErrors.length === 0, reviewErrors.join(' | '))

  const beforeOps = await verb('project.ops')
  const importedFeedback = await verb('comment.import', { path: FEEDBACK, rationale: 'focused review round trip' })
  expect('Feedback import succeeds', importedFeedback.ok && importedFeedback.result?.count === 2, JSON.stringify(importedFeedback.error ?? {}))
  const afterOps = await verb('project.ops')
  expect('Feedback batch appends one atomic op', afterOps.result.ops.length === beforeOps.result.ops.length + 1 && afterOps.result.ops.at(-1)?.verb === 'comment.import')
  const comments = await verb('comment.list')
  const external = comments.result.comments.filter((comment) => comment.review_source?.render_id === result.render_id)
  expect('Imported comments preserve render provenance', external.length === 2 && external.every((comment) => comment.review_source.render_hash === result.render_hash))
  const importedRow = appPage.locator(`[data-cut-comment="${external[0].id}"]`)
  await importedRow.waitFor()
  await importedRow.locator('.cm__row-head').click()
  const sourceBadge = importedRow.locator(`[data-cut-comment-source="${result.render_id}"]`)
  await sourceBadge.waitFor()
  expect('Comments rail exposes external render provenance', (await sourceBadge.textContent())?.includes(`External · ${result.render_id}`))
  if (process.env.REVIEW_HANDOFF_APP_SCREENSHOT) {
    await appPage.screenshot({ path: process.env.REVIEW_HANDOFF_APP_SCREENSHOT, fullPage: true })
  }

  const afterReviewMetadata = await verb('comment.export')
  expect('Review metadata does not stale unchanged rendered bytes', afterReviewMetadata.ok && afterReviewMetadata.result?.stale === false, JSON.stringify(afterReviewMetadata.error ?? {}))
  const timelineChange = await verb('edit.add_marker', { at_ms: 750, label: 'post-review edit' })
  expect('Render-affecting change lands before stale rejection check', timelineChange.ok, JSON.stringify(timelineChange.error ?? {}))
  const opsAfterEdit = await verb('project.ops')

  const staleRetry = await verb('comment.import', { path: FEEDBACK })
  expect('A changed project rejects stale feedback by default', !staleRetry.ok && staleRetry.error?.code === 'conflict', JSON.stringify(staleRetry.error ?? {}))
  const opsAfterStale = await verb('project.ops')
  expect('Rejected stale import appends no op', opsAfterStale.result.ops.length === opsAfterEdit.result.ops.length)

  const tampered = { ...feedback, render_hash: 'sha256:tampered' }
  writeFileSync(TAMPERED, `${JSON.stringify(tampered, null, 2)}\n`)
  const tamperedResult = await verb('comment.import', { path: TAMPERED, allow_stale: true, rationale: 'negative verification' })
  expect('Tampered render binding is rejected', !tamperedResult.ok && tamperedResult.error?.code === 'conflict', JSON.stringify(tamperedResult.error ?? {}))

  const receiptPath = join(PROJECT, 'receipts', `${result.render_id}.json`)
  const receiptText = readFileSync(receiptPath, 'utf8')
  const tamperedReceipt = { ...JSON.parse(receiptText), output_path: MEDIA, output_hash: hashFile(MEDIA) }
  writeFileSync(receiptPath, `${JSON.stringify(tamperedReceipt, null, 2)}\n`)
  const escapedExport = await verb('comment.export', { allow_stale: true })
  writeFileSync(receiptPath, receiptText)
  expect('Tampered receipt cannot export a file outside project exports', !escapedExport.ok && escapedExport.error?.code === 'invalid_args', JSON.stringify(escapedExport.error ?? {}))

  const closed = await verb('project.close')
  if (!closed.ok) throw new Error(`project.close failed: ${JSON.stringify(closed.error)}`)
  const reopened = await verb('project.open', { path: PROJECT })
  if (!reopened.ok) throw new Error(`project.open failed: ${JSON.stringify(reopened.error)}`)
  const replayed = await verb('comment.list')
  expect('Imported batch survives project replay', replayed.result.comments.filter((comment) => comment.review_source?.render_id === result.render_id).length === 2)
  expect('App surface has no browser errors', browserErrors.length === 0, browserErrors.join(' | '))

  if (process.env.REVIEW_HANDOFF_SCREENSHOT) {
    await reviewPage.screenshot({ path: process.env.REVIEW_HANDOFF_SCREENSHOT, fullPage: true })
  }
} finally {
  await browser?.close()
  rmSync(TMP, { recursive: true, force: true })
}
