import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'

import { AGENT_DOCS } from '../lib/agent-docs.mjs'
import {
  installedSurfaceDetour,
  waitForConnectedUi,
} from '../lib/installed-runtime-evidence.mjs'
import {
  beginInstalledStartupReadiness,
  normalizeColdLaunchBudgets,
  validateInstalledStartupReadiness,
} from '../lib/installed-startup-readiness.mjs'
import {
  buildInstalledWalkthroughReceipt,
  INSTALLED_INTEGRITY_SCHEMA,
  INSTALLED_RUNTIME_SCHEMA,
} from '../lib/installed-walkthrough-receipt.mjs'

const SOURCE = {
  gitCommit: 'a'.repeat(40),
  version: '0.6.105',
  contentManifestSha256: 'b'.repeat(64),
}
const ARTIFACT = { sha256: 'c'.repeat(64) }
const ACTION_MANIFEST_SHA = 'a'.repeat(64)
const SETTINGS_ACTIONS = [
  'setup-btn', 'settings-category:agent-control', 'settings-category:health-recovery', 'settings-category:about',
  'health-refresh', 'health-open-assets', 'health-open-recording', 'health-open-toolchain',
  'keymap-toggle', 'agent-control-test', 'environment-close',
]
const LIBRARY_ACTIONS = [
  'library-search', 'library-page-next', 'library-page-prev',
  'library-view-list', 'library-view-grid',
]
const PLATFORMS = {
  'windows-installed': 'win32',
  'macos-installed': 'darwin',
  'linux-control': 'linux',
}
const COMMAND_IDS = {
  'windows-installed': ['authenticode-pre', 'authenticode-post'],
  'macos-installed': [
    'codesign-pre', 'spctl-pre', 'stapler-pre',
    'codesign-post', 'spctl-post', 'stapler-post',
  ],
  'linux-control': ['dpkg-info-pre', 'dpkg-info-post'],
}

async function withUiStateServer(responses, body) {
  let requests = 0
  const server = createServer((request, response) => {
    if (request.method !== 'POST' || request.url !== '/api/verb/ui.state') {
      response.writeHead(404).end()
      return
    }
    const payload = responses[Math.min(requests, responses.length - 1)]
    requests += 1
    response.writeHead(payload.status || 200, { 'content-type': 'application/json' })
    response.end(JSON.stringify(payload.body))
  })
  await new Promise((resolveListen) => server.listen(0, '127.0.0.1', resolveListen))
  const { port } = server.address()
  try {
    await body(`http://127.0.0.1:${port}`, () => requests)
  } finally {
    await new Promise((resolveClose) => server.close(resolveClose))
  }
}

test('installed runtime waits boundedly for a connected UI client', async () => {
  await withUiStateServer([
    { body: { ok: true, result: { schema: 'shellx-cut/ui-state/2', connected: true, ui_clients: 0 } } },
    { status: 409, body: { ok: false, error: { code: 'no_ui_client', message: 'not connected' } } },
    { body: { ok: true, result: { schema: 'shellx-cut/ui-state/2', connected: true, ui_clients: 1 } } },
  ], async (engineBase, requests) => {
    const ready = await waitForConnectedUi(engineBase, { timeoutMs: 500, pollMs: 5 })
    assert.equal(ready.state.result.ui_clients, 1)
    assert.equal(ready.attempts, 3)
    assert.equal(requests(), 3)
    assert.ok(ready.elapsedMs < 500)
  })
})

test('installed runtime UI-client readiness fails within its configured bound', async () => {
  await withUiStateServer([
    { body: { ok: true, result: { schema: 'shellx-cut/ui-state/2', connected: true, ui_clients: 0 } } },
  ], async (engineBase) => {
    const startedAt = Date.now()
    await assert.rejects(
      waitForConnectedUi(engineBase, { timeoutMs: 35, pollMs: 5 }),
      /timed out waiting for installed UI client/,
    )
    assert.ok(Date.now() - startedAt < 1_000)
  })
})

test('installed runtime uses an exclusive workspace detour for an already-open Library', () => {
  assert.equal(installedSurfaceDetour('library', ['library']), 'record')
  assert.equal(
    installedSurfaceDetour('settings-about', ['settings-about', 'settings-agent-control']),
    'settings-health-recovery',
  )
})

test('startup readiness records a pre-spawn t0 and evaluates only caller-supplied budgets', () => {
  const tracker = beginInstalledStartupReadiness({
    surface: 'windows-installed',
    t0EpochMs: 10_000,
    budgets: { listener: 100, api: 150, uiClient: 200, domRoot: 250 },
  })
  tracker.mark('shellSpawned', { atEpochMs: 10_005, evidence: { launcher: 'fixture' } })
  tracker.mark('listener', { atEpochMs: 10_050, evidence: { cdp: 'loopback' } })
  tracker.mark('api', { atEpochMs: 10_060, evidence: { endpoint: '/api/verbs' } })
  tracker.mark('uiClient', { atEpochMs: 10_090, evidence: { clients: 1 } })
  tracker.mark('domRoot', { atEpochMs: 10_120, evidence: { selector: '[data-cut-app-root]' } })
  tracker.mark('slowFfmpeg', { atEpochMs: 10_300, evidence: { marker: 'test-owned' } })
  const receipt = tracker.build({ generatedAt: '2026-08-27T00:00:00.000Z' })

  assert.equal(receipt.status, 'measured')
  assert.equal(receipt.classification, 'measurement-only')
  assert.equal(receipt.t0.meaning, 'recorded before native shell spawn')
  assert.equal(receipt.checkpoints.domRoot.elapsedMs, 120)
  assert.equal(receipt.slowFfmpeg.uiAndDomPrecedeMarker, true)
  assert.equal(receipt.budget.status, 'pass')
  assert.equal(receipt.budget.claimEligible, true)
  assert.equal(validateInstalledStartupReadiness(receipt, {
    surface: 'windows-installed', requireBudget: true,
  }), receipt)
})

test('startup readiness does not invent a budget and rejects a slow-marker race', () => {
  const tracker = beginInstalledStartupReadiness({
    surface: 'macos-installed',
    t0EpochMs: 20_000,
  })
  tracker.mark('shellSpawned', { atEpochMs: 20_001 })
  tracker.mark('listener', { atEpochMs: 20_020 })
  tracker.mark('api', { atEpochMs: 20_030 })
  tracker.mark('uiClient', { atEpochMs: 20_040 })
  tracker.mark('domRoot', { atEpochMs: 20_050 })
  const receipt = tracker.build()
  assert.equal(receipt.budget.status, 'caller-budget-required')
  assert.equal(receipt.budget.claimEligible, false)
  assert.deepEqual(receipt.budget.missing, ['listener', 'api', 'uiClient', 'domRoot'])
  assert.throws(() => validateInstalledStartupReadiness(receipt, { requireBudget: true }), /caller-supplied/)

  const raced = beginInstalledStartupReadiness({ surface: 'windows-installed', t0EpochMs: 30_000 })
  raced.mark('shellSpawned', { atEpochMs: 30_001 })
  raced.mark('listener', { atEpochMs: 30_020 })
  raced.mark('api', { atEpochMs: 30_030 })
  raced.mark('uiClient', { atEpochMs: 30_060 })
  raced.mark('domRoot', { atEpochMs: 30_090 })
  raced.mark('slowFfmpeg', { atEpochMs: 30_080 })
  assert.throws(() => raced.build(), /DOM root readiness did not precede/)
})

test('cold-launch budget parsing rejects partial, unknown, and non-positive policies', () => {
  assert.deepEqual(normalizeColdLaunchBudgets({ listener: 1 }), {
    configured: false,
    missing: ['api', 'uiClient', 'domRoot'],
    ms: { listener: 1 },
  })
  assert.throws(() => normalizeColdLaunchBudgets({ listener: 0 }), /positive integer/)
  assert.throws(() => normalizeColdLaunchBudgets({ frame: 500 }), /unknown cold-launch budget/)
})

function runtime(surface) {
  return {
    schema: INSTALLED_RUNTIME_SCHEMA,
    status: 'pass',
    installedApp: true,
    surface,
    source: { ...SOURCE },
    host: { platform: PLATFORMS[surface], arch: 'test' },
    agentDocs: {
      schema: 'shellx-cut/installed-agent-docs-verify@1',
      version: SOURCE.version,
      checked: AGENT_DOCS.length,
      served: AGENT_DOCS.length,
      failures: [],
    },
    debugApi: {
      registryVerbs: 264,
      expectedVerbs: 264,
      uiStateSchema: 'shellx-cut/ui-state/2',
      uiClients: 1,
      opens: ['settings-agent-control', 'settings-health-recovery', 'library', 'settings-about'].map((panel, index) => ({
        panel,
        applied: true,
        stateRevision: index + 1,
        visual: surface === 'macos-installed' ? { ok: true, mode: 'screenshot' } : null,
      })),
    },
    mcp: {
      schema: 'shellx-cut/mcp-self-test/1',
      mode: 'proxy',
      readOnly: true,
      ping: true,
      sameEngine: true,
      tools: 264,
      expectedTools: 264,
    },
    webdriverTestFeature: surface === 'linux-control'
      ? { absent: true, proof: 'binary-marker-absent', bytesChecked: 1024 }
      : { absent: true, proof: 'closed-port' },
    webdriverTestPort: surface === 'linux-control'
      ? { port: 4445, closed: false, outcome: 'connected' }
      : { port: 4445, closed: true, outcome: 'ECONNREFUSED' },
  }
}

function fullCoverage(surface) {
  const results = [...SETTINGS_ACTIONS, ...LIBRARY_ACTIONS].map((actionId) => ({
    actionId,
    rowKind: 'ui_action',
    present: 'pass',
    render: 'pass',
    click: 'pass',
    result: 'pass',
  }))
  return {
    schema: 'shellx-cut/full-coverage-results@1',
    ok: true,
    full: true,
    strictAllActions: true,
    surface,
    runtime: {
      installedApp: true,
      nativeAttached: true,
      sourceContentManifestSha256: SOURCE.contentManifestSha256,
    },
    actionManifest: { sha256: 'd'.repeat(64) },
    sourceActionManifest: {
      sha256: ACTION_MANIFEST_SHA,
      expectedSha256: ACTION_MANIFEST_SHA,
      matchesExpected: true,
    },
    runtimeSourceActionManifest: {
      sha256: ACTION_MANIFEST_SHA,
      expectedSha256: ACTION_MANIFEST_SHA,
      total: 667,
      matchesExpected: true,
    },
    summary: { controls: { strictUnverified: 0, failures: 0 } },
    results,
  }
}

function integrity(surface) {
  return {
    schema: INSTALLED_INTEGRITY_SCHEMA,
    status: 'pass',
    surface,
    source: { ...SOURCE },
    artifact: {
      sha256: ARTIFACT.sha256,
      preUseSha256: ARTIFACT.sha256,
      postUseSha256: ARTIFACT.sha256,
    },
    webdriverTestFeatureAbsent: true,
    signed: surface === 'windows-installed',
    notarized: surface === 'macos-installed',
    commands: COMMAND_IDS[surface].map((id) => ({
      id,
      executable: 'native-verifier',
      args: ['--verify'],
      status: 0,
      outputSha256: 'e'.repeat(64),
    })),
  }
}

function build(surface, mutate = () => {}) {
  const inputs = {
    source: structuredClone(SOURCE),
    surface,
    artifact: structuredClone(ARTIFACT),
    runtimeEvidence: runtime(surface),
    fullCoverageReceipt: surface === 'macos-installed' ? null : fullCoverage(surface),
    integrityEvidence: integrity(surface),
  }
  mutate(inputs)
  return buildInstalledWalkthroughReceipt(inputs)
}

test('builds the bounded seven-row installed walkthrough on all three release surfaces', () => {
  for (const surface of Object.keys(PLATFORMS)) {
    const receipt = build(surface)
    assert.equal(receipt.schema, 'shellx-cut/installed-surface-walkthrough@1')
    assert.equal(receipt.surface, surface)
    assert.deepEqual(receipt.rows.map((row) => row.id), [
      'installed-agent-docs', 'settings', 'health-recovery', 'library', 'about', 'debug-api', 'mcp-self-test',
    ])
    assert.equal(receipt.rows.every((row) => row.status === 'pass'), true)
  }
})

test('rejects source, artifact, platform, candidate-only and WebDriver mismatches', () => {
  for (const mutate of [
    (input) => { input.runtimeEvidence.source.gitCommit = 'f'.repeat(40) },
    (input) => { input.integrityEvidence.artifact.postUseSha256 = 'f'.repeat(64) },
    (input) => { input.runtimeEvidence.host.platform = 'linux' },
    (input) => { input.fullCoverageReceipt.runtime.installedApp = false },
    (input) => { input.runtimeEvidence.webdriverTestPort.closed = false },
  ]) {
    assert.throws(() => build('windows-installed', mutate))
  }
})

test('requires binary marker absence proof for the external Linux driver path', () => {
  assert.throws(() => build('linux-control', (input) => {
    input.runtimeEvidence.webdriverTestFeature = { absent: true, proof: 'closed-port' }
  }))
})

test('rejects every missing installed runtime row and generic MCP or Debug API claims', () => {
  const mutations = [
    (input) => { input.runtimeEvidence.agentDocs.served -= 1 },
    (input) => { input.runtimeEvidence.debugApi.opens.splice(0, 1) },
    (input) => { input.runtimeEvidence.debugApi.registryVerbs -= 1 },
    (input) => { input.runtimeEvidence.mcp.sameEngine = false },
  ]
  for (const mutate of mutations) assert.throws(() => build('linux-control', mutate))
})

test('rejects forged or incomplete full-coverage source-action manifests', () => {
  for (const mutate of [
    (input) => { input.fullCoverageReceipt.runtimeSourceActionManifest.sha256 = 'f'.repeat(64) },
    (input) => { input.fullCoverageReceipt.sourceActionManifest.expectedSha256 = 'f'.repeat(64) },
    (input) => { input.fullCoverageReceipt.runtimeSourceActionManifest.total = 0 },
  ]) assert.throws(() => build('linux-control', mutate), /source-action inventory mismatch/)
})

test('rejects each required Settings and Library action when the installed matrix omits it', () => {
  for (const actionId of [...SETTINGS_ACTIONS, ...LIBRARY_ACTIONS]) {
    assert.throws(() => build('windows-installed', (input) => {
      input.fullCoverageReceipt.results = input.fullCoverageReceipt.results
        .filter((row) => row.actionId !== actionId)
    }), new RegExp(actionId.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')))
  }
})

test('rejects generic, incomplete, failed, unsigned, and unnotarized integrity claims', () => {
  assert.throws(() => build('linux-control', (input) => { input.integrityEvidence.commands = [] }))
  assert.throws(() => build('linux-control', (input) => { input.integrityEvidence.commands[0].status = 1 }))
  assert.throws(() => build('windows-installed', (input) => { input.integrityEvidence.signed = false }))
  assert.throws(() => build('macos-installed', (input) => { input.integrityEvidence.notarized = false }))
  assert.throws(() => build('macos-installed', (input) => {
    input.runtimeEvidence.debugApi.opens[1].visual.ok = false
  }))
})
