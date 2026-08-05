// generate-templates-verify.mjs - focused UI gate for the native
// Generate templates workspace. This is not the older Find-pane assets.generate
// surface. It proves result evidence for generate.preview and generate.insert.
//
// RUN:
//   SWEEP_CUTD=http://127.0.0.1:6178 SWEEP_APP=http://127.0.0.1:5178 \
//     node public-tests/generate-templates-verify.mjs

import { chromium } from 'playwright'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6178'
const APP = process.env.SWEEP_APP || 'http://127.0.0.1:5178'
const VERB_TIMEOUT_MS = Number(process.env.VERB_TIMEOUT_MS || 60000)

async function verb(name, args = {}) {
  const r = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:generate-templates-verify' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(VERB_TIMEOUT_MS),
  })
  return r.json()
}

async function waitForState(pred, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const s = (await verb('project.state')).result
    last = s
    if (pred(s)) return s
    await new Promise((r) => setTimeout(r, 250))
  }
  throw new Error(`state condition did not become true; last=${JSON.stringify(last)?.slice(0, 500)}`)
}

async function captureVerbResp(page, name, act, timeoutMs = 60000) {
  const escaped = name.replace('.', '\\.')
  const wait = page.waitForResponse(
    (resp) => resp.url().includes(`/api/verb/${name}`) || resp.url().includes(`/api/verb/${escaped}`),
    { timeout: timeoutMs },
  )
  await act()
  const resp = await wait
  return resp.json()
}

function findClip(project, clipId) {
  return (project?.tracks || [])
    .flatMap((track) => (track.clips || []).map((clip) => ({ ...clip, _track: track.id })))
    .find((clip) => clip.id === clipId)
}

async function main() {
  const suffix = Math.random().toString(36).slice(2, 8)
  const name = `genui_${suffix}`
  const dir = join(mkdtempSync(join(tmpdir(), 'shellx-cut-genui-')), `${name}.cutproj`)
  const created = await verb('project.create', {
    name,
    dir,
    settings: { width: 640, height: 360, fps: 24 },
  })
  if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created.error)}`)

  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 920 } })
  const evidence = { project: dir }
  try {
    await page.goto(APP, { waitUntil: 'networkidle' })

    const mode = page.locator('[data-cut-left-tab="generate"]')
    await mode.waitFor({ state: 'visible', timeout: 5000 })
    await mode.click()

    const panel = page.locator('[data-cut-panel="generate-templates"]')
    await panel.waitFor({ state: 'visible', timeout: 8000 })

    const card = page.locator('[data-cut-generate-template-id="builtin.lower-third.clean"]').first()
    await card.waitFor({ state: 'visible', timeout: 8000 })
    evidence.cardCount = await page.locator('[data-cut-generate-template-card]').count()
    if (evidence.cardCount < 1) throw new Error('no Generate template cards rendered')
    await card.click()

    await page.locator('[data-cut-generate-param="name"]').fill('Marta UI')
    await page.locator('[data-cut-generate-param="accent"]').fill('#33CC99').catch(() => {})

    const preview = await captureVerbResp(page, 'generate.preview', async () => {
      await page.locator('[data-cut-generate-template-preview]').click()
    })
    if (!preview.ok) throw new Error(`generate.preview failed: ${JSON.stringify(preview.error)}`)
    if (preview.result?.mime !== 'image/png') throw new Error(`preview mime mismatch: ${preview.result?.mime}`)
    if (!preview.result?.url) throw new Error(`preview did not return a browser URL: ${JSON.stringify(preview.result)}`)
    const img = page.locator('[data-cut-generate-template-preview-img]').first()
    await img.waitFor({ state: 'visible', timeout: 8000 })
    const imgBox = await img.boundingBox()
    if (!imgBox || imgBox.width <= 1 || imgBox.height <= 1) {
      throw new Error(`preview image did not render with a real box: ${JSON.stringify(imgBox)}`)
    }
    evidence.preview = {
      preview_id: preview.result.preview_id,
      url: preview.result.url,
      mime: preview.result.mime,
      width: preview.result.width,
      height: preview.result.height,
      box: { width: Math.round(imgBox.width), height: Math.round(imgBox.height) },
    }

    const beforeOps = ((await verb('project.ops')).result?.ops || []).length
    const insert = await captureVerbResp(page, 'generate.insert', async () => {
      await page.locator('[data-cut-generate-template-insert]').click()
    })
    if (!insert.ok) throw new Error(`generate.insert failed: ${JSON.stringify(insert.error)}`)
    const clipId = insert.result?.clips?.[0]
    const assetId = insert.result?.assets?.[0]
    const checkpointId = insert.result?.checkpoint?.id
    if (!checkpointId || !clipId || !assetId) throw new Error(`insert result missing evidence ids: ${JSON.stringify(insert.result)}`)
    if (insert.result?.lowering?.verb !== 'title.add') throw new Error(`insert lowered to ${insert.result?.lowering?.verb}`)

    const afterState = await waitForState((s) => {
      const clip = findClip(s, clipId)
      return clip?.title_text === 'Marta UI'
    })
    const clip = findClip(afterState, clipId)
    const afterOps = ((await verb('project.ops')).result?.ops || []).length
    if (afterOps <= beforeOps) throw new Error(`op count did not increase: before=${beforeOps} after=${afterOps}`)

    evidence.insert = {
      checkpoint: checkpointId,
      op_ids: insert.result.op_ids,
      clips: insert.result.clips,
      assets: insert.result.assets,
      lowering: insert.result.lowering,
      materialized_clip: {
        id: clip.id,
        asset: clip.asset,
        title_text: clip.title_text,
        track: clip._track,
      },
      ops: { before: beforeOps, after: afterOps },
    }

    const reverted = await verb('project.revert', { to: checkpointId, rationale: 'generate templates verify cleanup' })
    if (!reverted.ok) throw new Error(`project.revert failed: ${JSON.stringify(reverted.error)}`)
    await waitForState((s) => !findClip(s, clipId))
    evidence.revert = { reverted_to: reverted.result?.reverted_to, clip_removed: true }

    console.log(JSON.stringify({ ok: true, evidence }, null, 2))
  } finally {
    await browser.close()
  }
}

main().catch((err) => {
  console.error(err?.stack || err)
  process.exit(1)
})
