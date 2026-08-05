// generate-storyboard-verify.mjs - focused API gate for the native Generate
// Storyboard contract. It proves result evidence for plan, preview, insert,
// revert, and the one-question director path.
//
// RUN:
//   CUTD_GENERATE_STORYBOARD_ADAPTER=$PWD/ui/public-tests/fixtures/generate-storyboard-adapter.py \
//     app/target/debug/cutd serve --addr 127.0.0.1:6179 --headless
//   cd ui && SWEEP_CUTD=http://127.0.0.1:6179 node public-tests/generate-storyboard-verify.mjs

import { mkdtempSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6179'
const VERB_TIMEOUT_MS = Number(process.env.VERB_TIMEOUT_MS || 60000)

async function verb(name, args = {}) {
  const r = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:generate-storyboard-verify' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(VERB_TIMEOUT_MS),
  })
  return r.json()
}

async function opsLen() {
  const r = await verb('project.ops')
  if (!r.ok) throw new Error(`project.ops failed: ${JSON.stringify(r.error)}`)
  return (r.result?.ops || []).length
}

function assertStoryboardEvidence(result) {
  if (result?.status !== 'completed') throw new Error(`storyboard status mismatch: ${JSON.stringify(result)}`)
  if (result.storyboard?.schema !== 'shellx-cut/generate-storyboard/1') throw new Error(`missing storyboard schema: ${JSON.stringify(result.storyboard)}`)
  if (!Array.isArray(result.storyboard?.scenes) || result.storyboard.scenes.length < 3) throw new Error(`storyboard scenes missing: ${JSON.stringify(result.storyboard?.scenes)}`)
  if (result.validation?.ok !== true) throw new Error(`validation did not pass: ${JSON.stringify(result.validation)}`)
  if (!result.evidence?.template_ids?.includes('builtin.title-card.episode')) throw new Error(`template evidence missing title card: ${JSON.stringify(result.evidence)}`)
}

function assertPng(path, label) {
  const bytes = readFileSync(path)
  const png = bytes.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))
  if (!png || bytes.length <= 1024) throw new Error(`${label} is not a real PNG: ${path} len=${bytes.length}`)
}

async function projectState() {
  const r = await verb('project.state')
  if (!r.ok) throw new Error(`project.state failed: ${JSON.stringify(r.error)}`)
  return r.result
}

function projectHasClip(state, clipId) {
  return (state?.tracks || []).some((track) => (track.clips || []).some((clip) => clip.id === clipId))
}

async function main() {
  const suffix = Math.random().toString(36).slice(2, 8)
  const name = `genstory_${suffix}`
  const dir = join(mkdtempSync(join(tmpdir(), 'shellx-cut-genstory-')), `${name}.cutproj`)
  const created = await verb('project.create', {
    name,
    dir,
    settings: { width: 640, height: 360, fps: 24 },
  })
  if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created.error)}`)

  const before = await opsLen()
  const storyboard = await verb('generate.storyboard', {
    input: 'Plan a clean 12 second launch video with title, lower third, and CTA.',
    mode: 'quick_prompt',
    policy: 'plan',
    agent: 'auto',
    rationale: 'generate storyboard verify: completed evidence',
  })
  if (!storyboard.ok) throw new Error(`generate.storyboard failed: ${JSON.stringify(storyboard.error)}`)
  assertStoryboardEvidence(storyboard.result)
  if (storyboard.result.evidence?.policy !== 'plan') throw new Error(`plan policy evidence mismatch: ${JSON.stringify(storyboard.result.evidence)}`)
  if (storyboard.result.evidence?.mutated !== false) throw new Error(`plan mutated flag wrong: ${JSON.stringify(storyboard.result.evidence)}`)
  const after = await opsLen()
  if (after !== before) throw new Error(`storyboard plan mutated ops: ${before}->${after}`)

  const director = await verb('generate.storyboard', {
    input: 'Help me craft a launch video brief.',
    mode: 'director_brief',
    policy: 'plan',
    answers: {},
    agent: 'auto',
    rationale: 'generate storyboard verify: one question',
  })
  if (!director.ok) throw new Error(`director storyboard failed: ${JSON.stringify(director.error)}`)
  const directorResult = director.result
  if (directorResult?.status !== 'needs_input') throw new Error(`director status mismatch: ${JSON.stringify(directorResult)}`)
  if (!Array.isArray(directorResult.questions) || directorResult.questions.length !== 1) throw new Error(`director did not return exactly one question: ${JSON.stringify(directorResult.questions)}`)
  if (directorResult.evidence?.mutated !== false) throw new Error(`director mutated flag wrong: ${JSON.stringify(directorResult.evidence)}`)
  const afterDirector = await opsLen()
  if (afterDirector !== before) throw new Error(`director plan mutated ops: ${before}->${afterDirector}`)

  const previewBefore = await opsLen()
  const preview = await verb('generate.storyboard', {
    input: 'Preview a clean 12 second launch video with title, lower third, and CTA.',
    mode: 'quick_prompt',
    policy: 'preview',
    agent: 'auto',
    rationale: 'generate storyboard verify: preview evidence',
  })
  if (!preview.ok) throw new Error(`generate.storyboard preview failed: ${JSON.stringify(preview.error)}`)
  const previewResult = preview.result
  assertStoryboardEvidence(previewResult)
  if (previewResult.evidence?.policy !== 'preview' || previewResult.evidence?.mutated !== false) throw new Error(`preview evidence wrong: ${JSON.stringify(previewResult.evidence)}`)
  const previewScenes = previewResult.preview?.scenes || []
  if (previewScenes.length !== 3) throw new Error(`preview scene evidence missing: ${JSON.stringify(previewResult.preview)}`)
  for (const scene of previewScenes) {
    if (scene.status !== 'previewed') throw new Error(`preview scene status wrong: ${JSON.stringify(scene)}`)
    if (scene.mime !== 'image/png' || !scene.path || !scene.preview_id) throw new Error(`preview scene PNG evidence missing: ${JSON.stringify(scene)}`)
    if (scene.width !== 640 || scene.height !== 360) throw new Error(`preview scene geometry wrong: ${JSON.stringify(scene)}`)
    assertPng(scene.path, `preview ${scene.scene_id}`)
  }
  const previewAfter = await opsLen()
  if (previewAfter !== previewBefore) throw new Error(`storyboard preview mutated ops: ${previewBefore}->${previewAfter}`)

  const insertBefore = await opsLen()
  const insert = await verb('generate.storyboard', {
    input: 'Insert a clean 12 second launch video with title, lower third, and CTA.',
    mode: 'quick_prompt',
    policy: 'insert',
    agent: 'auto',
    rationale: 'generate storyboard verify: insert evidence',
  })
  if (!insert.ok) throw new Error(`generate.storyboard insert failed: ${JSON.stringify(insert.error)}`)
  const insertResult = insert.result
  assertStoryboardEvidence(insertResult)
  if (insertResult.evidence?.policy !== 'insert' || insertResult.evidence?.mutated !== true) throw new Error(`insert evidence wrong: ${JSON.stringify(insertResult.evidence)}`)
  const inserted = insertResult.insert
  if (!inserted?.checkpoints?.[0]) throw new Error(`insert checkpoint evidence missing: ${JSON.stringify(inserted)}`)
  if (!Array.isArray(inserted.clips) || inserted.clips.length !== 3) throw new Error(`insert clip evidence missing: ${JSON.stringify(inserted)}`)
  if (!Array.isArray(inserted.scenes) || inserted.scenes.length !== 3) throw new Error(`insert scene evidence missing: ${JSON.stringify(inserted)}`)
  for (const scene of inserted.scenes) {
    if (scene.status !== 'inserted') throw new Error(`insert scene status wrong: ${JSON.stringify(scene)}`)
    if (!scene.checkpoint?.id || !scene.clips?.length || !scene.assets?.length || !scene.op_ids?.length) throw new Error(`insert scene evidence incomplete: ${JSON.stringify(scene)}`)
  }
  const stateAfterInsert = await projectState()
  for (const clipId of inserted.clips) {
    if (!projectHasClip(stateAfterInsert, clipId)) throw new Error(`inserted clip not present in project state: ${clipId}`)
  }
  const insertAfter = await opsLen()
  if (insertAfter <= insertBefore) throw new Error(`storyboard insert did not append ops: ${insertBefore}->${insertAfter}`)

  const reverted = await verb('project.revert', {
    to: inserted.checkpoints[0],
    rationale: 'generate storyboard verify cleanup',
  })
  if (!reverted.ok) throw new Error(`project.revert after storyboard insert failed: ${JSON.stringify(reverted.error)}`)
  const stateAfterRevert = await projectState()
  for (const clipId of inserted.clips) {
    if (projectHasClip(stateAfterRevert, clipId)) throw new Error(`reverted storyboard clip still present: ${clipId}`)
  }
  const afterRevert = await opsLen()

  console.log(JSON.stringify({
    ok: true,
    evidence: {
      project: dir,
      completed: {
        status: storyboard.result.status,
        scene_count: storyboard.result.evidence.scene_count,
        duration_ms: storyboard.result.evidence.duration_ms,
        template_ids: storyboard.result.evidence.template_ids,
        skill_path: storyboard.result.evidence.skill_path,
      },
      director: {
        status: directorResult.status,
        questions: directorResult.questions.map((q) => ({ id: q.id, field: q.field })),
        missing: directorResult.evidence.brief_fields.missing,
      },
      preview: {
        status: previewResult.status,
        scenes: previewScenes.map((scene) => ({
          scene_id: scene.scene_id,
          template_id: scene.template_id,
          preview_id: scene.preview_id,
          path: scene.path,
          mime: scene.mime,
          width: scene.width,
          height: scene.height,
        })),
      },
      insert: {
        status: insertResult.status,
        checkpoints: inserted.checkpoints,
        op_ids: inserted.op_ids,
        clips: inserted.clips,
        assets: inserted.assets,
        scene_count: inserted.scenes.length,
        restore_hint: inserted.restore_hint,
      },
      revert: {
        to: inserted.checkpoints[0],
        removed_clips: inserted.clips,
      },
      ops: { before, after, afterDirector, previewBefore, previewAfter, insertBefore, insertAfter, afterRevert },
    },
  }, null, 2))
}

main().catch((err) => {
  console.error(err?.stack || err)
  process.exit(1)
})
