import { chromium } from 'playwright'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6171'
const APP = process.env.SWEEP_APP || CUTD
const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = join(HERE, '..', '..')
const CLIP = process.env.RELEASE_CLIP || join(REPO, 'testdata', 'talking_head.mp4')
const TMP = mkdtempSync(join(tmpdir(), 'cut-scopes-'))
const KEEP = process.env.KEEP_SCOPES_VERIFY === '1'
const SCREENSHOT = process.env.SCOPES_SCREENSHOT
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

let failures = 0
function check(name, ok, detail = '') {
  // eslint-disable-next-line no-console
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? ` — ${detail}` : ''}`)
  if (!ok) failures += 1
}

async function verb(name, args = {}) {
  const r = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:scopes-panel-verify' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(90000),
  })
  return r.json()
}

async function waitJob(jobId, label, timeoutMs = 180000) {
  if (!jobId) return null
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const r = await verb('jobs.status', { job_id: jobId })
    last = r.result
    if (last?.state === 'done' || last?.state === 'failed') return last
    await sleep(500)
  }
  throw new Error(`${label} job ${jobId} timed out; last=${JSON.stringify(last)}`)
}

async function captureVerbResponse(page, name, act) {
  let body
  const onResponse = async (response) => {
    if (body !== undefined || !response.url().includes(`/api/verb/${name}`)) return
    try { body = await response.json() } catch {}
  }
  page.on('response', onResponse)
  try {
    await act()
    for (let i = 0; i < 180 && body === undefined; i += 1) await sleep(250)
    return body
  } finally {
    page.off('response', onResponse)
  }
}

async function main() {
  const projectDir = join(TMP, 'scopes-panel.cutproj')
  const created = await verb('project.create', { name: 'scopes-panel', dir: projectDir })
  check('project.create', created.ok === true, created.error?.message || projectDir)
  if (!created.ok) return

  const imported = await verb('media.import', { path: CLIP, proxy: false, rationale: 'scopes panel verifier seed' })
  const asset = imported.result?.asset_id
  check('media.import', imported.ok === true && !!asset, asset || imported.error?.message || '')
  if (!imported.ok || !asset) return
  const job = await waitJob(imported.result?.job_id, 'media.import')
  check('media.import job', job?.state === 'done', job?.state || '')

  const directScopes = await verb('verify.scopes', { at_ms: 1000, scope_images: true, kinds: ['vectorscope', 'waveform', 'histogram'] })
  check('verify.scopes direct', directScopes.ok === true && directScopes.result?.luma && directScopes.result?.scopes, directScopes.error?.message || '')

  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  const errors = []
  page.on('console', (msg) => { if (msg.type() === 'error') errors.push(msg.text()) })
  page.on('pageerror', (err) => errors.push(err.message))

  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  await page.waitForSelector('[data-cut-action="expand-rail"], [data-cut-panel="review"]', { timeout: 15000 })
  const sought = await verb('ui.playhead', { at_ms: 1750 })
  check('ui.playhead scopes frame', sought.ok === true, sought.error?.message || '')
  const openedScopes = await verb('ui.open', { panel: 'scopes' })
  check('ui.open scopes', openedScopes.ok === true, openedScopes.error?.message || '')
  await page.waitForSelector('[data-cut-panel="review"]', { timeout: 10000 })
  await page.waitForSelector('[data-cut-scopes]', { timeout: 10000 })
  check('Scopes tab visible', await page.locator('[data-cut-scopes]').isVisible())
  await page.waitForFunction(() => document.querySelector('[data-cut-scopes-at-ms]')?.value === '1750')
  check('Scopes follows current playhead', await page.locator('[data-cut-scopes-at-ms]').inputValue() === '1750')

  const images = page.locator('[data-cut-scopes-images]')
  if (!(await images.isChecked())) await images.check()

  const uiScopes = await captureVerbResponse(page, 'verify.scopes', async () => {
    await page.locator('[data-cut-action="scopes-run"]').click()
  })
  await page.waitForSelector('[data-cut-scopes-result]', { timeout: 30000 })
  const resultState = await page.locator('[data-cut-scopes-result]').getAttribute('data-cut-scopes-result')
  const imageLinks = await page.locator('[data-cut-scopes-image]').count()
  await page.waitForFunction(() => Array.from(document.querySelectorAll('[data-cut-scopes-image] img')).every((img) => img.complete && img.naturalWidth > 0), null, { timeout: 30000 })
  const inlineImages = await page.locator('[data-cut-scopes-image] img').evaluateAll((images) => images.filter((img) => img.naturalWidth > 0).length)
  const warningsText = await page.locator('[data-cut-scopes-warnings]').textContent().catch(() => '')

  check('Scopes tab calls verify.scopes', uiScopes?.ok === true, uiScopes?.error?.message || '')
  check('Scopes checks current playhead', uiScopes?.result?.at_ms === 1750, `at_ms=${uiScopes?.result?.at_ms}`)
  check('Scopes result card renders', resultState === 'pass' || resultState === 'warn', `state=${resultState}`)
  check('Scope image links render', imageLinks >= 3, `links=${imageLinks}`)
  check('Scope images render inline', inlineImages >= 3, `images=${inlineImages}`)
  check('Warnings copy renders', typeof warningsText === 'string' && warningsText.length > 0, warningsText || '')
  const overflow = await page.locator('[data-cut-scopes]').evaluate((el) => ({ x: el.scrollWidth - el.clientWidth, bodyX: document.documentElement.scrollWidth - document.documentElement.clientWidth }))
  check('Scopes minimum-window overflow', overflow.x === 0 && overflow.bodyX === 0, JSON.stringify(overflow))
  check('No browser errors', errors.length === 0, errors.slice(0, 3).join(' | '))

  if (SCREENSHOT) {
    await page.locator('[data-cut-scopes-image]').first().scrollIntoViewIfNeeded()
    await page.screenshot({ path: SCREENSHOT, fullPage: true })
  }

  await browser.close()
}

try {
  await main()
} finally {
  if (!KEEP) rmSync(TMP, { recursive: true, force: true })
}

if (failures > 0) process.exit(1)
