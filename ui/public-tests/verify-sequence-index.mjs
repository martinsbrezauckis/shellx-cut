import { mkdtempSync, rmSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6216'
const APP = process.env.SWEEP_APP || CUTD
const temp = mkdtempSync(join(tmpdir(), 'cut-sequence-index-'))
const projectDir = join(temp, 'sequence-index-rig.cutproj')
const mediaPath = join(temp, 'Interview Hero.mp4')
const screenshotPath = process.env.SEQUENCE_INDEX_SCREENSHOT
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

async function expectVerb(name, args = {}) {
  const envelope = await verb(name, args)
  if (!envelope.ok) throw new Error(`${name} failed: ${envelope.error?.message || 'unknown error'}`)
  return envelope.result
}

async function waitForUiOpen(panel, timeoutMs = 8000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await verb('ui.open', { panel })
    if (last.ok) return last.result
    await new Promise((resolveWait) => setTimeout(resolveWait, 250))
  }
  throw new Error(`ui.open ${panel} did not reach a connected UI: ${JSON.stringify(last)}`)
}

async function waitForProject(predicate, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const project = await expectVerb('project.state')
    if (predicate(project)) return project
    await new Promise((resolveWait) => setTimeout(resolveWait, 100))
  }
  throw new Error('timed out waiting for project state')
}

async function summary(page, total, breakdown) {
  const locator = page.locator('[data-cut-sequence-index-summary]')
  let expected = locator.filter({ hasText: `${total} result${total === 1 ? '' : 's'}` })
  if (breakdown) expected = expected.filter({ hasText: breakdown })
  await expected.waitFor()
  return (await locator.textContent()).replace(/\s+/g, ' ').trim()
}

let browser
try {
  execFileSync('ffmpeg', [
    '-nostdin', '-hide_banner', '-loglevel', 'error',
    '-f', 'lavfi', '-i', 'color=c=0x28506f:s=640x360:r=30',
    '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000',
    '-t', '2', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-shortest', mediaPath,
  ])
  await expectVerb('project.create', {
    name: 'sequence-index-rig',
    dir: projectDir,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  const imported = await expectVerb('media.import', { path: mediaPath, proxy: false })
  const importedProject = await waitForProject((project) => project.tracks.some((track) =>
    track.kind === 'video' && track.clips?.some((clip) => clip.asset === imported.asset_id),
  ))
  const videoClip = importedProject.tracks
    .find((track) => track.kind === 'video')
    ?.clips?.find((clip) => clip.asset === imported.asset_id)
  if (!videoClip?.id) throw new Error('imported video clip is missing from the timeline')
  await expectVerb('edit.effect', {
    clip: videoClip.id,
    effects: [{ type: 'vignette', amount: 0.4 }],
  })
  await expectVerb('edit.add_marker', {
    at_ms: 900,
    label: 'Review hook',
    note: '=1+1, client approved, ready',
  })
  const social = await expectVerb('project.sequence_create', { name: 'Social Cut', from: 'active' })
  const socialMarker = await expectVerb('edit.add_marker', { at_ms: 1500, label: 'TikTok ending' })
  await expectVerb('edit.update_marker', { id: socialMarker.marker_id, color: 'purple' })

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async (value) => { window.__cutSequenceIndexCsv = value } },
    })
  })
  const consoleErrors = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  let switchArgs = null
  let playheadArgs = null
  page.on('request', (request) => {
    if (request.url().includes('/api/verb/project.sequence_switch')) switchArgs = request.postDataJSON()
    if (request.url().includes('/api/verb/ui.playhead')) playheadArgs = request.postDataJSON()
  })

  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  const opened = await waitForUiOpen('sequence-index')
  const sequenceTab = page.locator('[data-cut-find-tab="sequence-index"]')
  await sequenceTab.waitFor()
  await page.waitForFunction(() => document.querySelector('[data-cut-find-tab="sequence-index"]')?.getAttribute('aria-selected') === 'true')
  check(
    'ui.open reveals the connected Sequence Index tab',
    (opened.applied === true || opened.sent === true)
      && await sequenceTab.getAttribute('aria-selected') === 'true',
    JSON.stringify(opened),
  )
  const allSummary = await summary(page, 7, '4 clips / 3 markers')
  check('index spans active and inactive sequences', allSummary.includes('4 clips / 3 markers'), allSummary)

  await page.locator('[data-cut-sequence-index-copy]').click()
  const copiedCsv = await page.evaluate(() => window.__cutSequenceIndexCsv || '')
  check(
    'bounded path-light Sequence Index rows copy as escaped CSV',
    copiedCsv.startsWith('sequence,kind,label,at_ms')
      && copiedCsv.includes('issues,track_visible,track_locked,track_muted')
      && copiedCsv.includes('"\'=1+1, client approved, ready"')
      && !copiedCsv.includes(temp),
    `bytes=${copiedCsv.length} spreadsheetSafe=${copiedCsv.includes('"\'=1+1, client approved, ready"')} path=${copiedCsv.includes(temp)}`,
  )

  const bodyText = await page.locator('[data-cut-sequence-index]').textContent()
  check(
    'full source path is absent from Sequence Index DOM',
    bodyText.includes('Interview Hero.mp4') && !bodyText.includes(temp),
    `basename=${bodyText.includes('Interview Hero.mp4')} path=${bodyText.includes(temp)}`,
  )

  const sourceButton = page.locator('button[data-cut-sequence-index-source]').first()
  await sourceButton.click()
  await page.locator('[data-cut-left-tab="assets"][aria-selected="true"]').waitFor()
  await page.locator('[data-cut-source-monitor]').waitFor()
  check(
    'Source action reveals the source monitor instead of opening it behind Find',
    await page.locator('[data-cut-left-tab="assets"]').getAttribute('aria-selected') === 'true'
      && await page.locator('[data-cut-source-monitor]').count() === 1,
    `assets=${await page.locator('[data-cut-left-tab="assets"]').getAttribute('aria-selected')} monitor=${await page.locator('[data-cut-source-monitor]').count()}`,
  )
  await page.locator('[data-cut-source-monitor-close]').click()
  await page.locator('[data-cut-left-tab="find"]').click()
  await page.locator('[data-cut-find-tab="sequence-index"]').click()
  await page.locator('[data-cut-sequence-index-summary]').waitFor()

  await page.locator('[data-cut-sequence-index-track]').selectOption('video')
  check('track filter isolates video clips', (await summary(page, 2, '2 clips / 0 markers')).includes('2 clips / 0 markers'), await summary(page, 2, '2 clips / 0 markers'))
  await page.locator('[data-cut-sequence-index-track]').selectOption('')

  rmSync(mediaPath)
  await page.locator('[data-cut-sequence-index-status]').selectOption('offline')
  check('live status filter isolates offline media', (await summary(page, 4, '4 clips / 0 markers')).includes('4 issues'), await summary(page, 4, '4 clips / 0 markers'))
  await page.locator('[data-cut-sequence-index-status]').selectOption('effects')
  check('effect filter and badges expose effect-bearing clips', (await summary(page, 2, '2 clips / 0 markers')).includes('2 clips'), await page.locator('.si__badges').first().textContent())
  await page.locator('[data-cut-sequence-index-status]').selectOption('all')

  await page.locator('[data-cut-sequence-index-kind="marker"]').click()
  check('marker filter excludes clips', (await summary(page, 3, '0 clips / 3 markers')).includes('0 clips / 3 markers'), await summary(page, 3, '0 clips / 3 markers'))
  await page.locator('[data-cut-sequence-index-query]').fill('TikTok purple')
  await page.locator('[data-cut-sequence-index-search]').click()
  check('marker text and color are searchable', (await summary(page, 1, '0 clips / 1 marker')).includes('1 marker'), await summary(page, 1, '0 clips / 1 marker'))

  await page.locator('[data-cut-sequence-index-query]').fill('')
  await page.locator('[data-cut-sequence-index-search]').click()
  await page.locator('[data-cut-sequence-index-kind="all"]').click()
  await page.locator('[data-cut-sequence-index-sequence]').selectOption('seq1')
  check('sequence filter scopes results', (await summary(page, 3, '2 clips / 1 marker')).includes('2 clips / 1 marker'), await summary(page, 3, '2 clips / 1 marker'))
  await page.locator('[data-cut-sequence-index-kind="marker"]').click()
  await summary(page, 1, '0 clips / 1 marker')

  const reviewRow = page.locator('[data-cut-sequence-index-row-kind="marker"]').filter({ hasText: 'Review hook' })
  const openButton = reviewRow.locator('button[data-cut-sequence-index-open]')
  const openBox = await openButton.boundingBox()
  const timelineBox = await page.locator('.app__timeline').boundingBox()
  const hitTarget = openBox ? await page.evaluate(({ x, y }) => {
    const hit = document.elementFromPoint(x, y)
    return hit?.closest('button')?.getAttribute('data-cut-sequence-index-open') ?? null
  }, { x: openBox.x + openBox.width / 2, y: openBox.y + openBox.height / 2 }) : 'no-box'
  const fullyAboveTimeline = Boolean(openBox && timelineBox && openBox.y + openBox.height <= timelineBox.y)
  check(
    'visible result Open action owns its full hit target',
    hitTarget === 'seq1:marker:m1:900' && fullyAboveTimeline,
    `button=${JSON.stringify(openBox)} timeline=${JSON.stringify(timelineBox)} hit=${hitTarget}`,
  )
  if (screenshotPath) await page.screenshot({ path: screenshotPath, fullPage: true })
  await openButton.click()
  await page.locator('[data-cut-sequence-active="seq1"]').waitFor()
  await page.waitForTimeout(150)
  check(
    'inactive-sequence result switches sequence and seeks',
    switchArgs?.id === 'seq1' && playheadArgs?.at_ms === 900,
    `switch=${JSON.stringify(switchArgs)} playhead=${JSON.stringify(playheadArgs)} from=${social.active_sequence}`,
  )

  const layout = await page.evaluate(() => ({
    rootOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
    panelOverflow: [...document.querySelectorAll('[data-cut-sequence-index], [data-cut-sequence-index-row]')]
      .some((element) => element.scrollWidth > element.clientWidth),
    tabOverflow: [...document.querySelectorAll('[data-cut-find-tab]')]
      .some((element) => element.scrollWidth > element.clientWidth),
  }))
  check(
    'Sequence Index fits the supported minimum window',
    layout.rootOverflow === 0 && !layout.panelOverflow && !layout.tabOverflow,
    JSON.stringify(layout),
  )
  check('Sequence Index emits no browser console errors', consoleErrors.length === 0, consoleErrors.join(' | ') || 'none')
} finally {
  await browser?.close().catch(() => {})
  await verb('project.close').catch(() => {})
  await verb('project.forget', { path: projectDir }).catch(() => {})
  rmSync(temp, { recursive: true, force: true })
}

if (checks.some((entry) => !entry.pass)) process.exitCode = 1
