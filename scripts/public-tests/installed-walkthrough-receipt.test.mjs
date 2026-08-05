import test from 'node:test'
import assert from 'node:assert/strict'

import { AGENT_DOCS } from '../lib/agent-docs.mjs'
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
  'setup-btn', 'settings-category:agent-control', 'settings-category:about',
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
      registryVerbs: 260,
      expectedVerbs: 260,
      uiStateSchema: 'shellx-cut/ui-state/2',
      uiClients: 1,
      opens: ['settings-agent-control', 'library', 'settings-about'].map((panel, index) => ({
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
      tools: 260,
      expectedTools: 260,
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

test('builds the bounded six-row installed walkthrough on all three release surfaces', () => {
  for (const surface of Object.keys(PLATFORMS)) {
    const receipt = build(surface)
    assert.equal(receipt.schema, 'shellx-cut/installed-surface-walkthrough@1')
    assert.equal(receipt.surface, surface)
    assert.deepEqual(receipt.rows.map((row) => row.id), [
      'installed-agent-docs', 'settings', 'library', 'about', 'debug-api', 'mcp-self-test',
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
