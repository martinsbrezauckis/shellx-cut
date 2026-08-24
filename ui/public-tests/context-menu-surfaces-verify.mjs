// Focused real-browser UX-CONTEXT-01 proof. Requires a local cutd + Vite app:
// SWEEP_CUTD=http://127.0.0.1:6193 SWEEP_APP=http://127.0.0.1:5193 \
// CONTEXT_MENU_MUXED_CLIP=/tmp/muxed.mp4 CONTEXT_MENU_PROJECT_ROOT=/repo/.scratch/context \
// node public-tests/context-menu-surfaces-verify.mjs
import { chromium } from 'playwright'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { verifyReverseMatchFrameAtStart } from './lib/contextMenuReverseMatchFrame.mjs'
import { sealedPlaywrightChromiumLaunchOptions } from './lib/sealedPlaywrightChromium.mjs'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6193'
const APP = process.env.SWEEP_APP || 'http://127.0.0.1:5193'
const CLIP = process.env.CONTEXT_MENU_MUXED_CLIP
const PROJECT_NAME = `context-surfaces-${process.pid}`
const PROJECT_ROOT = process.env.CONTEXT_MENU_PROJECT_ROOT
const PROJECT = process.env.CONTEXT_MENU_PROJECT
  || join(PROJECT_ROOT ? resolve(PROJECT_ROOT) : tmpdir(), `${PROJECT_NAME}.cutproj`)
if (!CLIP) throw new Error('CONTEXT_MENU_MUXED_CLIP must point to a small muxed video fixture')

const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:ui' },
    body: JSON.stringify(args),
  })
  return response.json()
}
const state = async () => (await verb('project.state')).result
let pass = 0
let fail = 0
const check = (name, ok, detail = '') => {
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` — ${detail}` : ''}`)
  if (ok) pass++
  else fail++
}
const menuItems = async (page, menu, selector) => (await page.locator(menu).locator(selector).evaluateAll((nodes) => nodes.map((node) => ({
  key: node.getAttribute(selector.slice(1, -1)), disabled: node.hasAttribute('disabled'),
}))))

const created = await verb('project.create', { name: PROJECT_NAME, dir: PROJECT })
if (!created.ok) throw new Error(`project.create failed: ${created.error?.message ?? 'unknown'}`)
const opened = await verb('project.open', { path: PROJECT })
if (!opened.ok) throw new Error(`project.open failed: ${opened.error?.message ?? 'unknown'}`)
const imported = await verb('media.import', { path: CLIP })
if (!imported.ok) throw new Error(`media.import failed: ${imported.error?.message ?? 'unknown'}`)
await wait(800)
let project = await state()
const duplicate = await verb('project.sequence_create', {
  name: 'Source navigation alternate',
  from: 'active',
  rationale: 'focused source navigation browser fixture',
})
const alternateSequenceId = duplicate.result?.sequence?.id
if (!duplicate.ok || !alternateSequenceId) throw new Error(`project.sequence_create failed: ${duplicate.error?.message ?? 'no sequence id'}`)
const sequenceList = await verb('project.sequence_list')
const primarySequence = sequenceList.result?.sequences?.find((sequence) => sequence.id !== alternateSequenceId)
const primarySequenceId = primarySequence?.id
if (!primarySequenceId || !primarySequence?.name) throw new Error('could not identify primary source-navigation sequence')
const switchedPrimary = await verb('project.sequence_switch', { id: primarySequenceId, rationale: 'focus source navigation primary sequence' })
if (!switchedPrimary.ok) throw new Error(`project.sequence_switch failed: ${switchedPrimary.error?.message ?? 'unknown'}`)
project = await state()
let videoTrack = project.tracks.find((track) => track.kind === 'video')
let videoClip = videoTrack?.clips?.find((clip) => clip.asset)
const assetId = videoClip?.asset
if (!videoTrack || !videoClip || !assetId) throw new Error('fixture did not produce a base video asset and clip')

// Give the primary sequence a real upstream transition. The alternate sequence
// stays hard-cut, so selecting the primary's second occurrence crosses sequence
// AND must use its laid (not nominal) start position.
const sourceDuration = videoClip.src_out_ms - videoClip.src_in_ms
const splitAtMs = Math.floor(sourceDuration / 2)
const xfadeMs = Math.min(300, Math.floor(splitAtMs / 2), Math.floor((sourceDuration - splitAtMs) / 2))
if (!Number.isInteger(splitAtMs) || xfadeMs < 1) throw new Error(`fixture is too short for cross-sequence crossfade navigation: ${sourceDuration}ms`)
const split = await verb('edit.split', { track: videoTrack.id, at_ms: splitAtMs })
if (!split.ok) throw new Error(`edit.split failed: ${split.error?.message ?? 'unknown'}`)
const crossfade = await verb('edit.crossfade', { track: videoTrack.id, at_ms: splitAtMs, duration_ms: xfadeMs })
if (!crossfade.ok) throw new Error(`edit.crossfade failed: ${crossfade.error?.message ?? 'unknown'}`)
project = await state()
videoTrack = project.tracks.find((track) => track.id === videoTrack.id)
videoClip = videoTrack?.clips?.find((clip) => clip.asset)
const xfadeClip = videoTrack?.clips?.find((clip) => clip.xfade_in_ms === xfadeMs)
if (!videoTrack || !videoClip || !xfadeClip) throw new Error('crossfade fixture did not retain the selected media occurrence')
const xfadeOccurrence = {
  sequenceId: primarySequenceId,
  trackId: videoTrack.id,
  clipId: xfadeClip.id,
  atMs: splitAtMs - xfadeMs,
}
const switchedAlternate = await verb('project.sequence_switch', { id: alternateSequenceId, rationale: 'focus source navigation alternate' })
if (!switchedAlternate.ok) throw new Error(`project.sequence_switch failed: ${switchedAlternate.error?.message ?? 'unknown'}`)
project = await state()
videoTrack = project.tracks.find((track) => track.kind === 'video')
videoClip = videoTrack?.clips?.find((clip) => clip.asset)
if (!videoTrack || !videoClip) throw new Error('alternate sequence lost its source-navigation clip')

const browser = await chromium.launch(sealedPlaywrightChromiumLaunchOptions())
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } })
await page.goto(APP, { waitUntil: 'domcontentloaded' })
await verb('project.open', { path: PROJECT })
await page.reload({ waitUntil: 'domcontentloaded' })
await page.locator('[data-cut-mode="edit"]').first().waitFor({ state: 'visible', timeout: 15_000 })
await page.waitForTimeout(1000)

async function openRightClick(target, menu) {
  await page.keyboard.press('Escape').catch(() => {})
  // A trim affordance intentionally overlaps a few edge pixels of a gap. Force
  // sends the real pointer event at the target coordinate, exercising the
  // resolver's elementsFromPoint gap ownership rather than Playwright's guard.
  await target.click({ button: 'right', force: true })
  await page.locator(menu).waitFor({ state: 'visible', timeout: 2500 })
  return page.locator(menu)
}

// Preview keeps the base video identity captured at right-click and hands the
// exact asset/source time to the existing Source Monitor bridge.
{
  const monitor = page.locator('[data-cut-monitor]').first()
  const center = await monitor.evaluate((node) => {
    const rect = node.getBoundingClientRect()
    const hit = document.elementFromPoint(
      rect.left + (rect.width / 2),
      rect.top + (rect.height / 2),
    )
    return {
      hit: hit?.tagName.toLowerCase() || '',
      className: typeof hit?.className === 'string' ? hit.className : '',
      passiveMedia: hit?.matches('.pv-base, img[data-cut-poster]') === true,
      routesToMonitor: !!hit && (hit === node || node.contains(hit)),
    }
  })
  check(
    'Preview monitor center routes its native context gesture past passive media',
    center.routesToMonitor && !center.passiveMedia,
    JSON.stringify(center),
  )
  await page.keyboard.press('Escape').catch(() => {})
  await page.locator('[data-cut-preview-menu-button]').click()
  const preview = page.locator('[data-cut-preview-menu]').first()
  await preview.waitFor({ state: 'visible', timeout: 2500 })
  const items = await preview.locator('[data-cut-preview-ctx]').evaluateAll((nodes) => nodes.map((node) => ({ key: node.getAttribute('data-cut-preview-ctx'), disabled: node.hasAttribute('disabled') })))
  check('Preview menu exposes exact-base source and marker routes', items.some((item) => item.key === 'preview-open-source' && !item.disabled) && items.some((item) => item.key === 'preview-add-marker' && !item.disabled))
  await preview.locator('[data-cut-preview-ctx="preview-open-source"]').click()
  const source = page.locator(`[data-cut-source-monitor="${assetId}"]`)
  await source.waitFor({ state: 'visible', timeout: 5_000 })
  check('Preview source route preserves exact asset identity', await source.count() === 1)
  await page.locator('[data-cut-source-monitor-close]').click()
  await source.waitFor({ state: 'detached', timeout: 5_000 })
}

// Assets opens a menu for the clicked card only; an image/non-media could not
// borrow this source route because the component class-gates it before render.
{
  await page.locator('[data-cut-left-tab="assets"]').click()
  await page.locator(`[data-cut-asset-menu-button="${assetId}"]`).click()
  const menu = page.locator('[data-cut-asset-menu]')
  await menu.waitFor({ state: 'visible', timeout: 2500 })
  const items = await menu.locator('[data-cut-asset-ctx]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-cut-asset-ctx')))
  check('Assets menu targets the clicked asset', items.includes('asset-open-source') && items.includes('asset-add-playhead'), items.join(','))
  await page.keyboard.press('Escape')
  await page.waitForTimeout(80)
  check('Assets menu dismisses with Escape', await page.locator('[data-cut-asset-menu]').count() === 0)
}

// Clip-context source reveal owns the same stable action IDs as Source Monitor,
// but must prove its own exact right-click instance. Browser mode never invokes
// native shell: it gives an assertive refusal. The Project route then leaves the
// menu and visibly selects the exact registered asset in the existing tray.
{
  const clip = page.locator(`[data-cut-clip="${videoClip.id}"]`).first()
  let menu = await openRightClick(clip, '[data-cut-clip-menu]')
  const revealFile = menu.locator('[data-cut-action="reveal-source-file"]')
  check('Clip context exposes enabled registered Source file reveal',
    await revealFile.count() === 1 && !(await revealFile.isDisabled()))
  await revealFile.focus()
  await page.keyboard.press('Enter')
  const refusal = page.locator('[data-cut-user-action-feedback]').first()
  await refusal.waitFor({ state: 'visible', timeout: 2500 })
  const refusalMessage = await refusal.locator('[data-cut-user-action-feedback-message]').textContent()
  check('Clip context browser Source file reveal refuses without native invoke',
    refusalMessage?.includes('Open the desktop app to reveal the registered source file')
      && await refusal.getAttribute('role') === 'alert'
      && await refusal.getAttribute('aria-live') === 'assertive',
    refusalMessage || 'missing alert')
  await refusal.locator('[data-cut-user-action-dismiss]').click()

  menu = await openRightClick(clip, '[data-cut-clip-menu]')
  const revealProject = menu.locator('[data-cut-action="reveal-source-project"]')
  check('Clip context exposes enabled exact Project reveal',
    await revealProject.count() === 1 && !(await revealProject.isDisabled()))
  await revealProject.click()
  const selected = page.locator(`[data-cut-asset-selected="${assetId}"]`).first()
  await selected.waitFor({ state: 'visible', timeout: 5000 })
  check('Clip context Project reveal selects the exact registered asset',
    await selected.count() === 1 && await selected.getAttribute('data-cut-asset-selected') === assetId)
}

// Match Frame is a true keyboard-operated route from the exact footage clip:
// first prove the reversed first-frame boundary, then open Source Monitor at
// source time and use the exact per-asset Sequence Index to switch and seek the
// primary sequence's selected occurrence after an upstream crossfade.
await verifyReverseMatchFrameAtStart({ page, verb, state, check, openRightClick, videoClip, assetId })

{
  const clip = page.locator(`[data-cut-clip="${videoClip.id}"]`).first()
  const menu = await openRightClick(clip, '[data-cut-clip-menu]')
  const matchFrame = menu.locator('[data-cut-action="match-frame"]')
  check('Footage clip exposes exact Match Frame', await matchFrame.count() === 1 && !(await matchFrame.isDisabled()))
  await matchFrame.focus()
  await page.keyboard.press('Enter')
  const source = page.locator(`[data-cut-source-monitor="${assetId}"]`)
  await source.waitFor({ state: 'visible', timeout: 2500 })
  await page.waitForFunction((expectedAsset) => {
    const media = document.querySelector(`[data-cut-source-monitor="${expectedAsset}"] video`)
    return media instanceof HTMLVideoElement && Number.isFinite(media.duration) && media.duration > 0
  }, assetId)
  check('Clip Match Frame opens the exact source frame',
    (await source.locator('[data-cut-source-current]').textContent()) === '0:00.000',
    `source=${await source.locator('[data-cut-source-current]').textContent()}`)

  const usesButton = source.locator('[data-cut-action="source-monitor-all-uses"]')
  let rejectedIndexRequest = false
  await page.route('**/api/verb/project.sequence_index', async (route) => {
    if (!rejectedIndexRequest) {
      rejectedIndexRequest = true
      await route.abort('failed')
      return
    }
    await route.continue()
  })
  await usesButton.click()
  await source.locator('[data-cut-source-note]').filter({ hasText: 'Server unreachable' }).waitFor({ state: 'visible', timeout: 2500 })
  check('All uses transport rejection clears loading and exposes recovery',
    rejectedIndexRequest && !(await usesButton.isDisabled()) && (await usesButton.textContent()) === 'Hide uses',
    `disabled=${await usesButton.isDisabled()} label=${await usesButton.textContent()}`)
  await usesButton.click()
  check('All uses can hide after a transport rejection',
    (await usesButton.getAttribute('aria-expanded')) === 'false',
    `expanded=${await usesButton.getAttribute('aria-expanded')}`)
  await usesButton.click()
  const uses = source.locator('[data-cut-source-uses]')
  await uses.waitFor({ state: 'visible', timeout: 2500 })
  await page.unroute('**/api/verb/project.sequence_index')
  const xfadeUse = uses.locator(`[data-cut-source-use="${xfadeOccurrence.sequenceId}:${xfadeOccurrence.trackId}:${xfadeOccurrence.clipId}"]`)
  await xfadeUse.waitFor({ state: 'visible', timeout: 2500 })
  const playheadRequests = []
  const sequenceSwitchRequests = []
  const onRequest = (request) => {
    if (request.url().includes('/api/verb/ui.playhead')) playheadRequests.push(request.postDataJSON())
    if (request.url().includes('/api/verb/project.sequence_switch')) sequenceSwitchRequests.push(request.postDataJSON())
  }
  page.on('request', onRequest)
  await xfadeUse.click()
  try {
    await source.waitFor({ state: 'detached', timeout: 10_000 })
  } catch (error) {
    const stuckSequences = await verb('project.sequence_list')
    const note = await source.locator('[data-cut-source-note]').textContent().catch(() => '')
    throw new Error(
      `All uses did not close Source Monitor: active=${stuckSequences.result?.active_sequence || 'none'} `
      + `switchRequests=${JSON.stringify(sequenceSwitchRequests)} `
      + `playheadRequests=${JSON.stringify(playheadRequests)} note=${note || 'none'}; ${error.message}`,
    )
  }
  page.off('request', onRequest)
  const finalSequenceList = await verb('project.sequence_list')
  const finalActiveSequence = finalSequenceList.result?.active_sequence
  const primarySequenceIsActive = finalSequenceList.result?.sequences
    ?.some((sequence) => sequence.id === primarySequenceId && sequence.active === true)
  project = await state()
  const ui = await verb('ui.state')
  check('All uses switches and seeks the selected crossfade occurrence in laid time',
    finalActiveSequence === primarySequenceId
      && primarySequenceIsActive
      && sequenceSwitchRequests.some((request) => request.id === primarySequenceId)
      && playheadRequests.some((request) => request.at_ms === xfadeOccurrence.atMs)
      && ui.result?.playhead_ms === xfadeOccurrence.atMs,
    `active=${finalActiveSequence || 'none'} expected=${xfadeOccurrence.atMs} `
      + `switches=${JSON.stringify(sequenceSwitchRequests)} `
      + `playheadRequests=${JSON.stringify(playheadRequests)} ui=${ui.result?.playhead_ms}`)
}

// A track header has no clip identity. At the primary sequence's real
// crossfade overlap, it must not quietly take whichever laid clip happens to
// be enumerated first; the visible disabled action tells the editor how to
// disambiguate. Clip-target Match Frame above remains the exact route.
{
  const header = page.locator(`[data-cut-track-header="${xfadeOccurrence.trackId}"]`).first()
  await header.focus()
  await page.keyboard.press('Shift+F10')
  const menu = page.locator('[data-cut-track-menu]')
  await menu.waitFor({ state: 'visible', timeout: 2500 })
  const matchFrame = menu.locator('[data-cut-action="match-frame-track"]')
  const reason = await matchFrame.getAttribute('aria-description')
  check('Track Match Frame refuses an ambiguous crossfade',
    await matchFrame.count() === 1
      && await matchFrame.isDisabled()
      && reason === 'More than one video clip covers this playhead; select one clip to match its exact frame'
      && await page.locator('[data-cut-source-monitor]').count() === 0,
    `disabled=${await matchFrame.isDisabled()}; reason=${reason || 'missing'}`)
  await page.keyboard.press('Escape')
}

const resetAfterCrossfade = await verb('ui.playhead', { at_ms: 0 })
if (!resetAfterCrossfade.ok) throw new Error(`could not reset playhead after crossfade ambiguity proof: ${resetAfterCrossfade.error?.message ?? 'unknown'}`)

// Recent Projects operates on the exact card; current-project reopen/delete
// are visible but disabled with explanatory titles rather than retargeted.
{
  await page.locator('[data-cut-left-tab="projects"]').click()
  const card = page.locator(`[data-cut-project-card]`).filter({ hasText: project.name }).first()
  await card.waitFor({ state: 'visible', timeout: 3500 })
  const menu = await openRightClick(card, '[data-cut-project-menu]')
  const reopen = menu.locator('[data-cut-project-ctx="project-reopen"]')
  const remove = menu.locator('[data-cut-project-ctx="project-delete"]')
  check('Projects current card refuses reopen/delete', await reopen.isDisabled() && await remove.isDisabled())
}

// Native custom speed carries the engine window directly. Valid boundaries and
// valid precision are enabled; invalid/ambiguous numeric values never dispatch.
{
  const clip = page.locator(`[data-cut-clip="${videoClip.id}"]`).first()
  const menu = await openRightClick(clip, '[data-cut-clip-menu]')
  await menu.locator('[data-cut-ctx="speed-time"]').click()
  const input = page.locator('[data-cut-custom-speed-input]')
  const apply = page.locator('[data-cut-ctx="speed-custom"]')
  for (const value of ['0.25', '4', '0.251']) {
    await input.fill(value)
    check(`Custom speed accepts engine-valid ${value}×`, !(await apply.isDisabled()))
  }
  for (const value of ['0.249', '4.001', '']) {
    await input.fill(value)
    check(`Custom speed refuses invalid ${value || 'empty value'}`, await apply.isDisabled())
  }
  await page.keyboard.press('Escape')
}

// Leave a real video gap using the engine's lift semantics; the gap menu must
// be gap-only and copied-source fit remains disabled until an exact source is
// present in the app clipboard.
const lifted = await verb('edit.ripple_delete', { track: videoTrack.id, range_ms: [500, 1000], ripple: false, rationale: 'context-menu browser fixture gap' })
check('fixture creates a lift gap', lifted.ok)
await page.reload({ waitUntil: 'domcontentloaded' })
await page.locator('[data-cut-mode="edit"]').first().waitFor({ state: 'visible', timeout: 15_000 })
await page.waitForTimeout(650)
{
  const gap = page.locator('[data-cut-gap]').first()
  const menu = await openRightClick(gap, '[data-cut-gap-menu]')
  const items = await menu.locator('[data-cut-timeline-ctx]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-cut-timeline-ctx')))
  const fit = menu.locator('[data-cut-timeline-ctx="gap-fit-clipboard"]')
  check('Gap menu exposes only gap-valid routes', items.length === 4 && items.includes('gap-paste') && items.includes('gap-select-range') && items.includes('gap-fit-clipboard') && !items.includes('speed-time'), items.join(','))
  check('Gap fit refuses absent clipboard identity', await fit.isDisabled())
}

// Empty lane context comes from the lane itself, not a stale clip id. The menu
// is operational (seek/marker/range/tracks) and contains no clip edit route.
{
  await page.keyboard.press('Escape')
  const lane = page.locator(`[data-cut-track="${videoTrack.id}"] .tl-lane`).first()
  await lane.evaluate((node) => {
    const rect = node.getBoundingClientRect()
    node.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: rect.left + Math.min(700, rect.width - 12), clientY: rect.top + 12 }))
  })
  const menu = page.locator('[data-cut-timeline-empty-menu]')
  await menu.waitFor({ state: 'visible', timeout: 2500 })
  const items = await menu.locator('[data-cut-timeline-ctx]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-cut-timeline-ctx')))
  check('Empty timeline menu is non-clip operational context', items.includes('empty-seek') && items.includes('empty-paste') && items.includes('empty-marker') && !items.includes('remove'), items.join(','))
}

// The track header owns discrete track controls; it deliberately does not
// duplicate the dense mixer or reorder controls that already live in the rail.
{
  await page.keyboard.press('Escape')
  const header = page.locator(`[data-cut-track-header="${videoTrack.id}"]`).first()
  await header.focus()
  await page.keyboard.press('Shift+F10')
  const menu = page.locator('[data-cut-track-menu]')
  await menu.waitFor({ state: 'visible', timeout: 2500 })
  const actions = await menu.locator('[data-cut-track-ctx]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-cut-track-ctx')))
  const matchFrame = menu.locator('[data-cut-action="match-frame-track"]')
  check('Track header keyboard menu owns Match Frame plus video track controls', actions.includes('match-frame') && actions.includes('lock') && actions.includes('visibility') && !actions.includes('remove') && !actions.includes('mute') && !(await matchFrame.isDisabled()), actions.join(','))
  await matchFrame.focus()
  await page.keyboard.press('Enter')
  const source = page.locator(`[data-cut-source-monitor="${assetId}"]`)
  await source.waitFor({ state: 'visible', timeout: 2500 })
  check('Track Match Frame keyboard action opens Source Monitor', await source.count() === 1)
  await source.locator('[data-cut-source-monitor-close]').click()
}

// Lock the exact track after the gap test. A contained clip may still use
// read-only Match Frame or Inspect, and may unlock its track; no edit mutation
// can be reached while locked. Clicking Unlock targets this exact track id.
const locked = await verb('edit.track_lock', { track: videoTrack.id, on: true, rationale: 'context-menu browser fixture lock' })
check('fixture locks base video track', locked.ok)
await page.reload({ waitUntil: 'domcontentloaded' })
await page.locator('[data-cut-mode="edit"]').first().waitFor({ state: 'visible', timeout: 15_000 })
await page.waitForTimeout(650)
{
  const lockedClip = page.locator(`[data-cut-track="${videoTrack.id}"][data-cut-track-locked="true"] [data-cut-clip]`).first()
  const menu = await openRightClick(lockedClip, '[data-cut-locked-track-menu]')
  const actions = await menu.locator('[data-cut-track-ctx]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-cut-track-ctx')))
  check('Locked track keeps read-only Match Frame but blocks edit mutations',
    actions.every((action) => ['match-frame', 'inspect', 'lock'].includes(action)) && actions.includes('match-frame'),
    actions.join(','))
  await menu.locator('[data-cut-track-ctx="lock"]').click()
  await page.waitForTimeout(350)
  project = await state()
  check('Locked menu unlock dispatches to the exact locked track', project.tracks.find((track) => track.id === videoTrack.id)?.locked !== true)
}

await browser.close()
console.log(`\n${fail === 0 ? 'PASS' : 'FAIL'} context-menu surfaces — ${pass} pass / ${fail} fail`)
process.exit(fail === 0 ? 0 : 1)
