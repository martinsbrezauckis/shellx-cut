// Focused runtime gate for the task-oriented selected-clip Inspector.
// Proves default disclosure state, collapsed applied-state summaries, public
// pointer/keyboard expansion, setup routing, and audio prerequisites.

import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { chromium } from 'playwright'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6208'
const APP = process.env.SWEEP_APP || 'http://127.0.0.1:5208'
const CLIP = process.env.RELEASE_CLIP || resolve('../testdata/insert_clip.mp4')
const temp = mkdtempSync(join(tmpdir(), 'cut-inspector-discovery-'))
const projectDir = join(temp, 'inspector-discovery.cutproj')
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
  const response = await verb('project.state')
  if (!response.ok) throw new Error(response.error?.message || 'project.state failed')
  return response.result
}

function mediaClip(project, kind) {
  return project.tracks.find((track) => track.kind === kind)?.clips.find((clip) => clip.asset)
}

async function waitFor(predicate, timeoutMs = 12_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const project = await projectState()
    if (predicate(project)) return project
    await new Promise((resolveWait) => setTimeout(resolveWait, 100))
  }
  throw new Error('timed out waiting for project state')
}

async function selectProperties(page, clipId) {
  await page.locator(`[data-cut-clip="${clipId}"]`).click()
  await page.locator('[data-cut-action="expand-rail"]').click().catch(() => {})
  await page.locator('[data-cut-right-tab="properties"]').click()
  await page.getByText(new RegExp(`clip · ${clipId}$`)).waitFor()
}

async function disclosureState(page, key) {
  return page.locator(`[data-cut-section="${key}"]`).getAttribute('data-cut-section-collapsed')
}

async function expandWithKeyboard(page, key) {
  const toggle = page.locator(`[data-cut-section-toggle="${key}"]`)
  await toggle.focus()
  await page.keyboard.press('Enter')
  await page.locator(`[data-cut-section="${key}"][data-cut-section-collapsed="false"]`).waitFor()
}

let browser
try {
  const created = await verb('project.create', {
    name: 'inspector-discovery',
    dir: projectDir,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  if (!created.ok) throw new Error(created.error?.message || 'project.create failed')
  const imported = await verb('media.import', { path: CLIP, proxy: false })
  if (!imported.ok) throw new Error(imported.error?.message || 'media.import failed')
  await new Promise((resolveWait) => setTimeout(resolveWait, 1_500))

  const initial = await projectState()
  const videoClip = mediaClip(initial, 'video')?.id
  const audioClip = mediaClip(initial, 'audio')?.id
  if (!videoClip || !audioClip) throw new Error('fixture did not create linked video and audio clips')

  for (const [name, args] of [
    ['edit.stabilize', { clip: videoClip, enabled: true, rationale: 'inspector discovery state' }],
    ['edit.speed', { clip: videoClip, factor: 1.5, rationale: 'inspector discovery state' }],
    ['edit.grade', { clip: videoClip, contrast: 1.2, rationale: 'inspector discovery state' }],
    ['edit.effect', { clip: videoClip, effects: [{ type: 'invert' }], rationale: 'inspector discovery state' }],
    ['edit.redact', {
      clip: videoClip,
      shape: 'rect',
      points: [[0.2, 0.2], [0.5, 0.5]],
      mode: 'blur',
      strength: 20,
      rationale: 'inspector discovery state',
    }],
    ['edit.effect', { clip: audioClip, effects: [{ type: 'denoise', amount: 0.5 }], rationale: 'inspector discovery state' }],
    ['edit.eq', { clip: audioClip, preset: 'voice', enabled: true, rationale: 'inspector discovery state' }],
  ]) {
    const response = await verb(name, args)
    if (!response.ok) throw new Error(`${name} failed: ${response.error?.message || response.error?.code}`)
  }
  await waitFor((project) => {
    const video = project.tracks.flatMap((track) => track.clips).find((clip) => clip.id === videoClip)
    const audio = project.tracks.flatMap((track) => track.clips).find((clip) => clip.id === audioClip)
    return video?.stabilize && video?.grade && video?.mask && video?.effects?.length === 1
      && audio?.eq && audio?.effects?.length === 1
  })

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  const consoleErrors = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  await page.waitForTimeout(900)
  await selectProperties(page, videoClip)

  const videoSections = await page.locator('[data-cut-panel="inspector"] [data-cut-section]').evaluateAll(
    (sections) => sections.map((section) => section.getAttribute('data-cut-section')),
  )
  check(
    'video tasks replace broad blob',
    !videoSections.includes('video-tools')
      && ['transform', 'speed', 'video-motion', 'video-color', 'video-effects', 'video-privacy']
        .every((key) => videoSections.includes(key)),
    `sections=${videoSections.join(',')}`,
  )
  check(
    'common motion tools start visible',
    await disclosureState(page, 'video-motion') === 'false'
      && await page.locator('[data-cut-inspector-action="stabilize"]').isVisible()
      && await page.locator('[data-cut-action="auto-zoom"]').isVisible(),
    'stabilization and auto zoom are immediately visible',
  )
  check(
    'specialist video tasks start collapsed',
    await disclosureState(page, 'video-color') === 'true'
      && await disclosureState(page, 'video-effects') === 'true'
      && await disclosureState(page, 'video-privacy') === 'true'
      && await page.locator('[data-cut-action="auto-balance"]').count() === 0
      && await page.locator('[data-cut-inspector-effect="invert"]').count() === 0
      && await page.locator('[data-cut-action="redact-draw"]').count() === 0,
    'specialist bodies are one labelled expansion away',
  )

  const videoSummaries = await page.locator('[data-cut-section-summary]').evaluateAll((nodes) =>
    Object.fromEntries(nodes.map((node) => [
      node.getAttribute('data-cut-section-summary'),
      {
        text: node.textContent?.trim(),
        tone: node.getAttribute('data-cut-section-summary-tone'),
      },
    ])),
  )
  check(
    'collapsed video summaries expose applied state',
    videoSummaries.speed?.text?.includes('1.5×')
      && videoSummaries['video-motion']?.text?.includes('Stabilized')
      && videoSummaries['video-color']?.text?.includes('Grade applied')
      && videoSummaries['video-effects']?.text?.includes('1 effect')
      && videoSummaries['video-privacy']?.text?.includes('Redaction applied')
      && ['speed', 'video-motion', 'video-color', 'video-effects', 'video-privacy']
        .every((key) => videoSummaries[key]?.tone === 'active'),
    JSON.stringify(videoSummaries),
  )

  await expandWithKeyboard(page, 'video-color')
  await page.locator('[data-cut-action="auto-balance"]').waitFor()
  await page.locator('[data-cut-section-toggle="video-effects"]').click()
  await page.locator('[data-cut-inspector-effect="invert"]').waitFor()
  await page.locator('[data-cut-section-toggle="video-privacy"]').click()
  await page.locator('[data-cut-action="redact-draw"]').waitFor()
  check('specialist video tasks expand through public controls', true, 'Color via Enter; Effects and Privacy via pointer clicks')
  check(
    'reset and undo remain visible',
    await page.locator('[data-cut-section-reset="speed"]').isVisible()
      && await page.locator('[data-cut-section-reset="transform"]').isVisible(),
    'section reset actions remain in collapsed headers',
  )

  await selectProperties(page, audioClip)
  const audioSections = await page.locator('[data-cut-panel="inspector"] [data-cut-section]').evaluateAll(
    (sections) => sections.map((section) => section.getAttribute('data-cut-section')),
  )
  check(
    'audio tasks replace broad blob',
    !audioSections.includes('audio-tools')
      && ['volume', 'audio-cleanup', 'audio-effects', 'audio-mix'].every((key) => audioSections.includes(key)),
    `sections=${audioSections.join(',')}`,
  )
  check(
    'voice cleanup starts visible',
    await disclosureState(page, 'audio-cleanup') === 'false'
      && await page.locator('[data-cut-action="audio-cleanup-voice"]').isVisible(),
    'one-click cleanup is immediately reachable',
  )
  const audioEffectsSummary = page.locator('[data-cut-section-summary="audio-effects"]')
  const mixSummary = page.locator('[data-cut-section-summary="audio-mix"]')
  check(
    'audio summaries expose effects and duck prerequisite',
    (await audioEffectsSummary.textContent()).includes('1 effect')
      && await audioEffectsSummary.getAttribute('data-cut-section-summary-tone') === 'active'
      && (await mixSummary.textContent()).includes('Needs a second audio track')
      && await mixSummary.getAttribute('data-cut-section-summary-tone') === 'warning',
    `effects="${await audioEffectsSummary.textContent()}" mix="${await mixSummary.textContent()}"`,
  )
  await expandWithKeyboard(page, 'audio-mix')
  check(
    'duck blocker gives a recovery action',
    await page.locator('[data-cut-inspector-blocked="duck"]').isVisible()
      && await page.getByRole('button', { name: 'Add music or audio' }).isVisible(),
    'missing speech reference is explained rather than hiding the action',
  )

  const removed = await verb('edit.stabilize', {
    clip: videoClip,
    enabled: false,
    rationale: 'inspector setup blocker check',
  })
  if (!removed.ok) throw new Error(removed.error?.message || 'could not clear stabilization')
  await page.route('**/api/verb/system.doctor', (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({
      ok: true,
      result: {
        schema: 'cut-doctor-v1',
        scanned_at: '2026-07-28T00:00:00Z',
        os: 'test',
        arch: 'test',
        app_version: 'test',
        essential_ok: true,
        cards: [{
          id: 'ffmpeg',
          kind: 'tool',
          status: 'ok',
          details: { can_stabilize: false },
        }],
      },
    }),
  }))
  await page.reload({ waitUntil: 'domcontentloaded' })
  await page.waitForTimeout(700)
  await selectProperties(page, videoClip)
  const stabilizeBlocker = page.locator('[data-cut-inspector-blocked="stabilize"]')
  check(
    'stabilization blocker names the installed prerequisite',
    await stabilizeBlocker.isVisible()
      && (await stabilizeBlocker.textContent()).includes('does not include stabilization')
      && await page.locator('[data-cut-inspector-action="stabilize"]').isDisabled(),
    (await stabilizeBlocker.textContent())?.trim() || 'missing blocker',
  )
  await stabilizeBlocker.getByRole('button', { name: 'Open video setup' }).click()
  await page.locator('[data-cut-settings-body="video-performance"]').waitFor()
  check('stabilization setup action routes exactly', true, 'opened Settings > Video & performance')
  check('browser console stays clean', consoleErrors.length === 0, `${consoleErrors.length} console error(s)`)
} finally {
  await browser?.close().catch(() => {})
  await verb('project.close').catch(() => {})
  await verb('project.forget', { path: projectDir }).catch(() => {})
  rmSync(temp, { recursive: true, force: true })
}

const failed = checks.filter((entry) => !entry.pass)
console.log(`\n${checks.length - failed.length} PASS, ${failed.length} FAIL`)
if (failed.length > 0) process.exitCode = 1
