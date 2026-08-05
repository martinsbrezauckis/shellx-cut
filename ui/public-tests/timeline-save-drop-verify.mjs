// timeline-save-drop-verify.mjs - focused Timeline result proof for
// Save to Assets, GIF, and the custom Assets -> Timeline drop bridge.
//
// RUN:
//   SWEEP_CUTD=http://127.0.0.1:6161 SWEEP_APP=http://127.0.0.1:6161 \
//     node ui/public-tests/timeline-save-drop-verify.mjs

import { chromium } from 'playwright'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { resolveDriverPath } from '../../scripts/lib/cross-host-media.mjs'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '..', '..')
const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6161'
const APP = process.env.SWEEP_APP || CUTD
const CLIP = process.env.RELEASE_CLIP || join(REPO, 'testdata', 'talking_head.mp4')
const RECEIPT = process.env.CUT_RECEIPT || ''
const VERB_TIMEOUT_MS = Number(process.env.VERB_TIMEOUT_MS || 120000)

const results = []
const evidence = {
  app: APP,
  cutd: CUTD,
  clip: CLIP,
  project: '',
  importedAsset: '',
  range: {},
  gif: {},
  drop: {},
}

function check(name, ok, detail = '') {
  const item = { name, ok: !!ok, detail }
  results.push(item)
  console.log(`${item.ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` - ${detail}` : ''}`)
}

async function verb(name, args = {}, timeoutMs = VERB_TIMEOUT_MS) {
  const r = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:timeline-save-drop-verify' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(timeoutMs),
  })
  return r.json()
}

async function state() {
  return (await verb('project.state')).result
}

function flatClips(project) {
  return (project?.tracks || []).flatMap((track) => (track.clips || []).map((clip) => ({ ...clip, _track: track.id, _kind: track.kind })))
}

function videoClips(project) {
  return flatClips(project).filter((clip) => clip._kind === 'video' && clip.asset)
}

function assetExists(project, assetId) {
  return !!assetId && !!project?.assets?.[assetId]
}

function driverPath(path) {
  return resolveDriverPath(String(path || ''))
}

function fileLooksReal(path, expectedMagic = null) {
  if (!existsSync(driverPath(path))) return false
  const bytes = readFileSync(driverPath(path))
  if (bytes.length <= 16) return false
  if (expectedMagic === 'GIF') return bytes.slice(0, 3).toString('ascii') === 'GIF'
  return true
}

async function waitJobs(maxS = 180) {
  for (let i = 0; i < maxS * 2; i++) {
    const js = (await verb('jobs.list')).result?.jobs || []
    if (!js.some((j) => j.state === 'queued' || j.state === 'running')) return i * 0.5
    await new Promise((r) => setTimeout(r, 500))
  }
  return -1
}

async function waitForState(pred, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await state()
    if (pred(last)) return last
    await new Promise((r) => setTimeout(r, 250))
  }
  throw new Error(`state condition did not become true; last=${JSON.stringify(last)?.slice(0, 700)}`)
}

async function captureVerbResp(page, name, act, timeoutMs = VERB_TIMEOUT_MS) {
  const wait = page.waitForResponse((resp) => resp.url().includes(`/api/verb/${name}`), { timeout: timeoutMs })
  await act()
  const resp = await wait
  return resp.json()
}

async function selectFirstVideoClip(page, clipId) {
  const clip = page.locator(`[data-cut-clip="${clipId}"]`).first()
  await clip.waitFor({ state: 'visible', timeout: 12000 })
  await clip.scrollIntoViewIfNeeded()
  await clip.click({ force: true })
  await page.waitForFunction(() => {
    const btn = document.querySelector('[data-cut-action="save-range"]')
    return btn instanceof HTMLButtonElement && !btn.disabled
  }, null, { timeout: 8000 })
}

async function main() {
  const tmp = mkdtempSync(join(tmpdir(), 'shellx-cut-timeline-save-drop-'))
  const suffix = Math.random().toString(36).slice(2, 8)
  const name = `timeline_save_drop_${suffix}`
  const projectDir = join(tmp, `${name}.cutproj`)
  evidence.project = projectDir

  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })

  try {
    check('fixture-present', existsSync(CLIP), CLIP)
    if (!existsSync(CLIP)) throw new Error(`missing fixture ${CLIP}`)

    const created = await verb('project.create', {
      name,
      dir: projectDir,
      settings: { width: 1280, height: 720, fps: 30 },
    })
    check('project.create', created?.ok === true, created?.ok ? projectDir : JSON.stringify(created?.error ?? created).slice(0, 240))
    if (created?.ok !== true) throw new Error('cannot continue without project')

    const imported = await verb('media.import', { path: CLIP, proxy: false, rationale: 'timeline save/drop verifier seed' })
    const asset = imported?.result?.asset_id
    evidence.importedAsset = asset
    check('media.import', imported?.ok === true && !!asset, imported?.ok ? String(asset) : JSON.stringify(imported?.error ?? imported).slice(0, 240))
    if (imported?.ok !== true || !asset) throw new Error('cannot continue without imported asset')
    await waitJobs()

    await page.goto(APP, { waitUntil: 'domcontentloaded' })
    await page.locator('[data-cut-panel="timeline"]').waitFor({ state: 'visible', timeout: 12000 })
    await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
    await page.waitForTimeout(500)

    const seeded = await waitForState((s) => videoClips(s).some((clip) => clip.asset === asset))
    const baseClip = videoClips(seeded).find((clip) => clip.asset === asset)
    check('timeline-seed-video-visible-in-state', !!baseClip, baseClip ? `${baseClip.id} on ${baseClip._track}` : '')
    if (!baseClip) throw new Error('media.import did not place a video clip')

    await selectFirstVideoClip(page, baseClip.id)

    const beforeRange = await state()
    const beforeAssets = Object.keys(beforeRange.assets || {}).length
    const range = await captureVerbResp(page, 'export.range', async () => {
      await page.locator('[data-cut-action="save-range"]').click()
    })
    await waitJobs()
    const afterRange = await waitForState((s) => assetExists(s, range?.result?.asset_id))
    evidence.range = {
      path: range?.result?.path,
      asset_id: range?.result?.asset_id,
      assetCountBefore: beforeAssets,
      assetCountAfter: Object.keys(afterRange.assets || {}).length,
    }
    check(
      'timeline-save-range-created-asset',
      range?.ok === true
        && !!range.result?.asset_id
        && assetExists(afterRange, range.result.asset_id)
        && Object.keys(afterRange.assets || {}).length > beforeAssets
        && fileLooksReal(range.result.path),
      JSON.stringify(evidence.range),
    )

    const beforeGif = await state()
    const beforeGifAssets = Object.keys(beforeGif.assets || {}).length
    const gif = await captureVerbResp(page, 'export.gif', async () => {
      await page.locator('[data-cut-action="save-gif"]').click()
    })
    await waitJobs()
    const afterGif = await waitForState((s) => assetExists(s, gif?.result?.asset_id))
    evidence.gif = {
      path: gif?.result?.path,
      asset_id: gif?.result?.asset_id,
      range_ms: gif?.result?.range_ms,
      assetCountBefore: beforeGifAssets,
      assetCountAfter: Object.keys(afterGif.assets || {}).length,
    }
    check(
      'timeline-save-gif-created-asset',
      gif?.ok === true
        && !!gif.result?.asset_id
        && assetExists(afterGif, gif.result.asset_id)
        && Object.keys(afterGif.assets || {}).length > beforeGifAssets
        && fileLooksReal(gif.result.path, 'GIF'),
      JSON.stringify(evidence.gif),
    )

    const beforeDrop = await state()
    const beforeDropVideoTracks = (beforeDrop.tracks || []).filter((t) => t.kind === 'video').length
    const beforeDropClips = flatClips(beforeDrop).length
    const baseVideoBefore = (beforeDrop.tracks || []).find((t) => t.kind === 'video')
    const baseVideoBeforeAssetClips = (baseVideoBefore?.clips || []).filter((clip) => clip.asset === asset).length
    const scroll = page.locator('[data-cut-timeline-scroll]')
    const box = await scroll.boundingBox()
    if (!box) throw new Error('timeline scroll box missing')
    const clientX = Math.round(box.x + Math.min(360, Math.max(220, box.width / 3)))
    const clientY = Math.round(box.y + Math.min(120, Math.max(70, box.height / 4)))
    await page.evaluate(({ asset, clientX, clientY }) => {
      document.dispatchEvent(new CustomEvent('cut:asset-dragmove', { detail: { asset, kind: 'video', clientX, clientY } }))
      document.dispatchEvent(new CustomEvent('cut:asset-drop', { detail: { asset, kind: 'video', clientX, clientY } }))
    }, { asset, clientX, clientY })
    const afterDrop = await waitForState((s) => {
      const videos = (s.tracks || []).filter((t) => t.kind === 'video')
      const baseVideo = (s.tracks || []).find((t) => t.id === baseVideoBefore?.id) || videos[0]
      const baseVideoAssetClips = (baseVideo?.clips || []).filter((clip) => clip.asset === asset).length
      const clips = flatClips(s)
      return videos.length === beforeDropVideoTracks
        && baseVideoAssetClips > baseVideoBeforeAssetClips
        && clips.length > beforeDropClips
    }, 20000)
    const baseVideoAfter = (afterDrop.tracks || []).find((t) => t.id === baseVideoBefore?.id)
      || (afterDrop.tracks || []).find((t) => t.kind === 'video')
    const baseVideoAfterAssetClips = (baseVideoAfter?.clips || []).filter((clip) => clip.asset === asset).length
    evidence.drop = {
      clientX,
      clientY,
      videoTracksBefore: beforeDropVideoTracks,
      videoTracksAfter: (afterDrop.tracks || []).filter((t) => t.kind === 'video').length,
      clipsBefore: beforeDropClips,
      clipsAfter: flatClips(afterDrop).length,
      baseVideoTrack: baseVideoAfter?.id,
      baseAssetClipsBefore: baseVideoBeforeAssetClips,
      baseAssetClipsAfter: baseVideoAfterAssetClips,
    }
    check(
      'timeline-drop-inserted-base-line',
      evidence.drop.videoTracksAfter === beforeDropVideoTracks
        && evidence.drop.baseAssetClipsAfter > evidence.drop.baseAssetClipsBefore
        && evidence.drop.clipsAfter > evidence.drop.clipsBefore,
      JSON.stringify(evidence.drop),
    )

    const beforeAltDrop = await state()
    const beforeAltDropVideoTracks = (beforeAltDrop.tracks || []).filter((t) => t.kind === 'video').length
    const beforeAltDropClips = flatClips(beforeAltDrop).length
    await page.evaluate(({ asset, clientX, clientY }) => {
      document.dispatchEvent(new CustomEvent('cut:asset-dragmove', { detail: { asset, kind: 'video', clientX, clientY, alt: true } }))
      document.dispatchEvent(new CustomEvent('cut:asset-drop', { detail: { asset, kind: 'video', clientX, clientY, alt: true } }))
    }, { asset, clientX, clientY })
    const afterAltDrop = await waitForState((s) => {
      const videos = (s.tracks || []).filter((t) => t.kind === 'video')
      const clips = flatClips(s)
      return videos.length > beforeAltDropVideoTracks && clips.length > beforeAltDropClips
    }, 20000)
    const newVideoTrack = (afterAltDrop.tracks || []).find((track) => (
      track.kind === 'video'
      && !(beforeAltDrop.tracks || []).some((old) => old.id === track.id)
      && (track.clips || []).some((clip) => clip.asset === asset)
    ))
    evidence.altDrop = {
      clientX,
      clientY,
      videoTracksBefore: beforeAltDropVideoTracks,
      videoTracksAfter: (afterAltDrop.tracks || []).filter((t) => t.kind === 'video').length,
      clipsBefore: beforeAltDropClips,
      clipsAfter: flatClips(afterAltDrop).length,
      newVideoTrack: newVideoTrack?.id,
    }
    check(
      'timeline-alt-drop-created-overlay-line',
      !!newVideoTrack && evidence.altDrop.videoTracksAfter > beforeAltDropVideoTracks && evidence.altDrop.clipsAfter > beforeAltDropClips,
      JSON.stringify(evidence.altDrop),
    )
  } finally {
    await browser.close().catch(() => {})
    await verb('project.close', {}).catch(() => {})
    await verb('project.delete', { path: projectDir }).catch(() => {})
    await verb('project.forget', { path: projectDir }).catch(() => {})
    rmSync(tmp, { recursive: true, force: true })
  }

  const fail = results.filter((r) => !r.ok).length
  const pass = results.length - fail
  const receipt = { pass, fail, results, evidence }
  if (RECEIPT) writeFileSync(RECEIPT, `${JSON.stringify(receipt, null, 2)}\n`)
  console.log(`SUMMARY pass=${pass} fail=${fail}`)
  if (fail) process.exit(1)
}

main().catch((error) => {
  console.error(error?.stack || String(error))
  if (RECEIPT) writeFileSync(RECEIPT, `${JSON.stringify({ pass: 0, fail: 1, error: String(error?.stack || error), results, evidence }, null, 2)}\n`)
  process.exit(1)
})
