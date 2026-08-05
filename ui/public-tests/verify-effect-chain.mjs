import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '../..')
const CLIP = resolve(REPO, 'testdata/insert_clip.mp4')
const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6208'
const APP = process.env.SWEEP_APP || 'http://127.0.0.1:5208'
const temp = mkdtempSync(join(tmpdir(), 'cut-effect-chain-'))
const projectDir = join(temp, 'effect-chain-rig.cutproj')
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

function findClip(project, clipId) {
  return project.tracks.flatMap((track) => track.clips).find((clip) => clip.id === clipId)
}

async function waitFor(predicate, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const project = await projectState()
    if (predicate(project)) return project
    await new Promise((resolveWait) => setTimeout(resolveWait, 80))
  }
  throw new Error('timed out waiting for project state')
}

let browser
try {
  const created = await verb('project.create', {
    name: 'effect-chain-rig',
    dir: projectDir,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  if (!created.ok) throw new Error(created.error?.message || 'project.create failed')
  const imported = await verb('media.import', { path: CLIP, proxy: false })
  if (!imported.ok) throw new Error(imported.error?.message || 'media.import failed')

  await new Promise((resolveWait) => setTimeout(resolveWait, 1_500))
  const initial = await projectState()
  const audioClip = initial.tracks.find((track) => track.kind === 'audio')?.clips.find((clip) => clip.asset)?.id
  const videoClip = initial.tracks.find((track) => track.kind === 'video')?.clips.find((clip) => clip.asset)?.id
  if (!audioClip || !videoClip) throw new Error('import did not create linked video/audio clips')

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  await page.waitForTimeout(700)
  await page.locator(`[data-cut-clip="${audioClip}"]`).click()
  await page.locator('[data-cut-action="expand-rail"]').click().catch(() => {})
  await page.locator('[data-cut-right-tab="properties"]').click()
  await page.locator('[data-cut-section-toggle="audio-effects"]').click()
  await page.locator('[data-cut-effect-chain="audio"]').waitFor()

  await verb('edit.effect', { clip: audioClip, effects: [] })
  await page.waitForTimeout(250)
  const requests = []
  const onRequest = (request) => {
    if (request.url().includes('/api/verb/edit.effect')) requests.push(request.postDataJSON())
  }
  page.on('request', onRequest)
  await page.locator('[data-cut-inspector-audio-effect="denoise"]').click()
  await page.locator('[data-cut-inspector-audio-effect="compressor"]').click()
  await waitFor((project) => findClip(project, audioClip)?.effects?.length === 2)
  page.off('request', onRequest)
  const rapidTypes = findClip(await projectState(), audioClip).effects.map((effect) => effect.type)
  check(
    'rapid additions serialize',
    JSON.stringify(rapidTypes) === JSON.stringify(['denoise', 'compressor'])
      && requests[1]?.effects?.length === 2,
    `requests=${requests.length} final=${rapidTypes.join('>')}`,
  )

  const visibleTypes = await page.locator('[data-cut-effect-chain-item]').evaluateAll((rows) =>
    rows.map((row) => row.getAttribute('data-cut-effect-chain-item')),
  )
  check(
    'chain shows order and parameters',
    JSON.stringify(visibleTypes) === JSON.stringify(['denoise', 'compressor'])
      && (await page.locator('[data-cut-effect-chain-item="compressor"]').textContent()).includes('amount 0.5'),
    `visible=${visibleTypes.join('>')}`,
  )

  await page.getByRole('button', { name: 'Move Compress up' }).click()
  await waitFor((project) => findClip(project, audioClip)?.effects?.[0]?.type === 'compressor')
  await page.getByRole('button', { name: 'Remove Denoise' }).click()
  await waitFor((project) => {
    const effects = findClip(project, audioClip)?.effects
    return effects?.length === 1 && effects[0]?.type === 'compressor'
  })
  check('chain reorder and remove persist', true, 'compressor moved first; denoise removed')

  await page.route('**/api/verb/edit.effect', (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({
      ok: false,
      error: { code: 'render_failed', message: 'Effect service unavailable', cause: 'rig' },
    }),
  }))
  await page.locator('[data-cut-inspector-audio-effect="gate"]').click()
  await page.locator('[data-cut-effect-chain-error]').waitFor()
  const failedTypes = await page.locator('[data-cut-effect-chain-item]').evaluateAll((rows) =>
    rows.map((row) => row.getAttribute('data-cut-effect-chain-item')),
  )
  check(
    'failed update rolls back visibly',
    JSON.stringify(failedTypes) === JSON.stringify(['compressor'])
      && (await page.locator('[data-cut-effect-chain-error]').textContent()) === 'Effect service unavailable',
    `visible=${failedTypes.join('>')}`,
  )
  await page.unroute('**/api/verb/edit.effect')

  await page.locator('[data-cut-rail-close]').click()
  await page.locator(`[data-cut-clip="${videoClip}"]`).click()
  await page.locator('[data-cut-action="expand-rail"]').click()
  await page.locator('[data-cut-right-tab="properties"]').click()
  await page.getByText(`Video clip · ${videoClip}`, { exact: true }).waitFor()
  await page.locator('[data-cut-section-toggle="video-effects"]').click()
  await page.locator('[data-cut-inspector-effect="invert"]').click()
  await waitFor((project) => findClip(project, videoClip)?.effects?.some((effect) => effect.type === 'invert'))
  await page.locator('[data-cut-effect-chain-item="invert"]').waitFor({ state: 'visible' })
  const layout = await page.evaluate(() => ({
    rootOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
    rowOverflow: [...document.querySelectorAll('[data-cut-effect-chain-item]')]
      .some((row) => row.scrollWidth > row.clientWidth),
  }))
  check(
    'video chain applies and fits minimum window',
    await page.locator('[data-cut-effect-chain-item="invert"]').isVisible()
      && (await page.locator('[data-cut-composed]').getAttribute('data-cut-composed')) === 'true'
      && layout.rootOverflow === 0
      && !layout.rowOverflow,
    `composed=${await page.locator('[data-cut-composed]').getAttribute('data-cut-composed')} rootOverflow=${layout.rootOverflow} rowOverflow=${layout.rowOverflow}`,
  )
} finally {
  await browser?.close().catch(() => {})
  await verb('project.close').catch(() => {})
  await verb('project.forget', { path: projectDir }).catch(() => {})
  rmSync(temp, { recursive: true, force: true })
}

if (checks.some((entry) => !entry.pass)) process.exitCode = 1
