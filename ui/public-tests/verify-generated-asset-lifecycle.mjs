import { chromium } from 'playwright'
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6215'
const APP = process.env.SWEEP_APP || CUTD
const TMP = mkdtempSync(join(tmpdir(), 'cut-generated-lifecycle-'))
const PROJECT = join(TMP, 'generated-lifecycle.cutproj')
const COUNT = process.env.CUTD_GENERATE_FIXTURE_COUNT
const SCREENSHOT = process.env.GENERATED_LIFECYCLE_SCREENSHOT
const CHROMIUM_EXECUTABLE = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:generated-lifecycle' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(90000),
  })
  return response.json()
}

function isVerbResponse(candidate, name) {
  return new URL(candidate.url()).pathname.endsWith(`/api/verb/${name}`)
}

async function generate(page) {
  await page.locator('[data-cut-generate-run]').click()
  await page.locator('[data-cut-generate-run][data-cut-generate-armed]').waitFor()
  const response = page.waitForResponse((candidate) => isVerbResponse(candidate, 'assets.generate'), { timeout: 90000 })
  await page.locator('[data-cut-generate-run]').click()
  const queued = await (await response).json()
  const jobId = queued.result?.job_id
  if (!queued.ok || !jobId) throw new Error(`generation was not queued: ${JSON.stringify(queued)}`)
  for (let attempt = 0; attempt < 300; attempt += 1) {
    const status = await verb('jobs.status', { job_id: jobId })
    if (status.result?.state === 'done' || status.result?.state === 'failed') return status.result
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`generation job ${jobId} did not finish`)
}

function invocationCount() {
  return COUNT && existsSync(COUNT) ? readFileSync(COUNT, 'utf8').trim().split(/\r?\n/).filter(Boolean).length : null
}

let browser
try {
  const created = await verb('project.create', { name: 'generated-lifecycle', dir: PROJECT })
  if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created.error)}`)

  browser = await chromium.launch(CHROMIUM_EXECUTABLE ? { executablePath: CHROMIUM_EXECUTABLE } : {})
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  const browserErrors = []
  page.on('console', (message) => { if (message.type() === 'error') browserErrors.push(message.text()) })
  page.on('pageerror', (error) => browserErrors.push(error.message))
  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  await page.locator('[data-cut-left-tab="assets"]').click()
  await page.locator('[data-cut-action="generate-asset"]').click()
  await page.locator('[data-cut-generate-prompt]').fill('an immutable blue release card')

  const first = await generate(page)
  if (first.state !== 'done' || first.result?.ok !== true || first.result?.generated?.reused !== false) throw new Error(`first generation failed: ${JSON.stringify(first)}`)
  await page.locator('[data-cut-generate-note]').waitFor()
  const firstNote = await page.locator('[data-cut-generate-note]').textContent()

  const second = await generate(page)
  if (second.state !== 'done' || second.result?.generated?.reused !== true) throw new Error(`generation reuse failed: ${JSON.stringify(second)}`)
  await page.waitForFunction(() => document.querySelector('[data-cut-generate-note]')?.textContent?.startsWith('Reused'))
  const secondNote = await page.locator('[data-cut-generate-note]').textContent()

  const firstResult = first.result
  const secondResult = second.result
  const provenance = JSON.parse(readFileSync(firstResult.generated.provenance_path, 'utf8'))
  const reuseInvocations = invocationCount()
  const firstHistoryItem = page.locator(`[data-cut-generated-asset="${firstResult.asset_id}"]`).first()
  await firstHistoryItem.waitFor({ state: 'visible', timeout: 15000 })
  const firstIntegrity = await firstHistoryItem.getAttribute('data-cut-generated-integrity')
  await firstHistoryItem.locator('[data-cut-generated-use-reference]').click()
  const referenceCount = await page.locator('[data-cut-generate-reference-count]').textContent()

  await page.locator('[data-cut-generate-prompt]').fill('a green release card guided by the selected reference')
  const referenced = await generate(page)
  if (referenced.state !== 'done' || referenced.result?.ok !== true) throw new Error(`referenced generation failed: ${JSON.stringify(referenced)}`)
  await page.waitForFunction(() => document.querySelector('[data-cut-generate-note]')?.textContent?.startsWith('Generated'))
  const referencedResult = referenced.result
  const referencedHistoryItem = page.locator(`[data-cut-generated-asset="${referencedResult.asset_id}"]`).first()
  await referencedHistoryItem.waitFor({ state: 'visible', timeout: 15000 })
  await referencedHistoryItem.locator('[data-cut-generated-variation]').click()
  const preparedVariation = await page.locator('[data-cut-generate-variation]').getAttribute('data-cut-generate-variation')
  const preparedReferenceCount = await page.locator('[data-cut-generate-reference-count]').textContent()

  const variation = await generate(page)
  if (variation.state !== 'done' || variation.result?.ok !== true) throw new Error(`variation generation failed: ${JSON.stringify(variation)}`)
  await page.waitForFunction(() => document.querySelector('[data-cut-generate-note]')?.textContent?.startsWith('Generated'))
  const variationResult = variation.result
  const variationHistoryItem = page.locator(`[data-cut-generated-asset="${variationResult.asset_id}"]`).first()
  await variationHistoryItem.waitFor({ state: 'visible', timeout: 15000 })

  await referencedHistoryItem.locator('[data-cut-generated-compare-select]').click()
  await variationHistoryItem.locator('[data-cut-generated-compare-select]').click()
  await page.locator('[data-cut-generated-compare]').click()
  await page.locator('[data-cut-generated-compare-dialog]').waitFor({ state: 'visible' })
  const comparedTakes = await page.locator('[data-cut-generated-compare-take]').count()
  if (SCREENSHOT) {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.screenshot({ path: SCREENSHOT.replace(/\.png$/i, '-compare.png') })
  }
  await page.locator(`[data-cut-generated-choose="${variationResult.asset_id}"]`).click()
  const chosenVariation = await variationHistoryItem.getAttribute('data-cut-generated-chosen')

  const insertResponse = page.waitForResponse((candidate) => isVerbResponse(candidate, 'edit.insert'), { timeout: 30000 })
  await variationHistoryItem.locator('[data-cut-generated-insert]').click()
  const inserted = await (await insertResponse).json()
  const insertedClip = inserted.result?.clip_id
  if (!inserted.ok || !insertedClip) throw new Error(`history insert failed: ${JSON.stringify(inserted)}`)
  const insertedState = await verb('project.state')
  const insertedLanded = insertedState.result?.tracks?.some((track) => track.clips?.some((clip) => clip.id === insertedClip && clip.asset === variationResult.asset_id))
  await verb('ui.select', { clip_ids: [insertedClip] })
  await page.waitForFunction((assetId) => {
    const button = document.querySelector(`[data-cut-generated-asset="${assetId}"] [data-cut-generated-replace]`)
    return button instanceof HTMLButtonElement && !button.disabled
  }, referencedResult.asset_id)
  const replaceResponse = page.waitForResponse((candidate) => isVerbResponse(candidate, 'edit.replace'), { timeout: 30000 })
  await referencedHistoryItem.locator('[data-cut-generated-replace]').click()
  const replaced = await (await replaceResponse).json()
  const replacedState = await verb('project.state')
  const replacePreserved = replaced.ok
    && replaced.result?.target_clip === insertedClip
    && replacedState.result?.tracks?.some((track) => track.clips?.some((clip) => clip.id === insertedClip && clip.asset === referencedResult.asset_id))

  await page.locator('[data-cut-generate-placement-mode="insert"]').click()
  await page.locator('[data-cut-generate-prompt]').fill('a placed amber release card')
  await page.locator('[data-cut-generate-run]').click()
  await page.locator('[data-cut-generate-run][data-cut-generate-armed]').waitFor()
  const placementResponse = page.waitForResponse((candidate) => isVerbResponse(candidate, 'assets.generate'), { timeout: 90000 })
  await page.locator('[data-cut-generate-run]').click()
  const placementQueued = await (await placementResponse).json()
  const placementJobId = placementQueued.result?.job_id
  const placementTarget = placementQueued.result?.placement?.target_clip
  if (!placementQueued.ok || !placementJobId || !placementTarget) throw new Error(`placed generation was not queued: ${JSON.stringify(placementQueued)}`)
  const pendingState = await verb('project.state')
  const pendingClip = pendingState.result?.tracks?.flatMap((track) => track.clips || []).find((clip) => clip.id === placementTarget)
  const pendingPath = pendingState.result?.assets?.[pendingClip?.asset]?.path
  const pendingVisible = Boolean(pendingClip && pendingPath?.includes(`${join('assets', 'placeholders')}`) && existsSync(pendingPath))
  const placed = await (async () => {
    for (let attempt = 0; attempt < 300; attempt += 1) {
      const status = await verb('jobs.status', { job_id: placementJobId })
      if (status.result?.state === 'done' || status.result?.state === 'failed') return status.result
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
    throw new Error(`placement generation job ${placementJobId} did not finish`)
  })()
  const placedState = await verb('project.state')
  const placedClip = placedState.result?.tracks?.flatMap((track) => track.clips || []).find((clip) => clip.id === placementTarget)
  const placedInSameSlot = placed.state === 'done'
    && placed.result?.placement?.state === 'applied'
    && placedClip?.asset === placed.result?.asset_id
    && !existsSync(pendingPath)

  const history = await verb('assets.generated_list', { limit: 10 })
  const historyText = JSON.stringify(history.result ?? {})
  if (SCREENSHOT) {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.locator('[data-cut-generate-placement]').scrollIntoViewIfNeeded()
    await page.screenshot({ path: SCREENSHOT, fullPage: true })
  }

  const state = await verb('project.state')
  const generatedAsset = state.result?.assets?.[firstResult.asset_id]
  const runsDir = join(PROJECT, 'cache', 'gen', 'runs')

  await page.locator('[data-cut-generate-prompt]').fill('a cancelled red release card')
  await page.locator('[data-cut-generate-run]').click()
  await page.locator('[data-cut-generate-run][data-cut-generate-armed]').waitFor()
  const cancelResponse = page.waitForResponse((candidate) => isVerbResponse(candidate, 'assets.generate'), { timeout: 90000 })
  await page.locator('[data-cut-generate-run]').click()
  const cancelQueued = await (await cancelResponse).json()
  const cancelJobId = cancelQueued.result?.job_id
  const cancelGenerationId = cancelQueued.result?.generation_id
  const cancelTarget = cancelQueued.result?.placement?.target_clip
  if (!cancelQueued.ok || !cancelJobId || !cancelGenerationId || !cancelTarget) throw new Error(`cancel generation was not queued: ${JSON.stringify(cancelQueued)}`)
  await page.locator(`[data-cut-generate-job-cancel="${cancelJobId}"]`).waitFor()
  for (let attempt = 0; attempt < 100 && invocationCount() !== 5; attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  await page.locator(`[data-cut-generate-job-cancel="${cancelJobId}"]`).click()
  await page.waitForFunction(() => document.querySelector('[data-cut-generate-error]')?.textContent?.includes('cancelled'))
  const cancelled = await verb('jobs.status', { job_id: cancelJobId })
  const cancelledSource = join(PROJECT, 'assets', 'generated', `${cancelGenerationId}.png`)
  const cancelledState = await verb('project.state')
  const cancelledClip = cancelledState.result?.tracks?.flatMap((track) => track.clips || []).find((clip) => clip.id === cancelTarget)
  const cancelledPlaceholderPath = cancelledState.result?.assets?.[cancelledClip?.asset]?.path

  const checks = {
    same_asset: firstResult.asset_id === secondResult.asset_id,
    one_provider_run: reuseInvocations === 1,
    immutable_source: generatedAsset?.path === join(PROJECT, 'assets', 'generated', `${firstResult.generated.generation_id}.png`),
    provenance: provenance.schema === 'shellx-cut/generated-asset/2' && provenance.generation_id === firstResult.generated.generation_id && provenance.prompt === 'an immutable blue release card' && String(provenance.content_hash || '').startsWith('sha256:'),
    history_verified: firstIntegrity === 'verified' && history.ok && history.result?.total === 4 && history.result?.verified === 4,
    history_path_light: !historyText.includes(PROJECT) && !historyText.includes('provenance_path'),
    reference_visible: referenceCount === '1/4' && referencedResult.generated.references?.[0]?.asset_id === firstResult.asset_id,
    variation_prepared: Boolean(preparedVariation) && preparedReferenceCount === '1/4',
    variation_family: variationResult.generated.family_id === referencedResult.generated.family_id && variationResult.generated.generation_id !== referencedResult.generated.generation_id && variationResult.generated.variation === preparedVariation,
    variation_reference: variationResult.generated.references?.[0]?.asset_id === firstResult.asset_id,
    compare_same_family: comparedTakes === 2 && chosenVariation === 'true',
    history_insert: insertedLanded === true,
    history_replace: replacePreserved === true,
    pending_visible: pendingVisible,
    pending_replaced: placedInSameSlot,
    scratch_clean: !existsSync(runsDir) || readdirSync(runsDir).length === 0,
    cost_honest: firstResult.generated.cost_usd === null && firstNote?.includes('price is not reported'),
    reuse_visible: secondNote?.startsWith('Reused') && secondNote.includes('provider CLI was not run'),
    cancel_terminal: cancelled.result?.state === 'failed' && cancelled.result?.error?.code === 'job_cancelled',
    cancel_no_asset: invocationCount() === 5 && !existsSync(cancelledSource),
    cancel_slot_retained: cancelledClip?.id === cancelTarget && Boolean(cancelledPlaceholderPath && existsSync(cancelledPlaceholderPath)),
    retry_visible: (await page.locator(`[data-cut-generate-retry="${cancelTarget}"]`).count()) === 1,
    cancel_visible: (await page.locator('[data-cut-generate-error]').textContent())?.includes('cancelled'),
    no_browser_errors: browserErrors.length === 0,
  }
  for (const [name, pass] of Object.entries(checks)) console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}`)
  if (Object.values(checks).some((pass) => !pass)) throw new Error(`generated lifecycle checks failed: ${JSON.stringify({ checks, firstNote, secondNote, browserErrors })}`)
} finally {
  await browser?.close()
  rmSync(TMP, { recursive: true, force: true })
}
