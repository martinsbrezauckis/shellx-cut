// Exact focused-browser scenarios owned by the canonical full-coverage runner.
// These records are source contracts for Release Studio, not standalone commands:
// a controller must provide the stated candidate binding before a focused receipt
// can be accepted as evidence.

import { EXPORT_LIFECYCLE_ACTION_NAMES } from './fullCoverageLifecycleExportActions.mjs'
import { RELINK_DERIVATION_ACTION_NAMES } from './fullCoverageLifecycleRelinkActions.mjs'
import { INTERACTION_FUZZ_SCENARIO_ID } from './fullCoverageInteractionFuzz.mjs'

export const FULL_COVERAGE_FOCUSED_SCENARIO_SCHEMA = 'shellx-cut/full-coverage-focused-scenarios@2'
export const FULL_COVERAGE_RESULT_SCHEMA = 'shellx-cut/full-coverage-results@1'

export const CONTROLLED_FOCUSED_CANDIDATE_ENV = Object.freeze([
  'FCV_CANDIDATE_ID',
  'FCV_SOURCE_GIT_COMMIT',
  'FCV_SOURCE_GIT_TREE',
  'FCV_SOURCE_WORKTREE',
  'FCV_SOURCE_CONTENT_MANIFEST_SHA256',
  'FCV_TEST_CONTROL_MANIFEST_SHA256',
  'FCV_FIXTURE_ID',
  'FCV_FIXTURE_SEED_SHA256',
  'FCV_RUNNER_ID',
  'FCV_RESULT_RECEIPT',
])

const RUNNER = Object.freeze({
  id: 'full-coverage-verify',
  entrypoint: 'ui/public-tests/full-coverage-verify.mjs',
})

function browserAction(actionId) {
  return Object.freeze({ actionId, kind: 'browser', dimensions: Object.freeze({ present: 'pass', render: 'pass', click: 'pass', result: 'pass' }) })
}

function supportAction(actionId, dimensions) {
  return Object.freeze({ actionId, kind: 'support', dimensions: Object.freeze(dimensions) })
}

function frozenScenario({ id, section, factory, actionIds, actionRequirements = actionIds.map(browserAction), candidateBound = true, requiredEnvironment = [] }) {
  if (actionRequirements.length !== actionIds.length || actionRequirements.some((entry, index) => entry.actionId !== actionIds[index])) {
    throw new Error(`focused scenario ${id} must declare one exact evidence contract for every action`)
  }
  return Object.freeze({
    id,
    selector: Object.freeze({ section, only: id }),
    runner: RUNNER,
    execution: Object.freeze({
      factory: Object.freeze(factory),
      emittedActionIds: Object.freeze([...actionIds]),
      actionRequirements: Object.freeze([...actionRequirements]),
    }),
    receipt: Object.freeze({
      schema: FULL_COVERAGE_RESULT_SCHEMA,
      root: 'FCV_RESULT_RECEIPT',
      candidateBinding: Object.freeze({
        required: candidateBound,
        environment: candidateBound ? CONTROLLED_FOCUSED_CANDIDATE_ENV : Object.freeze([]),
      }),
    }),
    requiredEnvironment: Object.freeze(['FCV_SECTION', 'FCV_ONLY', ...requiredEnvironment]),
  })
}

const CONTEXT_FACTORY = Object.freeze({
      module: 'ui/public-tests/lib/fullCoverageTimelineContextActions.mjs',
      export: 'createTimelineContextActionCoverage',
      sectionFunction: 'secContextMenu',
      invocation: 'createTimelineContextAuditCoverage',
  runnerInvocation: 'runTimelineContextActionCoverage',
})

function timelineSourceFactory(invocation) {
  return Object.freeze({
    module: 'ui/public-tests/lib/fullCoverageTimelineSourceAuditActions.mjs',
    export: 'createTimelineSourceAuditCoverage',
    sectionFunction: 'secTimelineSourceAudit',
    invocation,
    runnerInvocation: 'runTimelineSourceAuditCoverage',
  })
}

export const FULL_COVERAGE_FOCUSED_SCENARIOS = Object.freeze([
  frozenScenario({
    id: 'e2e-context-ownership-01',
    section: 'ctxmenu',
    factory: CONTEXT_FACTORY,
    actionIds: ['e2e-context-ownership-01'],
  }),
  frozenScenario({
    id: 'e2e-empty-paste-01',
    section: 'ctxmenu',
    factory: CONTEXT_FACTORY,
    actionIds: ['e2e-empty-paste-01'],
  }),
  frozenScenario({
    id: 'e2e-gap-boundary-01',
    section: 'ctxmenu',
    factory: CONTEXT_FACTORY,
    actionIds: ['e2e-gap-boundary-01'],
  }),
  frozenScenario({
    id: 'e2e-trim-tools-01',
    section: 'timeline-source-audit',
    factory: timelineSourceFactory('runTimelineTrimToolAudit'),
    actionIds: ['ripple-trim-start'],
  }),
  frozenScenario({
    id: 'e2e-source-insert-01',
    section: 'timeline-source-audit',
    factory: timelineSourceFactory('sourceInsert'),
    actionIds: ['source-insert'],
  }),
  frozenScenario({
    id: 'e2e-selection-sync-01',
    section: 'timeline-source-audit',
    factory: timelineSourceFactory('selectionSync'),
    actionIds: ['clip'],
  }),
  frozenScenario({
    id: 'e2e-keyboard-focus-01',
    section: 'timeline-source-audit',
    factory: timelineSourceFactory('keyboardFocus'),
    actionIds: ['undo'],
  }),
  frozenScenario({
    id: 'e2e-export-lifecycle-01',
    section: 'export',
    factory: {
      module: 'ui/public-tests/lib/fullCoverageLifecycleExportActions.mjs',
      export: 'runExportLifecycleCoverage',
      sectionFunction: 'secExport',
      invocation: 'runExportLifecycleCoverage',
      runnerInvocation: 'runExportLifecycleCoverage',
    },
    actionIds: EXPORT_LIFECYCLE_ACTION_NAMES,
    actionRequirements: [
      browserAction(EXPORT_LIFECYCLE_ACTION_NAMES[0]),
      browserAction(EXPORT_LIFECYCLE_ACTION_NAMES[1]),
      supportAction(EXPORT_LIFECYCLE_ACTION_NAMES[2], { present: 'na', render: 'na', click: 'na', result: 'pass' }),
    ],
    candidateBound: true,
  }),
  frozenScenario({
    id: 'e2e-relink-derivation-01',
    section: 'residual',
    factory: {
      module: 'ui/public-tests/lib/fullCoverageLifecycleRelinkActions.mjs',
      export: 'runRelinkDerivationCoverage',
      sectionFunction: 'secResidualVerbs',
      invocation: 'runRelinkDerivationCoverage',
      runnerInvocation: 'runRelinkDerivationCoverage',
    },
    actionIds: RELINK_DERIVATION_ACTION_NAMES,
    actionRequirements: RELINK_DERIVATION_ACTION_NAMES.map((actionId) => supportAction(actionId, {
      present: 'pass', render: 'pass', click: 'na', result: 'pass',
    })),
    candidateBound: true,
  }),
  frozenScenario({
    id: INTERACTION_FUZZ_SCENARIO_ID,
    section: 'timeline-source-audit',
    factory: timelineSourceFactory('interactionFuzz.run'),
    actionIds: [INTERACTION_FUZZ_SCENARIO_ID],
    candidateBound: true,
    requiredEnvironment: ['FCV_INTERACTION_FUZZ_SEED'],
  }),
])

export function focusedScenarioById(id) {
  return FULL_COVERAGE_FOCUSED_SCENARIOS.find((scenario) => scenario.id === String(id || '').trim()) || null
}

export function validateFocusedScenarioRequest({ only = '', sections = [], matrixProfile = '', environment = process.env } = {}) {
  const requested = String(only || '').trim()
  if (!requested) return null
  if (!requested.startsWith('e2e-')) return null
  const scenario = focusedScenarioById(requested)
  if (!scenario) throw new Error(`FCV_ONLY=${requested} is not a declared focused full-coverage scenario`)
  if (String(matrixProfile || '').trim()) {
    throw new Error(`${scenario.id} requires its exact FCV_SECTION selector and cannot be combined with FCV_MATRIX_PROFILE`)
  }
  const selectedSections = [...new Set((sections || []).map((value) => String(value).trim()).filter(Boolean))]
  if (selectedSections.length !== 1 || selectedSections[0] !== scenario.selector.section) {
    throw new Error(`${scenario.id} requires exactly FCV_SECTION=${scenario.selector.section}`)
  }
  const missing = scenario.requiredEnvironment.filter((key) => !String(environment[key] || '').trim())
  if (scenario.receipt.candidateBinding.required) {
    missing.push(...scenario.receipt.candidateBinding.environment.filter((key) => !String(environment[key] || '').trim()))
  }
  if (missing.length) {
    throw new Error(`${scenario.id} requires candidate-bound focused evidence; missing ${[...new Set(missing)].join(', ')}`)
  }
  return scenario
}

export function assessFocusedScenarioReachability({ scenario, completedSections = [], rows = [] } = {}) {
  if (!scenario) return Object.freeze({ ok: true, missingSections: [], missingActionIds: [] })
  const missingSections = completedSections.includes(scenario.selector.section) ? [] : [scenario.selector.section]
  const observed = new Set((rows || []).flatMap((row) => [row?.actionId, row?.name]).filter(Boolean))
  const missingActionIds = scenario.execution.emittedActionIds.filter((actionId) => !observed.has(actionId))
  return Object.freeze({
    ok: missingSections.length === 0 && missingActionIds.length === 0,
    missingSections: Object.freeze(missingSections),
    missingActionIds: Object.freeze(missingActionIds),
  })
}

// A focused receipt is useful to Release Studio only when each declared action
// actually yielded a complete four-dimension row. Reachability above deliberately
// remains a small selector/factory check; this companion makes the evidence shape
// fail closed without changing the public reachability result contract.
export function assessFocusedScenarioActionRows({ scenario, rows = [] } = {}) {
  if (!scenario) return Object.freeze({ ok: true, incompleteActionIds: [] })
  const incompleteActionIds = scenario.execution.actionRequirements.filter((requirement) => {
    const actionId = requirement.actionId
    const matches = rows.filter((row) => row?.actionId === actionId || row?.name === actionId)
    return !matches.some((row) => Object.entries(requirement.dimensions).every(([dimension, value]) => row?.[dimension] === value))
  }).map((requirement) => requirement.actionId)
  return Object.freeze({
    ok: incompleteActionIds.length === 0,
    incompleteActionIds: Object.freeze(incompleteActionIds),
  })
}
