// Receipt-backed relink derivation regression. The file picker is native-only,
// so this exercises the same public media.relink verb that picker owns and
// verifies the reflected browser state after reopening the real project.

import { copyFileSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'

export const RELINK_DERIVATION_ACTION_NAMES = Object.freeze([
  'e2e-relink-derivation-01-same-content-preserves-derived-state',
  'e2e-relink-derivation-01-changed-content-invalidates-deterministic-media-rederives-reopens',
])

function assetState(project, assetId) {
  const asset = project?.assets?.[assetId] || {}
  return {
    hash: asset.hash || '', probe: asset.probe || null, proxy: asset.proxy || null,
    filmstrip: asset.filmstrip || null, transcript: asset.transcript || null,
    perception: asset.perception || null,
  }
}

function timelineRefs(project, assetId) {
  return (project?.tracks || []).flatMap((track) => (track.clips || [])
    .filter((clip) => clip.asset === assetId)
    .map((clip) => clip.id)).sort()
}

function sameDerived(left, right) {
  return ['probe', 'proxy', 'filmstrip', 'transcript', 'perception']
    .every((key) => JSON.stringify(left[key]) === JSON.stringify(right[key]))
}

async function browserAssetEvidence(page, assetId, sleep) {
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
  await page.locator('[data-cut-left-tab="assets"]').click().catch(() => {})
  const panel = page.locator('[data-cut-panel="assets"]').first()
  await panel.waitFor({ state: 'visible', timeout: 12_000 }).catch(() => {})
  const card = page.locator(`[data-cut-asset-card="${assetId}"]`).first()
  await card.waitFor({ state: 'visible', timeout: 12_000 }).catch(() => {})
  await sleep(200)
  return {
    present: (await card.count()) > 0,
    visible: await card.isVisible().catch(() => false),
    title: await card.getAttribute('title').catch(() => ''),
  }
}

export function createRelinkLifecycleFixture({ driverDir, engineDir, ffmpeg, joinHostPath, nextName }) {
  const stem = `e2e-relink-${nextName()}`
  const originalDriver = join(driverDir, `${stem}-original.mp4`)
  const sameDriver = join(driverDir, `${stem}-same.mp4`)
  const changedDriver = join(driverDir, `${stem}-changed.mp4`)
  const make = (path, color, frequency, duration) => spawnSync(ffmpeg, [
    '-hide_banner', '-loglevel', 'error', '-y',
    '-f', 'lavfi', '-i', `color=c=${color}:s=320x180:r=30:d=${duration}`,
    '-f', 'lavfi', '-i', `sine=frequency=${frequency}:sample_rate=48000:duration=${duration}`,
    '-shortest', '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-c:a', 'aac', path,
  ], { timeout: 60_000 })
  if (make(originalDriver, 'navy', 440, 2).status !== 0 || !existsSync(originalDriver)) return null
  copyFileSync(originalDriver, sameDriver)
  if (make(changedDriver, 'darkred', 660, 3).status !== 0 || !existsSync(changedDriver)) return null
  return {
    originalEngine: joinHostPath(engineDir, `${stem}-original.mp4`),
    sameEngine: joinHostPath(engineDir, `${stem}-same.mp4`),
    changedEngine: joinHostPath(engineDir, `${stem}-changed.mp4`),
  }
}

export async function runRelinkDerivationCoverage(page, {
  projectPath,
  assetId,
  fixture,
  verb,
  state,
  waitForState,
  awaitJob,
  ensureNonEmptyTranscript,
  reloadApp,
  sleep,
  record,
}) {
  const transcript = await ensureNonEmptyTranscript(page, projectPath, assetId, 'e2e relink retained transcript contract')
  const beforeProject = await waitForState((value) => {
    const asset = assetState(value, assetId)
    return !!asset.probe && !!asset.proxy && !!asset.transcript
  }, 120_000)
  const before = assetState(beforeProject || await state(), assetId)
  const refsBefore = timelineRefs(beforeProject || await state(), assetId)
  const same = fixture
    ? await verb('media.relink', { asset: assetId, path: fixture.sameEngine, rationale: 'e2e same-content relink' })
    : { ok: false, error: { message: 'fixture generation failed' } }
  const sameProject = await waitForState((value) => assetState(value, assetId).hash === before.hash, 20_000)
  const afterSame = assetState(sameProject || await state(), assetId)
  const refsAfterSame = timelineRefs(sameProject || await state(), assetId)
  const sameBrowser = await browserAssetEvidence(page, assetId, sleep)
  const retained = !!(sameDerived(before, afterSame) && before.proxy && before.transcript)
  const sameRefs = JSON.stringify(refsBefore) === JSON.stringify(refsAfterSame)
  record('residual', RELINK_DERIVATION_ACTION_NAMES[0], {
    rowKind: 'support', actionId: RELINK_DERIVATION_ACTION_NAMES[0],
    present: sameBrowser.present ? 'pass' : 'fail', render: sameBrowser.visible ? 'pass' : 'fail', click: 'na',
    result: same.ok && same.result?.hash_changed === false && same.result?.derived_cleared === false && retained && sameRefs ? 'pass' : 'fail',
  }, `same relink ok=${same.ok}; hash_changed=${same.result?.hash_changed}; derived_cleared=${same.result?.derived_cleared}; initial-proxy=${!!before.proxy}; seeded-transcript=${transcript.seeded}; retained=${retained}; stable-refs=${sameRefs} refs=${refsBefore.length}`)

  const changed = fixture
    ? await verb('media.relink', { asset: assetId, path: fixture.changedEngine, rationale: 'e2e changed-content relink' })
    : { ok: false, error: { message: 'fixture generation failed' } }
  const invalidatedProject = await waitForState((value) => {
    const asset = assetState(value, assetId)
    return asset.hash !== before.hash && !asset.probe && !asset.proxy && !asset.filmstrip
      && !asset.transcript && !asset.perception
  }, 20_000)
  const importTerminal = changed.result?.job_id ? await awaitJob(changed.result.job_id, 120_000) : null
  const proxyTerminal = importTerminal?.result?.proxy_job
    ? await awaitJob(importTerminal.result.proxy_job, 120_000)
    : null
  const regeneratedProject = await waitForState((value) => {
    const asset = assetState(value, assetId)
    return !!asset.probe && !!asset.proxy && !!asset.filmstrip
  }, 120_000)
  const afterChanged = assetState(regeneratedProject || await state(), assetId)
  const refsAfterChanged = timelineRefs(regeneratedProject || await state(), assetId)
  const closed = await verb('project.close', {})
  const opened = closed.ok ? await verb('project.open', { path: projectPath }) : null
  if (opened?.ok) await reloadApp(page)
  const reopened = opened?.ok ? await waitForState((value) => !!value?.assets?.[assetId], 30_000) : null
  const afterReopen = assetState(reopened || await state(), assetId)
  const refsAfterReopen = timelineRefs(reopened || await state(), assetId)
  const changedBrowser = await browserAssetEvidence(page, assetId, sleep)
  const invalidated = !!invalidatedProject
  const regenerated = !!regeneratedProject && importTerminal?.state === 'done' && proxyTerminal?.state === 'done'
  const stableRefs = JSON.stringify(refsBefore) === JSON.stringify(refsAfterChanged)
    && JSON.stringify(refsBefore) === JSON.stringify(refsAfterReopen)
  const persisted = opened?.ok === true && !!afterReopen.probe && !!afterReopen.proxy && !!afterReopen.filmstrip
  record('residual', RELINK_DERIVATION_ACTION_NAMES[1], {
    rowKind: 'support', actionId: RELINK_DERIVATION_ACTION_NAMES[1],
    present: changedBrowser.present ? 'pass' : 'fail', render: changedBrowser.visible ? 'pass' : 'fail', click: 'na',
    result: changed.ok && changed.result?.hash_changed === true && changed.result?.derived_cleared === true && invalidated && regenerated && stableRefs && persisted ? 'pass' : 'fail',
  }, `changed relink ok=${changed.ok}; hash_changed=${changed.result?.hash_changed}; derived_cleared=${changed.result?.derived_cleared}; invalidated=${invalidated}; import=${importTerminal?.state || 'missing'} proxy=${proxyTerminal?.state || 'missing'}; probe=${!!afterChanged.probe} proxy=${!!afterChanged.proxy} filmstrip=${!!afterChanged.filmstrip}; transcript=${afterChanged.transcript ? 'rederived' : 'not claimed (optional enrich)'} perception=${afterChanged.perception ? 'rederived' : 'not claimed (optional enrich)'} waveform=not-applicable (no asset waveform pointer); stable-refs=${stableRefs}; reopen=${opened?.ok === true} persisted-media=${persisted}`)
}
