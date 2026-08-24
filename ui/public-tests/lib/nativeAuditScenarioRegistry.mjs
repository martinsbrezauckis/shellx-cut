// Declarative native audit aliases for the v0.6.110 P0a registry. They do not
// execute a runner or qualify an artifact: a controller must bind every record
// to one immutable candidate before native proof can change its status.

import { FULL_COVERAGE_RESULT_SCHEMA } from './fullCoverageControlCatalog.mjs'

export const NATIVE_AUDIT_SCENARIO_RECEIPT_SCHEMA = FULL_COVERAGE_RESULT_SCHEMA
export const NATIVE_AUDIT_SCENARIO_STATUSES = Object.freeze([
  'covered',
  'not-applicable',
  'blocked',
  'failed',
  'passed',
])
export const NATIVE_AUDIT_SCENARIO_IDS = Object.freeze([
  'NATIVE-FOCUS-01',
  'NATIVE-PICKER-01',
  'NATIVE-EXPORT-01',
  'NATIVE-COHERENCE-01',
  'NATIVE-AGENT-STUB-01',
])
// This describes an executable source route only. It is deliberately distinct
// from a passed native receipt: every registry row stays blocked until a
// controller binds one installed execution to an immutable candidate.
export const NATIVE_AUDIT_SOURCE_AVAILABILITY = 'implemented-unexecuted'

const CONTROLLER_BINDING_FIELDS = Object.freeze({
  candidate: Object.freeze(['id', 'sourceCommit', 'worktreeId']),
  content: Object.freeze(['manifestSha256']),
  artifact: Object.freeze(['sha256', 'version', 'installed']),
  host: Object.freeze(['id', 'platform', 'session', 'surface']),
  fixture: Object.freeze(['id', 'sha256']),
  command: Object.freeze(['id', 'runnerId']),
  receipt: Object.freeze([
    'schema',
    'scenarioId',
    'candidateId',
    'sourceCommit',
    'contentManifestSha256',
    'artifactSha256',
    'hostId',
    'fixtureId',
    'commandId',
    'inputRoute',
  ]),
})

function frozen(value) {
  return Object.freeze(value)
}

function nativeScenario({
  id,
  runner,
  source,
  requiredSurface,
  inputRoute,
  fixture,
  resultRequirements,
  blocker,
}) {
  return frozen({
    id,
    status: 'blocked',
    runner: frozen(runner),
    source: frozen(source),
    candidate: frozen({ required: CONTROLLER_BINDING_FIELDS.candidate }),
    content: frozen({ required: CONTROLLER_BINDING_FIELDS.content }),
    artifact: frozen({
      required: CONTROLLER_BINDING_FIELDS.artifact,
      requiredInstalled: true,
    }),
    host: frozen({
      required: CONTROLLER_BINDING_FIELDS.host,
      requiredSurface,
    }),
    fixture: frozen(fixture),
    command: frozen({
      required: CONTROLLER_BINDING_FIELDS.command,
      runnerId: runner.id,
      controllerBound: true,
    }),
    receipt: frozen({
      required: CONTROLLER_BINDING_FIELDS.receipt,
      schema: NATIVE_AUDIT_SCENARIO_RECEIPT_SCHEMA,
      controllerOwned: true,
    }),
    input: frozen({
      route: inputRoute,
      nativeOnly: true,
      browserSubstitution: 'forbidden',
    }),
    result: frozen({ required: frozen(resultRequirements) }),
    blocker: frozen(blocker),
  })
}

export const NATIVE_AUDIT_SCENARIOS = Object.freeze([
  nativeScenario({
    id: 'NATIVE-FOCUS-01',
    runner: {
      id: 'windows-installed-full-coverage',
      catalogCommand: 'windows-installed-full-coverage',
      selector: '--diagnostic-section',
    },
    source: {
      runner: 'scripts/windows-installed-full-coverage.mjs',
      nativeInputGuard: 'scripts/release/native-os-action-controller.mjs',
      spec: 'ui/public-tests/lib/fullCoverageNativeFocusActions.mjs',
      factory: 'createNativeFocusMenuEscapeCoverage',
      verifier: frozen({
        module: 'ui/public-tests/full-coverage-verify.mjs',
        invocation: 'runNativeFocusMenuEscapeCoverage(page)',
        section: 'app-chrome-actions',
      }),
      availability: NATIVE_AUDIT_SOURCE_AVAILABILITY,
    },
    requiredSurface: 'windows-installed',
    inputRoute: 'native-keyboard-focus-menu-escape',
    fixture: {
      required: CONTROLLER_BINDING_FIELDS.fixture,
      kind: 'native-focus-fixture',
    },
    resultRequirements: [
      'installed desktop receives focus',
      'WebView document receives focus inside that desktop',
      'menu opens from that focused desktop',
      'native Escape reaches the WebView document',
      'native Escape closes that exact menu',
      'no destructive verb is emitted',
    ],
    blocker: {
      owner: 'ShellX Cut native-test owner: ui/public-tests/lib/fullCoverageNativeFocusActions.mjs',
      remediation: 'Execute the wired installed focus -> menu -> native Escape route for one immutable candidate and persist its controller-owned input and scenario receipts.',
      reason: 'The installed focus implementation is imported and scheduled, but no controller-owned candidate, artifact, host, fixture, and receipt binding has been collected.',
    },
  }),
  nativeScenario({
    id: 'NATIVE-PICKER-01',
    runner: {
      id: 'windows-installed-full-coverage',
      catalogCommand: 'windows-installed-full-coverage',
      selector: '--diagnostic-section=assets',
    },
    source: {
      runner: 'scripts/windows-installed-full-coverage.mjs',
      spec: 'ui/public-tests/lib/fullCoverageAssetsPickerActions.mjs',
      factory: 'createAssetsPickerProbe',
      bridge: frozen({
        module: 'ui/public-tests/lib/fullCoverageAssetsActions.mjs',
        factory: 'createAssetsActionCoverage',
      }),
      verifier: frozen({
        module: 'ui/public-tests/full-coverage-verify.mjs',
        invocation: 'runAssetsActionCoverage(page, { secondMedia: SECOND })',
        section: 'assets',
      }),
      action: 'import-cta',
      availability: NATIVE_AUDIT_SOURCE_AVAILABILITY,
    },
    requiredSurface: 'windows-installed',
    inputRoute: 'native-os-picker-select',
    fixture: {
      required: CONTROLLER_BINDING_FIELDS.fixture,
      kind: 'local-media-import',
    },
    resultRequirements: [
      'host observes and selects a native picker path',
      'media.import returns an asset id',
      'project state contains that imported asset',
      'rendered Assets surface shows the imported result',
    ],
    blocker: {
      owner: 'ShellX Cut Assets native-test owner: ui/public-tests/lib/fullCoverageAssetsPickerActions.mjs',
      remediation: 'Execute the wired import-cta native picker route for one immutable candidate and persist its controller-owned picker and full-coverage receipts.',
      reason: 'The native picker/import/project-state implementation is wired, but no controller-owned candidate, artifact, host, fixture, and receipt binding has been collected.',
    },
  }),
  nativeScenario({
    id: 'NATIVE-EXPORT-01',
    runner: {
      id: 'windows-installed-full-coverage',
      catalogCommand: 'windows-installed-full-coverage',
      selector: '--diagnostic-section=export --diagnostic-only=e2e-export-lifecycle-01',
    },
    source: {
      runner: 'scripts/windows-installed-full-coverage.mjs',
      spec: 'ui/public-tests/lib/fullCoverageLifecycleExportActions.mjs',
      factory: 'runExportLifecycleCoverage',
      verifier: frozen({
        module: 'ui/public-tests/full-coverage-verify.mjs',
        invocation: 'runExportLifecycleCoverage(page, {',
        section: 'export',
      }),
      actions: frozen([
        'e2e-export-lifecycle-01-start-visible-progress',
        'e2e-export-lifecycle-01-targeted-cancel-terminal',
        'e2e-export-lifecycle-01-exact-new-output-identity',
      ]),
      availability: NATIVE_AUDIT_SOURCE_AVAILABILITY,
    },
    requiredSurface: 'windows-installed',
    inputRoute: 'native-installed-export-ui',
    fixture: {
      required: CONTROLLER_BINDING_FIELDS.fixture,
      kind: 'local-render-output',
    },
    resultRequirements: [
      'export progress is rendered while the job is running',
      'targeted cancellation ends non-successfully with no output path',
      'a same-name predecessor hash cannot satisfy the new-output result',
      'the native installed surface and output identity share one receipt',
    ],
    blocker: {
      owner: 'ShellX Cut export native-test owner: ui/public-tests/lib/fullCoverageLifecycleExportActions.mjs',
      remediation: 'Execute the wired lifecycle route for one immutable installed candidate and persist the controller binding that joins native surface, output artifact, and terminal receipt.',
      reason: 'The lifecycle implementation is wired, but no controller-owned candidate, artifact, host, fixture, and receipt binding has been collected for all terminal effects together.',
    },
  }),
  nativeScenario({
    id: 'NATIVE-COHERENCE-01',
    runner: {
      id: 'windows-installed-full-coverage',
      catalogCommand: 'windows-installed-full-coverage',
      selector: 'installed-run receipt set',
    },
    source: {
      runner: 'scripts/windows-installed-full-coverage.mjs',
      installedRuntimeEvidence: 'scripts/lib/installed-runtime-evidence.mjs',
      installedSourceReceipt: 'scripts/lib/windows-installed-source-receipt.mjs',
      spec: 'ui/public-tests/lib/fullCoverageNativeCoherenceActions.mjs',
      factory: 'createNativeCoherenceObservationCoverage',
      verifier: frozen({
        module: 'ui/public-tests/full-coverage-verify.mjs',
        invocation: 'runNativeCoherenceObservationCoverage(page)',
        section: 'app-chrome-actions',
      }),
      availability: NATIVE_AUDIT_SOURCE_AVAILABILITY,
    },
    requiredSurface: 'windows-installed',
    inputRoute: 'installed-native-shell-observation',
    fixture: {
      required: CONTROLLER_BINDING_FIELDS.fixture,
      kind: 'installed-artifact-coherence',
    },
    resultRequirements: [
      'installed shell identity matches the candidate artifact hash and version',
      'Doctor reports the same version and candidate content identity',
      'updater surface resolves to that same candidate identity',
      'no stale install or borrowed receipt is accepted',
    ],
    blocker: {
      owner: 'ShellX Cut native-test owner: ui/public-tests/lib/fullCoverageNativeCoherenceActions.mjs',
      remediation: 'Execute the wired signed installed coherence route for one immutable candidate and persist shell, Doctor, version, and updater observations under the same candidate/artifact receipt.',
      reason: 'The coherence implementation is imported and scheduled, but no controller-owned signed candidate, artifact, host, fixture, and receipt binding has been collected.',
    },
  }),
  nativeScenario({
    id: 'NATIVE-AGENT-STUB-01',
    runner: {
      id: 'windows-installed-full-coverage',
      catalogCommand: 'windows-installed-full-coverage',
      selector: '--diagnostic-section=chat-actions',
    },
    source: {
      runner: 'scripts/windows-installed-full-coverage.mjs',
      spec: 'ui/public-tests/lib/fullCoverageNativeAgentStubActions.mjs',
      factory: 'createNativeAgentStubCoverage',
      verifier: frozen({
        module: 'ui/public-tests/full-coverage-verify.mjs',
        invocation: 'runNativeAgentStubCoverage(page)',
        section: 'chat-actions',
      }),
      deterministicStub: 'scripts/release/fixtures/agent-chat-provider-fixture.mjs',
      mcpFilter: 'scripts/public-tests/agent-chat-containment.test.mjs',
      reviewSupport: 'ui/public-tests/verify-agent-chat-review-server.mjs',
      availability: NATIVE_AUDIT_SOURCE_AVAILABILITY,
    },
    requiredSurface: 'windows-installed',
    inputRoute: 'native-agent-chat-deterministic-local-stub',
    fixture: {
      required: CONTROLLER_BINDING_FIELDS.fixture,
      kind: 'deterministic-local-stub',
      mcpAllowlist: frozen(['cutd']),
      forbidden: frozen(['provider-auth', 'canonical-auth', 'network-provider-route']),
    },
    resultRequirements: [
      'deterministic local stub receives one Agent Chat turn',
      'only the Cut MCP route is exposed and filtered',
      'the turn produces a review update bound to its actor and baseline',
      'provider auth and canonical auth are neither used nor touched',
    ],
    blocker: {
      owner: 'ShellX Cut Agent Chat native-test owner: ui/public-tests/lib/fullCoverageNativeAgentStubActions.mjs',
      remediation: 'Execute the wired installed Agent Chat stub route for one immutable candidate and persist the deterministic fixture, restricted-MCP proof, and review-update receipt without invoking provider or canonical authentication.',
      reason: 'The deterministic fixture, restricted-MCP checks, and native stub implementation are wired, but no controller-owned candidate, artifact, host, fixture, and scenario receipt has been collected.',
    },
  }),
])

function nonEmpty(value) {
  return typeof value === 'string' && value.trim().length > 0
}

function checkRequired(errors, value, fields, label) {
  for (const field of fields) {
    const fieldValue = value?.[field]
    if (fieldValue === undefined || fieldValue === null || fieldValue === '') {
      errors.push(`missing ${label}.${field}`)
    }
  }
}

// This checks only a prospective controller binding. It is deliberately not a
// receipt producer or a native-test runner, and cannot change a record status.
export function validateNativeAuditScenarioBinding(scenario, evidence) {
  const errors = []
  if (!scenario || !NATIVE_AUDIT_SCENARIO_IDS.includes(scenario.id)) {
    return ['unknown native audit scenario']
  }

  checkRequired(errors, evidence?.candidate, scenario.candidate.required, 'candidate')
  checkRequired(errors, evidence?.content, scenario.content.required, 'content')
  checkRequired(errors, evidence?.artifact, scenario.artifact.required, 'artifact')
  checkRequired(errors, evidence?.host, scenario.host.required, 'host')
  checkRequired(errors, evidence?.fixture, scenario.fixture.required, 'fixture')
  checkRequired(errors, evidence?.command, scenario.command.required, 'command')
  checkRequired(errors, evidence?.receipt, scenario.receipt.required, 'receipt')

  if (evidence?.artifact?.installed !== true) errors.push('artifact must be installed')
  if (evidence?.host?.surface !== scenario.host.requiredSurface) {
    errors.push(`wrong host surface: expected ${scenario.host.requiredSurface}`)
  }
  if (evidence?.execution?.surface !== scenario.host.requiredSurface) {
    errors.push(`wrong execution surface: expected ${scenario.host.requiredSurface}`)
  }
  if (evidence?.execution?.kind !== 'native') errors.push('browser-only execution is forbidden')
  if (evidence?.execution?.inputRoute !== scenario.input.route) {
    errors.push(`wrong input route: expected ${scenario.input.route}`)
  }
  if (evidence?.command?.runnerId !== scenario.runner.id) {
    errors.push(`wrong runner: expected ${scenario.runner.id}`)
  }
  if (evidence?.receipt?.schema !== scenario.receipt.schema) errors.push('wrong receipt schema')

  const bindings = [
    ['candidateId', evidence?.candidate?.id],
    ['sourceCommit', evidence?.candidate?.sourceCommit],
    ['contentManifestSha256', evidence?.content?.manifestSha256],
    ['artifactSha256', evidence?.artifact?.sha256],
    ['hostId', evidence?.host?.id],
    ['fixtureId', evidence?.fixture?.id],
    ['commandId', evidence?.command?.id],
    ['inputRoute', evidence?.execution?.inputRoute],
  ]
  for (const [field, expected] of bindings) {
    if (nonEmpty(expected) && evidence?.receipt?.[field] !== expected) {
      errors.push(`borrowed or mismatched receipt ${field}`)
    }
  }

  if (scenario.id === 'NATIVE-AGENT-STUB-01') {
    if (evidence?.fixture?.kind !== 'deterministic-local-stub') errors.push('agent fixture must be a deterministic local stub')
    if (JSON.stringify(evidence?.fixture?.mcpAllowlist || []) !== JSON.stringify(['cutd'])) {
      errors.push('agent fixture must expose only the Cut MCP route')
    }
    if (evidence?.execution?.providerAuthUsed !== false) errors.push('provider-auth route is forbidden')
    if (evidence?.execution?.canonicalAuthTouched !== false) errors.push('canonical-auth route is forbidden')
    if (evidence?.execution?.network !== 'loopback-only') errors.push('provider network route is forbidden')
  }

  return errors
}
