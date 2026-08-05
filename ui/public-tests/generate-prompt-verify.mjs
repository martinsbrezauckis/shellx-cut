// generate-prompt-verify.mjs - focused UI gate for the native Generate
// prompt path. It proves result evidence for generate.from_prompt preview and
// insert policies, not just that the Prompt tab can be clicked.
//
// RUN:
//   CUTD_GENERATE_PROMPT_ADAPTER=$PWD/ui/public-tests/fixtures/generate-prompt-adapter.py \
//     target/debug/cutd serve --addr 127.0.0.1:6178 --headless
//   cd ui && SWEEP_CUTD=http://127.0.0.1:6178 SWEEP_APP=http://127.0.0.1:5178 \
//     node public-tests/generate-prompt-verify.mjs

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
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:generate-prompt-verify' },
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
  const name = `genprompt_${suffix}`
  const dir = join(mkdtempSync(join(tmpdir(), 'shellx-cut-genprompt-')), `${name}.cutproj`)
  const created = await verb('project.create', {
    name,
    dir,
    settings: { width: 640, height: 360, fps: 24 },
  })
  if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created.error)}`)

  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 920 } })
  const evidence = { project: dir, prompt: 'Create a clean lower third for Marta' }
  try {
    await page.goto(APP, { waitUntil: 'networkidle' })

    await page.locator('[data-cut-left-tab="generate"]').click()
    await page.waitForSelector('[data-cut-panel="generate-templates"]', { timeout: 8000 })
    await page.waitForSelector('[data-cut-generate-template-card]', { timeout: 8000 })
    await page.locator('[data-cut-generate-template-id="builtin.lower-third.clean"]').first().click()

    const promptTab = page.locator('[data-cut-generate-tab="prompt"]')
    await promptTab.waitFor({ state: 'visible', timeout: 5000 })
    await promptTab.click()

    await page.locator('[data-cut-generate-prompt-input]').fill(evidence.prompt)
    await page.locator('[data-cut-generate-prompt-policy]').selectOption('preview')
    const preview = await captureVerbResp(page, 'generate.from_prompt', async () => {
      await page.locator('[data-cut-generate-prompt-run]').click()
    })
    if (!preview.ok) throw new Error(`generate.from_prompt preview dispatch failed: ${JSON.stringify(preview.error)}`)
    const previewResult = preview.result
    if (previewResult?.status !== 'completed') throw new Error(`preview status mismatch: ${JSON.stringify(previewResult)}`)
    if (previewResult.plan?.template_id !== 'builtin.lower-third.clean') throw new Error(`unexpected plan: ${JSON.stringify(previewResult.plan)}`)
    if (previewResult.preview?.mime !== 'image/png') throw new Error(`preview missing PNG evidence: ${JSON.stringify(previewResult.preview)}`)
    const img = page.locator('[data-cut-generate-prompt-preview-img]').first()
    await img.waitFor({ state: 'visible', timeout: 8000 })
    const natural = await img.evaluate((el) => ({ w: el.naturalWidth || 0, h: el.naturalHeight || 0 }))
    if (natural.w <= 0 || natural.h <= 0) throw new Error(`prompt preview image did not load: ${JSON.stringify(natural)}`)
    evidence.preview = {
      preview_id: previewResult.preview.preview_id,
      url: previewResult.preview.url,
      mime: previewResult.preview.mime,
      size: `${natural.w}x${natural.h}`,
      plan: previewResult.plan,
    }

    const beforeOps = ((await verb('project.ops')).result?.ops || []).length
    await page.locator('[data-cut-generate-prompt-policy]').selectOption('insert')
    const insert = await captureVerbResp(page, 'generate.from_prompt', async () => {
      await page.locator('[data-cut-generate-prompt-run]').click()
    })
    if (!insert.ok) throw new Error(`generate.from_prompt insert dispatch failed: ${JSON.stringify(insert.error)}`)
    const insertResult = insert.result
    if (insertResult?.status !== 'completed') throw new Error(`insert status mismatch: ${JSON.stringify(insertResult)}`)
    const inserted = insertResult.insert
    const checkpointId = inserted?.checkpoint?.id
    const clipId = inserted?.clips?.[0]
    if (!checkpointId || !clipId) throw new Error(`insert missing checkpoint/clip evidence: ${JSON.stringify(inserted)}`)
    if (inserted.lowering?.verb !== 'title.add') throw new Error(`insert lowered to ${inserted.lowering?.verb}`)
    const afterState = await waitForState((s) => !!findClip(s, clipId))
    const clip = findClip(afterState, clipId)
    const afterOps = ((await verb('project.ops')).result?.ops || []).length
    if (afterOps <= beforeOps) throw new Error(`op count did not increase: before=${beforeOps} after=${afterOps}`)
    evidence.insert = {
      checkpoint: checkpointId,
      op_ids: inserted.op_ids,
      clips: inserted.clips,
      assets: inserted.assets,
      lowering: inserted.lowering,
      materialized_clip: { id: clip.id, title_text: clip.title_text, track: clip._track },
      ops: { before: beforeOps, after: afterOps },
    }

    const reverted = await verb('project.revert', { to: checkpointId, rationale: 'generate prompt verify cleanup' })
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
