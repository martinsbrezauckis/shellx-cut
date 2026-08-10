// Review receipt-rerun source contract (run: npx tsx public-tests/receipt-rerun-control.test.ts).

import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { isRerunResult } from '../src/panels/Review/receiptRerunModel'

const root = resolve(import.meta.dirname, '..')
const control = readFileSync(resolve(root, 'src/panels/Review/ReceiptRerunControl.tsx'), 'utf8')
const model = readFileSync(resolve(root, 'src/panels/Review/receiptRerunModel.ts'), 'utf8')
const receipts = readFileSync(resolve(root, 'src/panels/Review/Receipts.tsx'), 'utf8')
const server = readFileSync(resolve(root, '../app/server/src/dispatch/verify_handlers/rerun.rs'), 'utf8')
const verbs = JSON.parse(readFileSync(resolve(root, '../schema/verbs.json'), 'utf8'))

for (const selector of [
  'data-cut-action="receipt-rerun"',
  'data-cut-action="receipt-rerun-cancel"',
  'data-cut-receipt-rerun-state=',
  'data-cut-receipt-rerun-scope=',
  'data-cut-receipt-rerun-cancelled=',
]) assert.match(control, new RegExp(selector), `Review rerun control exposes ${selector}`)

assert.match(model, /source_receipt_id !== identity\.renderId/, 'result source id must match the selected receipt')
assert.match(model, /output_hash !== identity\.outputHash/, 'result hash must match the selected receipt')
assert.match(model, /profile !== identity\.profile/, 'result profile must match the selected receipt')
assert.match(model, /verification_receipt !== `receipts\/verify_rerun_\$\{jobId\}\.json`/, 'result receipt must name the exact job')
assert.match(model, /result\.pass === result\.checks\.every\(\(check\) => check\.pass\)/, 'result pass must agree with every check')
assert.match(model, /isRerunHandle/, 'immediate job handle is receipt/hash correlated')
assert.match(model, /isRerunResult/, 'jobs.status result is receipt/hash/profile correlated')
assert.match(control, /jobs\.cancel/, 'running output checks have a cancellation route')
assert.match(control, /localStorage/, 'a remount resumes only a correlated durable job id')
assert.match(control, /stored\.output_path === identity\.outputPath/, 'saved jobs are isolated by receipt output path')
assert.match(control, /stored\.at_op === identity\.atOp/, 'saved jobs are isolated by receipt op pointer')
assert.doesNotMatch(control, /callVerb\('render\.final'/, 'the control cannot trigger a re-render')
assert.match(receipts, /<ReceiptRerunControl receipt=\{receipt\}/, 'every rendered receipt exposes the recheck control')

for (const required of [
  'selected_render_receipt',
  'is_plain_regular_file',
  'receipt.render_id != requested_id',
  'run_blocking_cancellable',
  'owned_job_process_control',
  'with_render_process_control',
  'run_instruments_owned_ephemeral',
  'cut_media::probe',
  'write_output_atomic',
  'duration_matches_receipt',
]) assert.match(server, new RegExp(required.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')), `server keeps ${required}`)
assert.ok((server.match(/fenced_output_for_receipt/g) ?? []).length >= 4, 'output is re-fenced before sidecar, probe, and terminal receipt')
assert.ok((server.match(/assert_receipt_hash/g) ?? []).length >= 4, 'output hash is checked before job, sidecar, probe, and terminal receipt')
assert.doesNotMatch(server, /remove_file/, 'rerun must not create and delete a predictable perception cache leaf')

const rerun = verbs.verbs.find((verb: { name: string }) => verb.name === 'verify.rerun')
assert.ok(rerun, 'verify.rerun is public schema')
assert.equal(rerun.behavior.dispatch, 'verify_rerun')
assert.equal(rerun.behavior.async_job, 'media')
assert.equal(rerun.behavior.agent_chat, 'deny')
assert.equal(rerun.behavior.side_effects.process, true)
assert.equal(rerun.args.properties.render_id.pattern, '^[A-Za-z0-9][A-Za-z0-9_.-]*$')

const identity = {
  renderId: 'render_001',
  outputHash: 'sha256:abc',
  outputPath: '/project-a/exports/render_001.mp4',
  atOp: 'op_000123',
  profile: 'talking_head' as const,
}
const jobId = 'job_001'
const expectedChecks = [
  'lufs',
  'black_or_frozen_frames',
  'uniform_border',
  'silence_at_edges',
  'duration_matches_receipt',
]
function resultFor(checks = expectedChecks.map((name) => ({
  name, pass: true, details: {}, evidence: {},
})), pass = checks.every((check) => check.pass)) {
  return {
    render_id: identity.renderId,
    source_receipt_id: identity.renderId,
    verification_receipt: `receipts/verify_rerun_${jobId}.json`,
    output_hash: identity.outputHash,
    checked_at: '2026-08-08T12:00:00Z',
    scope: 'rendered_output',
    profile: identity.profile,
    checks,
    pass,
  }
}

assert.equal(isRerunResult(resultFor(), identity, jobId), true, 'complete correlated result is accepted')
assert.equal(
  isRerunResult({ ...resultFor(), verification_receipt: 'receipts/verify_rerun_job_other.json' }, identity, jobId),
  false,
  'a receipt from another job is stale evidence',
)
assert.equal(
  isRerunResult(resultFor(expectedChecks.slice(0, 4).map((name) => ({ name, pass: true, details: {}, evidence: {} }))), identity, jobId),
  false,
  'a missing output check is partial evidence',
)
assert.equal(
  isRerunResult(resultFor([
    ...expectedChecks.slice(0, 4).map((name) => ({ name, pass: true, details: {}, evidence: {} })),
    { name: 'lufs', pass: true, details: {}, evidence: {} },
  ]), identity, jobId),
  false,
  'a duplicate output check is stale or partial evidence',
)
assert.equal(
  isRerunResult(resultFor(expectedChecks.map((name, index) => ({
    name: index === 4 ? 'unexpected_check' : name, pass: true, details: {}, evidence: {},
  })), true), identity, jobId),
  false,
  'an unexpected output check cannot substitute for the exact set',
)
assert.equal(isRerunResult(resultFor(undefined, false), identity, jobId), false, 'inconsistent result pass is rejected')

console.log('PASS receipt-bound rerun UI and process/receipt contract')
