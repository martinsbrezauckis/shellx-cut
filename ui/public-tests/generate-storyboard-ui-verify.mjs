// generate-storyboard-ui-verify.mjs - focused UI gate for Generate > Storyboard.
// It proves visible result evidence for generate.storyboard plan, preview, insert,
// and revert cleanup. This is intentionally separate from the verb-level
// generate-storyboard-verify.mjs so a passing click cannot hide missing evidence.
//
// RUN:
//   CUTD_GENERATE_STORYBOARD_ADAPTER=$PWD/ui/public-tests/fixtures/generate-storyboard-adapter.py \
//     target/debug/cutd serve --addr 127.0.0.1:6178 --headless
//   cd ui && CUTD_DEV_TARGET=http://127.0.0.1:6178 npm run dev -- --host 127.0.0.1 --port 5178
//   cd ui && SWEEP_CUTD=http://127.0.0.1:6178 SWEEP_APP=http://127.0.0.1:5178 \
//     node public-tests/generate-storyboard-ui-verify.mjs

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
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:generate-storyboard-ui-verify' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(VERB_TIMEOUT_MS),
  })
  return r.json()
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

async function waitForUiOpen(panel, timeoutMs = 8000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await verb('ui.open', { panel })
    if (last.ok) return last
    await sleep(250)
  }
  throw new Error(`ui.open ${panel} did not succeed: ${JSON.stringify(last)}`)
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

async function waitForStoryboardPreviewImageLoaded(page, timeoutMs = 8000) {
  const selector = '[data-cut-generate-storyboard-preview-img]'
  await page.waitForFunction((sel) => {
    const img = document.querySelector(sel)
    return !!img && img.complete && img.naturalWidth > 0 && img.naturalHeight > 0
  }, selector, { timeout: timeoutMs })
  return page.locator(selector).first().evaluate((el) => ({ w: el.naturalWidth || 0, h: el.naturalHeight || 0 }))
}

function findClip(project, clipId) {
  return (project?.tracks || [])
    .flatMap((track) => (track.clips || []).map((clip) => ({ ...clip, _track: track.id })))
    .find((clip) => clip.id === clipId)
}

async function main() {
  const suffix = Math.random().toString(36).slice(2, 8)
  const name = `genstoryui_${suffix}`
  const dir = join(mkdtempSync(join(tmpdir(), 'shellx-cut-genstoryui-')), `${name}.cutproj`)
  const created = await verb('project.create', {
    name,
    dir,
    settings: { width: 640, height: 360, fps: 24 },
  })
  if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created.error)}`)

  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 920 } })
  const evidence = { project: dir, prompt: 'Plan a clean 12 second launch video with title, lower third, and CTA.' }
  const consoleErrors = []
  page.on('console', (msg) => {
    if (msg.type() === 'error') consoleErrors.push(msg.text())
  })

  try {
    await page.goto(APP, { waitUntil: 'networkidle' })

    evidence.debug_open = await waitForUiOpen('generate-storyboard')
    await page.waitForSelector('[data-cut-panel="generate-templates"]', { timeout: 8000 })
    await page.waitForSelector('[data-cut-generate-storyboard]', { timeout: 5000 })
    const uiState = await verb('ui.state')
    if (!uiState.ok || !uiState.result?.panels?.includes('generate:storyboard')) {
      throw new Error(`ui.state did not report Generate Storyboard: ${JSON.stringify(uiState)}`)
    }
    evidence.debug_state = {
      panels: uiState.result.panels.filter((panel) => String(panel).includes('generate')),
    }

    await page.locator('[data-cut-generate-storyboard-input]').fill(evidence.prompt)
    await page.locator('[data-cut-generate-storyboard-mode]').selectOption('quick_prompt')
    await page.locator('[data-cut-generate-storyboard-agent]').selectOption('auto')
    await page.waitForFunction(() => {
      const status = document.querySelector('[data-cut-generate-template-status]')
      const plan = document.querySelector('[data-cut-generate-storyboard-plan]')
      return status?.textContent?.includes('Project ready') && plan && !plan.hasAttribute('disabled')
    }, null, { timeout: 8000 })

    const plan = await captureVerbResp(page, 'generate.storyboard', async () => {
      await page.locator('[data-cut-generate-storyboard-plan]').click()
    })
    if (!plan.ok) throw new Error(`generate.storyboard plan dispatch failed: ${JSON.stringify(plan.error)}`)
    const planResult = plan.result
    if (planResult?.status !== 'completed') throw new Error(`plan status mismatch: ${JSON.stringify(planResult)}`)
    if (planResult.evidence?.policy !== 'plan' || planResult.evidence?.mutated !== false) {
      throw new Error(`plan evidence mismatch: ${JSON.stringify(planResult.evidence)}`)
    }
    const sceneRows = page.locator('[data-cut-generate-storyboard-scene]')
    await sceneRows.first().waitFor({ state: 'visible', timeout: 5000 })
    const sceneCount = await sceneRows.count()
    const sceneText = await page.locator('[data-cut-generate-storyboard-scenes]').textContent()
    if (sceneCount < 3) throw new Error(`expected at least 3 storyboard scene rows, got ${sceneCount}`)
    if (!sceneText?.includes('builtin.title-card.episode')) throw new Error(`template evidence not visible: ${sceneText}`)
    evidence.plan = {
      status: planResult.status,
      scene_count: planResult.evidence.scene_count,
      duration_ms: planResult.evidence.duration_ms,
      template_ids: planResult.evidence.template_ids,
      visible_scene_rows: sceneCount,
    }

    const previewOpsBefore = ((await verb('project.ops')).result?.ops || []).length
    const preview = await captureVerbResp(page, 'generate.storyboard', async () => {
      await page.locator('[data-cut-generate-storyboard-preview]').click()
    })
    if (!preview.ok) throw new Error(`generate.storyboard preview dispatch failed: ${JSON.stringify(preview.error)}`)
    const previewResult = preview.result
    if (previewResult?.evidence?.policy !== 'preview' || previewResult.evidence.mutated !== false) {
      throw new Error(`preview evidence mismatch: ${JSON.stringify(previewResult?.evidence)}`)
    }
    const imgs = page.locator('[data-cut-generate-storyboard-preview-img]')
    await imgs.first().waitFor({ state: 'visible', timeout: 8000 })
    const natural = await waitForStoryboardPreviewImageLoaded(page, 8000)
    if (natural.w <= 0 || natural.h <= 0) throw new Error(`storyboard preview image did not load: ${JSON.stringify(natural)}`)
    const previewOpsAfter = ((await verb('project.ops')).result?.ops || []).length
    if (previewOpsAfter !== previewOpsBefore) throw new Error(`preview mutated ops: ${previewOpsBefore}->${previewOpsAfter}`)
    evidence.preview = {
      scenes: (previewResult.preview?.scenes || []).map((scene) => ({
        scene_id: scene.scene_id,
        template_id: scene.template_id,
        preview_id: scene.preview_id,
        url: scene.url,
        mime: scene.mime,
      })),
      first_image_size: `${natural.w}x${natural.h}`,
      ops: { before: previewOpsBefore, after: previewOpsAfter },
    }
    if (evidence.preview.scenes.length < 3) throw new Error(`preview scene evidence missing: ${JSON.stringify(previewResult.preview)}`)

    const insertOpsBefore = ((await verb('project.ops')).result?.ops || []).length
    const insert = await captureVerbResp(page, 'generate.storyboard', async () => {
      await page.locator('[data-cut-generate-storyboard-insert]').click()
    })
    if (!insert.ok) throw new Error(`generate.storyboard insert dispatch failed: ${JSON.stringify(insert.error)}`)
    const insertResult = insert.result
    const inserted = insertResult?.insert
    if (insertResult?.evidence?.policy !== 'insert' || insertResult.evidence.mutated !== true) {
      throw new Error(`insert evidence mismatch: ${JSON.stringify(insertResult?.evidence)}`)
    }
    const checkpointId = inserted?.checkpoints?.[0]
    if (!checkpointId || !Array.isArray(inserted.clips) || inserted.clips.length < 3) {
      throw new Error(`insert missing checkpoint/clip evidence: ${JSON.stringify(inserted)}`)
    }
    await page.locator('[data-cut-generate-storyboard-insert-result]').waitFor({ state: 'visible', timeout: 5000 })
    const insertText = await page.locator('[data-cut-generate-storyboard-insert-result]').textContent()
    if (!insertText?.includes(checkpointId) || !inserted.clips.every((clipId) => insertText.includes(clipId))) {
      throw new Error(`insert evidence not visible: ${insertText}`)
    }
    const afterInsertState = await waitForState((s) => inserted.clips.every((clipId) => findClip(s, clipId)))
    const insertOpsAfter = ((await verb('project.ops')).result?.ops || []).length
    if (insertOpsAfter <= insertOpsBefore) throw new Error(`insert did not append ops: ${insertOpsBefore}->${insertOpsAfter}`)
    evidence.insert = {
      checkpoint: checkpointId,
      op_ids: inserted.op_ids,
      clips: inserted.clips,
      assets: inserted.assets,
      scene_count: inserted.scenes.length,
      materialized_clips: inserted.clips.map((clipId) => {
        const clip = findClip(afterInsertState, clipId)
        return { id: clip.id, track: clip._track, asset: clip.asset, title_text: clip.title_text }
      }),
      ops: { before: insertOpsBefore, after: insertOpsAfter },
    }

    const reverted = await verb('project.revert', { to: checkpointId, rationale: 'generate storyboard UI verify cleanup' })
    if (!reverted.ok) throw new Error(`project.revert failed: ${JSON.stringify(reverted.error)}`)
    await waitForState((s) => inserted.clips.every((clipId) => !findClip(s, clipId)))
    evidence.revert = { reverted_to: reverted.result?.reverted_to, removed_clips: inserted.clips }

    if (consoleErrors.length > 0) throw new Error(`browser console errors: ${consoleErrors.join('\n')}`)
    console.log(JSON.stringify({ ok: true, evidence }, null, 2))
  } finally {
    await browser.close()
  }
}

main().catch((err) => {
  console.error(err?.stack || err)
  process.exit(1)
})
