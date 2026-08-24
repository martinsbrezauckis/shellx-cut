// Candidate-bound rendered audit rows for Timeline context ownership and paste.
// This deliberately extends the canonical full-coverage runner; it is not a
// second browser harness or a substitute for installed/native qualification.
import { join } from 'node:path'
import { proveSourceMonitorFrame } from './fullCoverageSourceMonitorFrameProof.mjs'

export const TIMELINE_CONTEXT_AUDIT_SCENARIOS = [
  { id: 'e2e-context-ownership-01', runner: 'timeline-context-audit', surface: 'browser-ui', receiptSchema: 'shellx-cut/full-coverage-results@1', command: ['FCV_SECTION=ctxmenu', 'FCV_ONLY=e2e-context-ownership-01', 'node public-tests/full-coverage-verify.mjs'], receiptRoot: 'full-coverage' },
  { id: 'e2e-empty-paste-01', runner: 'timeline-context-audit', surface: 'browser-ui', receiptSchema: 'shellx-cut/full-coverage-results@1', command: ['FCV_SECTION=ctxmenu', 'FCV_ONLY=e2e-empty-paste-01', 'node public-tests/full-coverage-verify.mjs'], receiptRoot: 'full-coverage' },
  { id: 'e2e-gap-boundary-01', runner: 'timeline-context-audit', surface: 'browser-ui', receiptSchema: 'shellx-cut/full-coverage-results@1', command: ['FCV_SECTION=ctxmenu', 'FCV_ONLY=e2e-gap-boundary-01', 'node public-tests/full-coverage-verify.mjs'], receiptRoot: 'full-coverage' },
]

export function createTimelineContextAuditCoverage({ probe, verb, state, waitForState, captureVerbResp, sleep, freshProject, closeOverlays, selectClipPair }) {
  const surface = 'timeline-context-audit'
  const clips = (project, kind) => (project?.tracks || []).flatMap((track) => (track.clips || [])
    .filter((clip) => !kind || track.kind === kind).map((clip) => ({ ...clip, trackId: track.id, trackKind: track.kind })))
  const media = (project, kind = 'video') => clips(project, kind).find((clip) => clip.id && clip.asset)
  const track = (project, id) => project?.tracks?.find((candidate) => candidate.id === id)
  const clipSpan = (clip) => Math.max(1, (clip?.src_out_ms || 0) - (clip?.src_in_ms || 0))
  const responseError = (response) => String(response?.error?.message || response?.error?.cause || response?.error?.code || '')
  const snapshot = (project) => JSON.stringify((project?.tracks || []).map((item) => [item.id, (item.clips || [])
    .map((clip) => [clip.id || clip.kind, clip.asset || '', clip.src_in_ms || 0, clip.src_out_ms || 0, clip.speed || 1])]))
  const selected = async () => (await verb('ui.state', {})).result?.selected_clip_ids || []
  const current = async () => state()
  const matches = (id, only) => !only || id.includes(only) || only.includes(id)
  const dismissed = async (page, selector) => page.locator(selector).first()
    .waitFor({ state: 'hidden', timeout: 4_000 }).then(() => true).catch(() => false)

  async function pasteRefusalAlert(page, response) {
    const alert = page.locator('[data-cut-user-action-feedback]').first()
    const shown = await alert.waitFor({ state: 'visible', timeout: 4_000 }).then(() => true).catch(() => false)
    const message = shown ? await alert.locator('[data-cut-user-action-feedback-message]').textContent() || '' : ''
    const accessible = shown && (await alert.getAttribute('role')) === 'alert' && (await alert.getAttribute('aria-live')) === 'assertive'
    if (shown) await alert.locator('[data-cut-user-action-dismiss]').click()
    return accessible && message.includes(String(response?.error?.message || ''))
  }

  async function openClip(page, id, { preserveSelection = false } = {}) {
    if (!preserveSelection) await closeOverlays(page)
    const target = page.locator(`[data-cut-clip="${id}"]`).first()
    await target.scrollIntoViewIfNeeded()
    await target.click({ button: 'right' })
    const menu = page.locator('[data-cut-clip-menu]').first()
    await menu.waitFor({ state: 'visible', timeout: 8_000 })
    return menu
  }

  async function openLane(page, trackId, ratio = 0.62) {
    await closeOverlays(page)
    const lane = page.locator(`[data-cut-track="${trackId}"] .tl-lane`).first()
    await lane.scrollIntoViewIfNeeded()
    const box = await lane.boundingBox()
    if (!box) throw new Error(`timeline lane ${trackId} has no bounding box`)
    await lane.click({ button: 'right', position: { x: Math.max(8, Math.min(box.width - 8, box.width * ratio)), y: Math.max(6, box.height / 2) } })
    const menu = page.locator('[data-cut-timeline-empty-menu]').first()
    await menu.waitFor({ state: 'visible', timeout: 8_000 })
    return menu
  }

  async function openGap(page, id) {
    await closeOverlays(page)
    const gap = page.locator(`[data-cut-gap="${id}"]`).first()
    await gap.scrollIntoViewIfNeeded()
    await gap.click({ button: 'right' })
    const menu = page.locator('[data-cut-gap-menu]').first()
    await menu.waitFor({ state: 'visible', timeout: 8_000 })
    return menu
  }

  async function copy(page, id) {
    const menu = await openClip(page, id)
    await menu.locator('[data-cut-ctx="copy"]').click()
    await page.locator('[data-cut-clip-menu]').waitFor({ state: 'hidden', timeout: 4_000 })
  }

  async function addTrack(kind, rationale) {
    const response = await verb('edit.add_track', { kind, rationale })
    const id = response?.result?.track_id || response?.result?.id
    if (!response?.ok || !id) throw new Error(`could not add ${kind} fixture track`)
    await waitForState((project) => !!track(project, id), 8_000)
    return id
  }

  async function removeSourceClip(source, rationale) {
    const response = await verb('edit.ripple_delete', {
      track: source.trackId, range_ms: [0, clipSpan(source)], ripple: false, rationale,
    })
    if (!response?.ok) throw new Error(`${rationale} could not delete copied source`)
    const removed = await waitForState((project) => !clips(project).some((clip) => clip.id === source.id), 10_000)
    if (!removed) throw new Error(`${rationale} source clip remained on the timeline`)
    return removed
  }

  async function makeUnavailableClipboard(page, tag) {
    const fixturePath = process.env.RELEASE_CLIP2
      || (process.env.CUT_TEST_MEDIA_DIR ? join(process.env.CUT_TEST_MEDIA_DIR, 'silent_screen.mp4') : '')
    if (!fixturePath) throw new Error(`${tag} needs RELEASE_CLIP2 or CUT_TEST_MEDIA_DIR/silent_screen.mp4 for a distinct asset`)
    const initial = await current()
    const base = media(initial)
    const imported = await verb('media.import', { path: fixturePath, proxy: false, rationale: `fcv: ${tag} distinct stale clipboard asset` })
    const asset = imported?.result?.asset_id || ''
    if (!imported?.ok || !asset || asset === base?.asset) throw new Error(`${tag} did not import a distinct asset`)
    const available = await waitForState((project) => !!project?.assets?.[asset], 10_000)
    if (!available) throw new Error(`${tag} distinct asset did not reach project state`)
    const sourceTrack = await addTrack('video', `fcv: ${tag} stale source track`)
    const inserted = await verb('edit.insert', {
      asset, track: sourceTrack, at_ms: 0, src_range_ms: [0, 600], ripple: false,
      rationale: `fcv: ${tag} stale source clip`,
    })
    if (!inserted?.ok) throw new Error(`${tag} could not place distinct stale source`)
    const project = await waitForState((next) => clips(next).find((clip) => clip.trackId === sourceTrack && clip.asset === asset)?.id, 10_000)
    const source = clips(project).find((clip) => clip.trackId === sourceTrack && clip.asset === asset)
    if (!source?.id) throw new Error(`${tag} distinct source did not render`)
    await copy(page, source.id)
    await removeSourceClip(source, `fcv: ${tag} delete source before asset removal`)
    const removedAsset = await verb('media.remove', { asset, rationale: `fcv: ${tag} invalidate copied asset` })
    if (!removedAsset?.ok) throw new Error(`${tag} could not remove copied asset: ${responseError(removedAsset) || 'unknown error'}`)
    const unavailable = await waitForState((next) => !next?.assets?.[asset], 10_000)
    if (!unavailable) throw new Error(`${tag} removed asset remains available in project state`)
    return { asset, sourceId: source.id }
  }

  async function contextFixture(page, tag) {
    await freshProject(page, tag)
    await closeOverlays(page)
    const initial = media(await current())
    if (!initial) throw new Error(`${tag} lacks a video fixture`)
    const span = Math.max(1, initial.src_out_ms - initial.src_in_ms)
    for (const atMs of [Math.round(span / 3), Math.round((span * 2) / 3)]) {
      const response = await verb('edit.split', { track: initial.trackId, at_ms: atMs, rationale: `fcv: ${tag} contiguous context clip` })
      if (!response?.ok) throw new Error(`${tag} could not split contiguous context clips at ${atMs}ms`)
    }
    const rendered = await waitForState((project) => clips(project, 'video')
      .filter((clip) => clip.trackId === initial.trackId && clip.asset).length >= 3, 10_000)
    const all = clips(rendered, 'video').filter((clip) => clip.trackId === initial.trackId && clip.asset)
    if (all.length < 3) throw new Error(`${tag} needs three video clips, found ${all.length}`)
    return { a: all[0].id, b: all[1].id, c: all[2].id }
  }

  async function gapFixture(page, tag, sourceSpan) {
    await freshProject(page, tag)
    await closeOverlays(page)
    const base = media(await current())
    if (!base) throw new Error(`${tag} lacks a video fixture`)
    const removed = await verb('edit.ripple_delete', { track: base.trackId, range_ms: [900, 1500], ripple: false, rationale: `fcv: ${tag} exact 600ms gap` })
    if (!removed?.ok) throw new Error(`${tag} could not create gap`)
    const sourceTrack = await addTrack('video', `fcv: ${tag} source track`)
    const seeded = await verb('edit.insert', { asset: base.asset, track: sourceTrack, at_ms: 0, src_range_ms: [0, sourceSpan], ripple: false, rationale: `fcv: ${tag} ${sourceSpan}ms source` })
    if (!seeded?.ok) throw new Error(`${tag} could not seed ${sourceSpan}ms source`)
    const ready = await waitForState((project) => {
      const source = clips(project).find((clip) => clip.trackId === sourceTrack && !!clip.asset)
      return !!source?.id
    }, 10_000)
    if (!ready) throw new Error(`${tag} fixture did not render source`)
    const source = clips(ready).find((clip) => clip.trackId === sourceTrack && !!clip.asset)
    const gap = page.locator(`[data-cut-track="${base.trackId}"] [data-cut-gap]`).first()
    await gap.waitFor({ state: 'visible', timeout: 10_000 })
    const gapId = await gap.getAttribute('data-cut-gap')
    if (!gapId || !source?.id) throw new Error(`${tag} fixture rendered gap/source without an exact id`)
    return { baseTrack: base.trackId, gapId, sourceId: source.id }
  }

  async function undoToGap(page, gapId) {
    const response = await captureVerbResp(page, 'project.undo', () => page.keyboard.press('Control+z'), 12_000)
    const restored = await page.locator(`[data-cut-gap="${gapId}"]`).first()
      .waitFor({ state: 'visible', timeout: 10_000 }).then(() => true).catch(() => false)
    return !!response?.ok && restored
  }

  async function runClipMatchFrame(page) {
    await freshProject(page, 'audit-clip-match-frame')
    await closeOverlays(page)
    const source = media(await current())
    const sourceInMs = Number(source?.src_in_ms)
    const sourceOutMs = Number(source?.src_out_ms)
    const sourceSpanMs = sourceOutMs - sourceInMs
    if (!source?.id || !source.asset || !Number.isFinite(sourceInMs) || !Number.isFinite(sourceOutMs) || sourceSpanMs < 3) {
      throw new Error('clip Match Frame fixture lacks a finite video source range')
    }
    const requestedFrameAtMs = Math.max(1, Math.min(900, Math.floor(sourceSpanMs / 3)))
    const anchored = await verb('ui.playhead', { at_ms: requestedFrameAtMs })
    const uiAfterAnchor = await verb('ui.state', {})
    const frameAtMs = Number(uiAfterAnchor?.result?.playhead_ms)
    if (!anchored?.ok || frameAtMs !== requestedFrameAtMs) {
      throw new Error(`clip Match Frame could not anchor playhead: ${JSON.stringify({ anchored, frameAtMs, requestedFrameAtMs })}`)
    }
    const expectedSourceMs = sourceInMs + frameAtMs
    await sleep(300)
    const menu = await openClip(page, source.id)
    const control = menu.locator('[data-cut-ctx="match-frame"]')
    await probe(page, {
      surface,
      name: 'ctx-match-frame',
      actionId: 'match-frame',
      sel: control,
      group: menu,
      groupName: 'ctx-clip-match-frame',
      doClick: async () => {
        await control.click()
        await page.locator(`[data-cut-source-monitor="${source.asset}"]`).waitFor({ state: 'visible', timeout: 8_000 })
      },
      assertResult: async () => proveSourceMonitorFrame(page, {
        assetId: source.asset,
        expectedSourceMs,
      }),
    })
  }

  async function runContextOwnership(page) {
    const fixture = await contextFixture(page, 'audit-context-ownership')
    await page.locator(`[data-cut-clip="${fixture.a}"]`).click()
    const bMenu = await openClip(page, fixture.b)
    const bOwns = JSON.stringify(await selected()) === JSON.stringify([fixture.b])
    const bFocused = await bMenu.evaluate((node) => document.activeElement === node)
    const deleted = await captureVerbResp(page, 'edit.ripple_delete', () => bMenu.locator('[data-cut-ctx="remove"]').click(), 12_000)
    const actionDismissed = await dismissed(page, '[data-cut-clip-menu]')
    const bRemovedAUnchanged = !!(await waitForState((project) => !clips(project).some((clip) => clip.id === fixture.b)
      && clips(project).some((clip) => clip.id === fixture.a), 10_000))
    const restored = await captureVerbResp(page, 'project.undo', () => page.keyboard.press('Control+z'), 12_000)
    const bRestored = !!(await waitForState((project) => clips(project).some((clip) => clip.id === fixture.b), 10_000))
    const escapeMenu = await openClip(page, fixture.b)
    const escapeFocused = await escapeMenu.evaluate((node) => document.activeElement === node)
    await page.keyboard.press('Escape')
    const escapeDismissed = await dismissed(page, '[data-cut-clip-menu]')
    const outsideMenu = await openClip(page, fixture.b)
    const outsideFocused = await outsideMenu.evaluate((node) => document.activeElement === node)
    await page.locator('[data-cut-ctx-backdrop]').click({ position: { x: 8, y: 8 } })
    const outsideDismissed = await dismissed(page, '[data-cut-clip-menu]')
    await page.locator(`[data-cut-clip="${fixture.a}"]`).click()
    const pairSelected = await selectClipPair(page, fixture.a, fixture.b)
    let beforeNest = await selected()
    for (let attempt = 0; pairSelected && attempt < 20 && !(beforeNest.includes(fixture.a) && beforeNest.includes(fixture.b)); attempt += 1) {
      await sleep(100)
      beforeNest = await selected()
    }
    const nestMenu = await openClip(page, fixture.b, { preserveSelection: true })
    const retained = pairSelected && JSON.stringify(await selected()) === JSON.stringify(beforeNest)
      && beforeNest.includes(fixture.a) && beforeNest.includes(fixture.b)
    const nestControl = nestMenu.locator('[data-cut-ctx="nest"]')
    const nestEnabled = await nestControl.isEnabled()
    const nest = nestEnabled
      ? await captureVerbResp(page, 'edit.nest', () => nestControl.click(), 12_000)
      : { ok: false }
    const nested = nestEnabled && !!(await waitForState((project) => !clips(project).some((clip) => clip.id === fixture.a || clip.id === fixture.b), 10_000))
    const cMenu = await openClip(page, fixture.c)
    const cOwns = JSON.stringify(await selected()) === JSON.stringify([fixture.c])
    const cFocused = await cMenu.evaluate((node) => document.activeElement === node)
    await page.keyboard.press('Escape')
    return { ok: bOwns && bFocused && !!deleted?.ok && bRemovedAUnchanged && actionDismissed && !!restored?.ok && bRestored && escapeFocused && escapeDismissed && outsideFocused && outsideDismissed && retained && !!nest?.ok && nested && cOwns && cFocused,
      detail: `B=${bOwns}; remove=${deleted?.ok}/${bRemovedAUnchanged}/${restored?.ok}/${bRestored}; focus=${bFocused}/${escapeFocused}/${outsideFocused}; dismiss=${actionDismissed}/${escapeDismissed}/${outsideDismissed}; A+B=${retained}; nest=${nestEnabled}/${nest?.ok}/${nested}; C=${cOwns}/${cFocused}` }
  }

  async function runEmptyPaste(page) {
    await freshProject(page, 'audit-empty-paste')
    await closeOverlays(page)
    const source = media(await current())
    if (!source) throw new Error('empty paste fixture lacks source')
    const targetTrack = await addTrack('video', 'fcv: empty paste target')
    await copy(page, source.id)
    const menu = await openLane(page, targetTrack)
    const targetAt = Number(await menu.getAttribute('data-cut-timeline-context-at-ms'))
    const targetId = await menu.getAttribute('data-cut-timeline-context-track')
    const before = track(await current(), targetTrack)?.clips?.length || 0
    let request = null
    const watch = (event) => { if (/\/api\/verb\/edit[.]paste$/.test(event.url())) { try { request = event.postDataJSON() } catch {} } }
    page.on('request', watch)
    const response = await captureVerbResp(page, 'edit.paste', () => menu.locator('[data-cut-timeline-ctx="empty-paste"]').click(), 12_000)
    page.off('request', watch)
    const inserted = await waitForState((project) => (track(project, targetTrack)?.clips?.length || 0) > before, 10_000)
    const exact = targetId === targetTrack && Number.isFinite(targetAt) && request?.to_track === targetTrack && request?.at_ms === targetAt
    await freshProject(page, 'audit-empty-paste-incompatible')
    const incompatible = media(await current())
    const audioTrack = await addTrack('audio', 'fcv: incompatible empty paste target')
    await copy(page, incompatible.id)
    const incompatibleBefore = snapshot(await current())
    const incompatibleMenu = await openLane(page, audioTrack)
    const incompatibleControl = incompatibleMenu.locator('[data-cut-timeline-ctx="empty-paste"]')
    const incompatibleReason = await incompatibleControl.getAttribute('aria-description') || ''
    const incompatibleClosed = await incompatibleControl.isDisabled() && /video track/i.test(incompatibleReason) && snapshot(await current()) === incompatibleBefore
    await freshProject(page, 'audit-empty-paste-source-deleted')
    const deletedSource = media(await current())
    const deletedTarget = await addTrack('video', 'fcv: source-deleted empty paste target')
    await copy(page, deletedSource.id)
    await removeSourceClip(deletedSource, 'fcv: source-deleted snapshot regression')
    const sourceDeletedBefore = track(await current(), deletedTarget)?.clips?.length || 0
    const sourceDeletedMenu = await openLane(page, deletedTarget)
    const sourceDeletedPaste = await captureVerbResp(page, 'edit.paste', () => sourceDeletedMenu.locator('[data-cut-timeline-ctx="empty-paste"]').click(), 12_000)
    const sourceDeletedSnapshot = !!sourceDeletedPaste?.ok && !!(await waitForState((project) => (track(project, deletedTarget)?.clips?.length || 0) > sourceDeletedBefore, 10_000))
    await freshProject(page, 'audit-empty-paste-unavailable')
    const unavailableTarget = await addTrack('video', 'fcv: unavailable empty paste target')
    await makeUnavailableClipboard(page, 'audit-empty-paste-unavailable')
    const unavailableBefore = snapshot(await current())
    const unavailableMenu = await openLane(page, unavailableTarget)
    const unavailablePaste = unavailableMenu.locator('[data-cut-timeline-ctx="empty-paste"]')
    const unavailableResponse = await captureVerbResp(page, 'edit.paste', () => unavailablePaste.click(), 12_000)
    const unavailableError = responseError(unavailableResponse)
    const unavailableAlert = await pasteRefusalAlert(page, unavailableResponse)
    const unavailableClosed = !unavailableResponse?.ok && /no asset|not .*imported/i.test(unavailableError) && unavailableAlert && snapshot(await current()) === unavailableBefore
    return { ok: !!response?.ok && !!inserted && exact && incompatibleClosed && sourceDeletedSnapshot && unavailableClosed,
      detail: `paste=${response?.ok}; exact=${exact}; inserted=${!!inserted}; incompatible=${incompatibleClosed}(${incompatibleReason || 'none'}); sourceDeletedSnapshot=${sourceDeletedSnapshot}; unavailableAsset=${unavailableClosed}/${unavailableAlert}(${unavailableError || 'none'})` }
  }

  async function runGapBoundary(page) {
    const valid = async (factor) => {
      const sourceSpan = factor === 0.25 ? 150 : 2400
      const pasteFixture = await gapFixture(page, `audit-gap-paste-${factor}`, sourceSpan)
      await copy(page, pasteFixture.sourceId)
      let menu = await openGap(page, pasteFixture.gapId)
      const pasteBefore = track(await current(), pasteFixture.baseTrack)?.clips?.length || 0
      const pasted = await captureVerbResp(page, 'edit.paste', () => menu.locator('[data-cut-timeline-ctx="gap-paste"]').click(), 12_000)
      const pasteVisible = !!(await waitForState((project) => (track(project, pasteFixture.baseTrack)?.clips?.length || 0) > pasteBefore, 10_000))
      const pasteUndo = await undoToGap(page, pasteFixture.gapId)
      const fitFixture = await gapFixture(page, `audit-gap-fit-${factor}`, sourceSpan)
      await copy(page, fitFixture.sourceId)
      menu = await openGap(page, fitFixture.gapId)
      const fitted = await captureVerbResp(page, 'edit.fit_to_fill', () => menu.locator('[data-cut-timeline-ctx="gap-fit-clipboard"]').click(), 12_000)
      const fitVisible = await page.locator(`[data-cut-gap="${fitFixture.gapId}"]`).first()
        .waitFor({ state: 'hidden', timeout: 10_000 }).then(() => true).catch(() => false)
      const fitUndo = await undoToGap(page, fitFixture.gapId)
      return !!pasted?.ok && pasteVisible && pasteUndo && !!fitted?.ok && fitVisible && fitUndo
    }
    const invalid = async (tag, sourceSpan, wrongKind = false) => {
      const fixture = await gapFixture(page, `audit-gap-${tag}`, sourceSpan)
      if (wrongKind) {
        let audio = media(await current(), 'audio')
        if (!audio) {
          const video = media(await current())
          const audioTrack = await addTrack('audio', 'fcv: wrong-kind gap source')
          const seeded = await verb('edit.insert', { asset: video?.asset, track: audioTrack, at_ms: 0, src_range_ms: [0, 600], ripple: false, rationale: 'fcv: wrong-kind gap source' })
          if (!seeded?.ok) throw new Error('could not seed wrong-kind audio source')
          const rendered = await waitForState((project) => !!media(project, 'audio'), 8_000)
          audio = media(rendered, 'audio')
        }
        if (!audio) throw new Error('wrong-kind fixture lacks audio clip')
        await copy(page, audio.id)
      } else {
        await copy(page, fixture.sourceId)
      }
      const before = snapshot(await current())
      const menu = await openGap(page, fixture.gapId)
      const fit = menu.locator('[data-cut-timeline-ctx="gap-fit-clipboard"]')
      const paste = menu.locator('[data-cut-timeline-ctx="gap-paste"]')
      const fitDisabled = await fit.isDisabled()
      const pasteDisabled = wrongKind ? await paste.isDisabled() : true
      return fitDisabled && pasteDisabled && snapshot(await current()) === before
    }
    const unavailable = async () => {
      const fixture = await gapFixture(page, 'audit-gap-unavailable', 600)
      await makeUnavailableClipboard(page, 'audit-gap-unavailable')
      const before = snapshot(await current())
      const menu = await openGap(page, fixture.gapId)
      const fit = menu.locator('[data-cut-timeline-ctx="gap-fit-clipboard"]')
      const paste = menu.locator('[data-cut-timeline-ctx="gap-paste"]')
      const fitReason = await fit.getAttribute('aria-description') || ''
      const fitDisabled = await fit.isDisabled()
      const response = await captureVerbResp(page, 'edit.paste', () => paste.click(), 12_000)
      const error = responseError(response)
      const alert = await pasteRefusalAlert(page, response)
      return { ok: fitDisabled && /still on the timeline|copy/i.test(fitReason) && !response?.ok
        && /no asset|not .*imported/i.test(error) && alert && snapshot(await current()) === before,
        detail: `fit=${fitDisabled}(${fitReason || 'none'}); paste=${response?.ok}; alert=${alert}; error=${error || 'none'}` }
    }
    const min = await valid(0.25)
    const max = await valid(4)
    const tooSlow = await invalid('too-slow', 149)
    const tooFast = await invalid('too-fast', 2401)
    const wrongKind = await invalid('wrong-kind', 600, true)
    const unavailableAsset = await unavailable()
    return { ok: min && max && tooSlow && tooFast && wrongKind && unavailableAsset.ok,
      detail: `0.25=${min}; 4=${max}; tooSlow=${tooSlow}; tooFast=${tooFast}; wrongKind=${wrongKind}; unavailableAsset=${unavailableAsset.ok}(${unavailableAsset.detail})` }
  }

  async function run(page, { only = '' } = {}) {
    if (!only || matches('ctx-match-frame', only)) await runClipMatchFrame(page)
    const scenarios = [
      ['e2e-context-ownership-01', runContextOwnership],
      ['e2e-empty-paste-01', runEmptyPaste],
      ['e2e-gap-boundary-01', runGapBoundary],
    ]
    for (const [id, execute] of scenarios) {
      if (!matches(id, only)) continue
      await probe(page, {
        surface, name: id, actionId: id, rowKind: 'support',
        sel: page.locator('[data-cut-panel="timeline"]').first(), group: page.locator('[data-cut-panel="timeline"]').first(), groupName: id,
        doClick: async () => { probe[`_${id}`] = await execute(page) },
        assertResult: async () => probe[`_${id}`],
      })
    }
  }

  return { run }
}
