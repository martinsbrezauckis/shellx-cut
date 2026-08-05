import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6277'
const temp = mkdtempSync(join(tmpdir(), 'cut-chat-review-server-'))
const projectDir = join(temp, 'chat-review-server.cutproj')
const checks = []

function check(name, pass, detail = '') {
  checks.push({ name, pass, detail })
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}${detail ? `  ${detail}` : ''}`)
}

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:chat-review-server-gate:ui' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(30_000),
  })
  return response.json()
}

try {
  const created = await verb('project.create', { name: 'chat-review-server', dir: projectDir })
  if (!created.ok) throw new Error(created.error?.message || 'project.create failed')
  const seeded = await verb('edit.add_marker', { at_ms: 100, label: 'Baseline marker' })
  if (!seeded.ok) throw new Error(seeded.error?.message || 'baseline marker failed')
  const before = await verb('project.ops')
  const baseline = before.result.ops.at(-1).op_id

  const turn = await verb('agent.chat', {
    message: 'Add the server-gate marker at 900 milliseconds',
    agent: 'claude',
    timeout_ms: 15_000,
  })
  const result = turn.result
  check('agent.chat reports a landed edit', turn.ok && result?.ok && result.actions.length === 1, JSON.stringify(result))
  check('turn returns deterministic plan', result?.plan?.request === 'Add the server-gate marker at 900 milliseconds')
  check('turn baseline is pre-edit history head', result?.review?.baseline === baseline, JSON.stringify(result?.review))
  check('turn diff is computed', !!result?.review?.diff && result.review.tip === result.actions[0].op_id)
  check('turn is safe to group-revert', result?.review?.revert_safe === true && result.review.concurrent_actions.length === 0)

  const after = await verb('project.ops')
  const action = after.result.ops.find((op) => op.op_id === result.actions[0].op_id)
  check('landed op carries unique turn actor', action?.actor?.name === result.review.turn_id, JSON.stringify(action?.actor))
  check('landed op carries Agent Chat surface', action?.actor?.via === 'agent.chat', JSON.stringify(action?.actor))

  const reverted = await verb('project.revert', { to: result.review.baseline, if_tip: result.review.tip, rationale: 'server review gate cleanup' })
  check('project.revert accepts turn baseline', reverted.ok, JSON.stringify(reverted))
  const state = await verb('project.state')
  check('group revert removes only turn result', state.ok && state.result.markers.some((marker) => marker.label === 'Baseline marker') && !state.result.markers.some((marker) => marker.label === 'Agent Chat server gate'))

  const concurrentTurn = verb('agent.chat', {
    message: 'Add another server-gate marker while the user edits',
    agent: 'claude',
    timeout_ms: 15_000,
  })
  await new Promise((resolve) => setTimeout(resolve, 120))
  const humanEdit = await verb('edit.add_marker', { at_ms: 500, label: 'Concurrent human marker' })
  if (!humanEdit.ok) throw new Error(humanEdit.error?.message || 'concurrent marker failed')
  const concurrent = await concurrentTurn
  const concurrentResult = concurrent.result
  check('concurrent human op is not claimed by agent', concurrentResult?.actions.length === 1 && concurrentResult.actions[0].op_id !== humanEdit.op_ids?.[0])
  check('concurrent op is reported with actor', concurrentResult?.review?.concurrent_actions.some((entry) => entry.op_id === humanEdit.op_ids?.[0] && entry.actor.name === 'chat-review-server-gate'))
  check('concurrency disables whole-turn revert', concurrentResult?.review?.revert_safe === false)

  const selective = await verb('edit.restore', {
    op_id: concurrentResult.actions[0].op_id,
    mode: 'tip',
    rationale: 'server review gate selective cleanup',
  })
  check('agent tip can be selectively rejected', selective.ok, JSON.stringify(selective))
  const concurrentState = await verb('project.state')
  check('selective reject preserves concurrent human edit', concurrentState.ok && concurrentState.result.markers.some((marker) => marker.label === 'Concurrent human marker') && !concurrentState.result.markers.some((marker) => marker.label === 'Agent Chat server gate'))
} catch (error) {
  check('gate completed', false, error instanceof Error ? error.stack || error.message : String(error))
} finally {
  rmSync(temp, { recursive: true, force: true })
}

if (checks.some((item) => !item.pass)) process.exitCode = 1
else console.log(`PASS agent-chat-review-server (${checks.length} checks)`)
