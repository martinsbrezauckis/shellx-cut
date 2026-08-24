import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import {
  assessFocusedScenarioActionRows,
  assessFocusedScenarioReachability,
  CONTROLLED_FOCUSED_CANDIDATE_ENV,
  FULL_COVERAGE_FOCUSED_SCENARIO_SCHEMA,
  FULL_COVERAGE_FOCUSED_SCENARIOS,
  validateFocusedScenarioRequest,
} from './lib/fullCoverageFocusedScenarioContract.mjs'
import { EXPORT_LIFECYCLE_ACTION_NAMES } from './lib/fullCoverageLifecycleExportActions.mjs'
import { RELINK_DERIVATION_ACTION_NAMES } from './lib/fullCoverageLifecycleRelinkActions.mjs'
import { INTERACTION_FUZZ_SCENARIO_ID } from './lib/fullCoverageInteractionFuzz.mjs'
import { TIMELINE_CONTEXT_AUDIT_SCENARIOS } from './lib/fullCoverageTimelineContextAuditScenarios.mjs'
import { TIMELINE_SOURCE_AUDIT_SCENARIOS } from './lib/fullCoverageTimelineSourceAuditScenarios.mjs'

const ids = FULL_COVERAGE_FOCUSED_SCENARIOS.map((scenario) => scenario.id)
assert.equal(FULL_COVERAGE_FOCUSED_SCENARIO_SCHEMA, 'shellx-cut/full-coverage-focused-scenarios@2')
assert.deepEqual(ids, [
  'e2e-context-ownership-01',
  'e2e-empty-paste-01',
  'e2e-gap-boundary-01',
  'e2e-trim-tools-01',
  'e2e-source-insert-01',
  'e2e-selection-sync-01',
  'e2e-keyboard-focus-01',
  'e2e-export-lifecycle-01',
  'e2e-relink-derivation-01',
  'e2e-interaction-fuzz-01',
])
assert.equal(new Set(ids).size, ids.length)

const byId = (id: string) => FULL_COVERAGE_FOCUSED_SCENARIOS.find((scenario) => scenario.id === id)!
for (const scenario of TIMELINE_CONTEXT_AUDIT_SCENARIOS) {
  const contract = byId(scenario.id)
  assert.equal(contract.selector.section, 'ctxmenu')
  assert.deepEqual(contract.execution.emittedActionIds, [scenario.id])
  assert.ok(scenario.command.includes(`FCV_SECTION=${contract.selector.section}`))
  assert.ok(scenario.command.includes(`FCV_ONLY=${contract.selector.only}`))
}
for (const scenario of TIMELINE_SOURCE_AUDIT_SCENARIOS) {
  const contract = byId(scenario.id)
  assert.equal(contract.selector.section, 'timeline-source-audit')
  assert.ok(scenario.command.includes(`FCV_SECTION=${contract.selector.section}`))
  assert.ok(scenario.command.includes(`FCV_ONLY=${contract.selector.only}`))
}
assert.deepEqual(byId('e2e-export-lifecycle-01').execution.emittedActionIds, EXPORT_LIFECYCLE_ACTION_NAMES)
assert.deepEqual(byId('e2e-relink-derivation-01').execution.emittedActionIds, RELINK_DERIVATION_ACTION_NAMES)
assert.deepEqual(byId(INTERACTION_FUZZ_SCENARIO_ID).execution.emittedActionIds, [INTERACTION_FUZZ_SCENARIO_ID])

const candidateEnvironment = Object.fromEntries(CONTROLLED_FOCUSED_CANDIDATE_ENV.map((key) => [key, `test-${key}`]))
for (const scenario of FULL_COVERAGE_FOCUSED_SCENARIOS) {
  const environment: Record<string, string> = {
    FCV_SECTION: scenario.selector.section,
    FCV_ONLY: scenario.id,
  }
  if (scenario.receipt.candidateBinding.required) Object.assign(environment, candidateEnvironment)
  if (scenario.id === 'e2e-interaction-fuzz-01') environment.FCV_INTERACTION_FUZZ_SEED = 'focused-contract-seed'

  assert.equal(
    validateFocusedScenarioRequest({
      only: scenario.id,
      sections: [scenario.selector.section],
      environment,
    })?.id,
    scenario.id,
    `${scenario.id} has an exact runnable focused route`,
  )
  assert.throws(
    () => validateFocusedScenarioRequest({ only: scenario.id, sections: ['wrong-section'], environment }),
    /requires exactly FCV_SECTION=/,
    `${scenario.id} cannot silently run through a different section`,
  )
  const emitted = scenario.execution.emittedActionIds.map((actionId) => ({ actionId }))
  assert.deepEqual(
    assessFocusedScenarioReachability({ scenario, completedSections: [scenario.selector.section], rows: emitted }),
    { ok: true, missingSections: [], missingActionIds: [] },
    `${scenario.id} receipt rows satisfy the declared contract`,
  )
  const unreachable = assessFocusedScenarioReachability({ scenario, completedSections: [], rows: [] })
  assert.equal(unreachable.ok, false, `${scenario.id} cannot pass with an uninvoked factory`)
  assert.deepEqual(unreachable.missingActionIds, scenario.execution.emittedActionIds)

  const completeRows = scenario.execution.actionRequirements.map(({ actionId, dimensions }) => ({ actionId, ...dimensions }))
  assert.deepEqual(
    assessFocusedScenarioActionRows({ scenario, rows: completeRows }),
    { ok: true, incompleteActionIds: [] },
    `${scenario.id} requires its exact declared evidence dimensions for every action`,
  )
  assert.equal(
    assessFocusedScenarioActionRows({ scenario, rows: completeRows.map(({ actionId }) => ({ actionId })) }).ok,
    false,
    `${scenario.id} rejects an action row whose evidence dimensions are incomplete`,
  )
  const browser = scenario.execution.actionRequirements.find(({ kind }) => kind === 'browser')
  if (browser) assert.equal(
    assessFocusedScenarioActionRows({ scenario, rows: completeRows.map((row) => row.actionId === browser.actionId ? { ...row, click: 'na' } : row) }).ok,
    false,
    `${scenario.id} rejects an N/A for a browser action`,
  )
  const support = scenario.execution.actionRequirements.find(({ kind }) => kind === 'support')
  if (support) assert.equal(
    assessFocusedScenarioActionRows({ scenario, rows: completeRows.map((row) => row.actionId === support.actionId ? { ...row, result: 'na' } : row) }).ok,
    false,
    `${scenario.id} rejects a semantic support-row result N/A`,
  )

  if (scenario.receipt.candidateBinding.required) {
    const missingBinding = { ...environment }
    delete missingBinding.FCV_RESULT_RECEIPT
    assert.throws(
      () => validateFocusedScenarioRequest({ only: scenario.id, sections: [scenario.selector.section], environment: missingBinding }),
      /candidate-bound focused evidence/,
      `${scenario.id} cannot emit an unbound focused receipt`,
    )
  }
}

assert.throws(
  () => validateFocusedScenarioRequest({ only: 'e2e-undisclosed-01', sections: ['ctxmenu'] }),
  /not a declared focused full-coverage scenario/,
  'unknown e2e selectors cannot silently produce an empty green receipt',
)

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const read = (relative: string) => readFileSync(resolve(root, relative), 'utf8')
const runner = read('public-tests/full-coverage-verify.mjs')
const timelineSourceRunner = read('public-tests/lib/fullCoverageTimelineSourceAuditActions.mjs')
for (const scenario of FULL_COVERAGE_FOCUSED_SCENARIOS) {
  const factorySource = read(scenario.execution.factory.module.replace(/^ui\//, ''))
  assert.match(factorySource, new RegExp(`export (?:async )?function ${scenario.execution.factory.export}\\b`), `${scenario.id} factory remains exported`)
  assert.match(factorySource, new RegExp(scenario.execution.factory.invocation.replace('.', '\\.') + '\\('), `${scenario.id} factory invokes its focused behavior`)
  assert.match(runner, new RegExp(`${scenario.execution.factory.runnerInvocation}\\(page`), `${scenario.id} factory route is invoked from the canonical runner`)
}
assert.match(runner, /validateFocusedScenarioRequest\(/, 'runner validates focused routing before execution')
assert.match(runner, /assessFocusedScenarioReachability\(/, 'runner records a reachability failure when declared rows are absent')
assert.match(runner, /assessFocusedScenarioActionRows\(/, 'runner rejects incomplete declared action rows')
assert.match(runner, /focusedScenario: FOCUSED_SCENARIO/, 'runner writes the selected immutable focused scenario into the result receipt')
assert.match(
  timelineSourceRunner,
  /await seekSource\(2,[^\n]*0:02[.]000[\s\S]*source-mark-in[\s\S]*source-in[^\n]*0:02[.]000[\s\S]*await seekSource\(5[.]5,[^\n]*0:05[.]500[\s\S]*source-mark-out[\s\S]*source-out[^\n]*0:05[.]500/,
  'source Insert waits for each rendered mark before advancing or inserting',
)

console.log('PASS focused scenario contracts, candidate binding, and canonical runner reachability')
