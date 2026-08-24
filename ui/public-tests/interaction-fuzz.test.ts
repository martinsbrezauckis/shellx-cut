import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import {
  executeInteractionFuzzSequence,
  generateInteractionFuzzSequence,
  INTERACTION_FUZZ_SCENARIO_ID,
  interactionFuzzClipIntegrity,
  interactionFuzzFailurePath,
  parseInteractionFuzzConfig,
} from './lib/fullCoverageInteractionFuzz.mjs'
import { TIMELINE_SOURCE_AUDIT_SCENARIOS } from './lib/fullCoverageTimelineSourceAuditScenarios.mjs'

const candidate = {
  FCV_INTERACTION_FUZZ_SEED: 'v110-interaction-fuzz-contract',
  FCV_SOURCE_GIT_COMMIT: 'a'.repeat(40),
  FCV_SOURCE_CONTENT_MANIFEST_SHA256: 'b'.repeat(64),
  FCV_RESULT_RECEIPT: join(tmpdir(), 'cut-interaction-fuzz-contract', 'result.json'),
}

const first = generateInteractionFuzzSequence(candidate.FCV_INTERACTION_FUZZ_SEED)
const second = generateInteractionFuzzSequence(candidate.FCV_INTERACTION_FUZZ_SEED)
assert.deepEqual(first, second, 'an identical seed must materialize the identical bounded sequence')
assert.equal(first.length, 12)
for (const required of ['select-primary', 'select-secondary', 'clipboard-roundtrip-primary', 'clipboard-roundtrip-secondary', 'playback-roundtrip']) {
  assert.ok(first.includes(required), `${required} stays represented in every generated sequence`)
}

assert.throws(() => parseInteractionFuzzConfig({ ...candidate, FCV_INTERACTION_FUZZ_SEED: '' }), /SEED is required/,
  'a missing seed must fail before the scenario can pass')
assert.throws(() => parseInteractionFuzzConfig({ ...candidate, FCV_SOURCE_GIT_COMMIT: '' }), /candidate commit/,
  'an unbound candidate must fail before the scenario can pass')
assert.throws(() => parseInteractionFuzzConfig({ ...candidate, FCV_RESULT_RECEIPT: '' }), /candidate-bound interaction fuzz evidence/,
  'a candidate-bound receipt destination is required')

const metadata = TIMELINE_SOURCE_AUDIT_SCENARIOS.find((scenario) => scenario.id === INTERACTION_FUZZ_SCENARIO_ID)
assert.ok(metadata, 'the fuzz scenario has declarative metadata')
assert.equal(metadata.runner, 'timeline-source-audit')
assert.equal(metadata.surface, 'browser-ui')
assert.equal(metadata.receiptSchema, 'shellx-cut/full-coverage-results@1')
assert.equal(metadata.receiptRoot, 'FCV_RESULT_RECEIPT')
assert.ok(metadata.command.some((part) => part.includes(INTERACTION_FUZZ_SCENARIO_ID)), 'metadata keeps the stable focused ID')
assert.ok(metadata.command.some((part) => part.includes('FCV_INTERACTION_FUZZ_SEED')), 'metadata declares the seed contract')
assert.ok(metadata.command.some((part) => part.includes('FCV_SOURCE_CONTENT_MANIFEST_SHA256')), 'metadata declares candidate manifest binding')

const legitimateGap = interactionFuzzClipIntegrity([
  { id: 'c1', asset: 'a1', src_in_ms: 0, src_out_ms: 1_000 },
  { kind: 'gap', duration_ms: 600 },
  { id: 'c2', asset: 'a1', src_in_ms: 1_000, src_out_ms: 2_000 },
])
assert.deepEqual(legitimateGap.mediaClips.map(({ id }) => id), ['c1', 'c2'])
assert.equal(legitimateGap.gaps.length, 1)
assert.equal(legitimateGap.invalidMediaRange, false, 'a gap is not misclassified as a malformed media clip')
assert.equal(legitimateGap.invalidGap, false, 'a positive-duration gap is structurally valid')
assert.equal(interactionFuzzClipIntegrity([{ kind: 'gap', duration_ms: 0 }]).invalidGap, true)
assert.equal(interactionFuzzClipIntegrity([{ id: 'c1', src_in_ms: 10, src_out_ms: 10 }]).invalidMediaRange, true)
const fuzzSource = readFileSync(join(import.meta.dirname, 'lib/fullCoverageInteractionFuzz.mjs'), 'utf8')
assert.match(fuzzSource, /waitForSettledMutation\(before\)/, 'clipboard fuzz captures a settled grouped paste before undo/redo comparison')

const scratch = mkdtempSync(join(tmpdir(), 'cut-interaction-fuzz-negative-'))
try {
  const config = parseInteractionFuzzConfig({ ...candidate, FCV_RESULT_RECEIPT: join(scratch, 'receipt.json') })
  const failurePath = interactionFuzzFailurePath(config.resultReceipt)
  await assert.rejects(
    executeInteractionFuzzSequence({
      config,
      sequence: ['select-primary'],
      executeAction: async () => ({ detail: 'forced action' }),
      assertInvariants: async () => ({ ok: false, detail: 'forced invariant failure' }),
    }),
    /forced invariant failure/,
    'an invariant failure must reject; it cannot be swallowed into a pass',
  )
  const failure = JSON.parse(readFileSync(failurePath, 'utf8'))
  assert.equal(failure.seed, config.seed)
  assert.equal(failure.source.gitCommit, config.source.gitCommit)
  assert.equal(failure.failedStep, 1)
  assert.equal(failure.trace.length, 1)
  assert.equal(failure.trace[0].invariant, 'fail')
} finally {
  rmSync(scratch, { recursive: true, force: true })
}

console.log('PASS deterministic interaction fuzz contract')
