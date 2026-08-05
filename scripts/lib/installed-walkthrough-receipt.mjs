import { AGENT_DOCS } from './agent-docs.mjs'
import {
  INSTALLED_RUNTIME_SCHEMA,
  REQUIRED_OPEN_SURFACES,
  SURFACE_PLATFORMS,
} from './installed-runtime-evidence.mjs'

export {
  collectInstalledRuntimeEvidence,
  INSTALLED_RUNTIME_SCHEMA,
  probeClosedPort,
} from './installed-runtime-evidence.mjs'
export const INSTALLED_INTEGRITY_SCHEMA = 'shellx-cut/installed-artifact-integrity@1'
export const INSTALLED_WALKTHROUGH_SCHEMA = 'shellx-cut/installed-surface-walkthrough@1'

const SHA256_RX = /^[a-f0-9]{64}$/
const COMMIT_RX = /^[a-f0-9]{40}$/
const REQUIRED_SETTINGS_ACTIONS = [
  'setup-btn',
  'settings-category:agent-control',
  'settings-category:about',
  'keymap-toggle',
  'agent-control-test',
  'environment-close',
]
const REQUIRED_LIBRARY_ACTIONS = [
  'library-search',
  'library-page-next',
  'library-page-prev',
  'library-view-list',
  'library-view-grid',
]
const REQUIRED_INTEGRITY_COMMANDS = {
  'windows-installed': ['authenticode-pre', 'authenticode-post'],
  'macos-installed': [
    'codesign-pre', 'spctl-pre', 'stapler-pre',
    'codesign-post', 'spctl-post', 'stapler-post',
  ],
  'linux-control': ['dpkg-info-pre', 'dpkg-info-post'],
}

function assert(condition, message) {
  if (!condition) throw new Error(message)
}

function validateRuntime(runtime, source, surface) {
  assert(runtime?.schema === INSTALLED_RUNTIME_SCHEMA && runtime.status === 'pass'
    && runtime.installedApp === true, 'installed runtime receipt is not a pass')
  assert(runtime.surface === surface, 'installed runtime receipt surface mismatch')
  assert(JSON.stringify(runtime.source) === JSON.stringify(source), 'installed runtime source mismatch')
  assert(runtime.host?.platform === SURFACE_PLATFORMS[surface], 'installed runtime platform mismatch')
  assert(runtime.agentDocs?.checked === AGENT_DOCS.length
    && runtime.agentDocs?.served === AGENT_DOCS.length
    && runtime.agentDocs?.version === source.version
    && Array.isArray(runtime.agentDocs?.failures)
    && runtime.agentDocs.failures.length === 0, 'installed agent-doc runtime proof is incomplete')
  assert(runtime.debugApi?.registryVerbs === runtime.debugApi?.expectedVerbs
    && runtime.debugApi.registryVerbs > 0
    && runtime.debugApi.uiStateSchema === 'shellx-cut/ui-state/2'
    && runtime.debugApi.uiClients > 0, 'installed Debug API proof is incomplete')
  assert(JSON.stringify(runtime.debugApi.opens?.map((item) => item.panel))
    === JSON.stringify(REQUIRED_OPEN_SURFACES), 'installed UI-open proof is incomplete or out of order')
  for (const opened of runtime.debugApi.opens) {
    assert(opened.applied === true && Number.isInteger(opened.stateRevision) && opened.stateRevision > 0,
      `installed UI-open proof failed for ${opened.panel}`)
  }
  assert(runtime.mcp?.schema === 'shellx-cut/mcp-self-test/1'
    && runtime.mcp.mode === 'proxy'
    && runtime.mcp.readOnly === true
    && runtime.mcp.ping === true
    && runtime.mcp.sameEngine === true
    && runtime.mcp.tools > 0
    && runtime.mcp.tools === runtime.mcp.expectedTools,
  'installed MCP proof is incomplete')
  assert(runtime.webdriverTestFeature?.absent === true,
    'installed app did not prove the WebDriver test feature absent')
  if (surface === 'linux-control') {
    assert(runtime.webdriverTestFeature.proof === 'binary-marker-absent'
      && runtime.webdriverTestFeature.bytesChecked > 0,
    'Linux installed app did not prove the WebDriver build marker absent')
  } else {
    assert(runtime.webdriverTestFeature.proof === 'closed-port'
      && runtime.webdriverTestPort?.port === 4445
      && runtime.webdriverTestPort?.closed === true
      && runtime.webdriverTestPort?.outcome === 'ECONNREFUSED',
    'installed app did not prove the embedded WebDriver port absent while running')
  }
}

function passingAction(receipt, actionId) {
  const matches = (receipt?.results || []).filter((row) => row?.actionId === actionId)
  return matches.length === 1 && matches[0].rowKind === 'ui_action'
    && ['present', 'render', 'click', 'result'].every((field) => matches[0][field] === 'pass')
}

function validateFullCoverage(receipt, source, surface) {
  if (surface === 'macos-installed') return
  assert(receipt?.schema === 'shellx-cut/full-coverage-results@1'
    && receipt.ok === true && receipt.full === true && receipt.strictAllActions === true,
  'installed full-coverage receipt is not a strict pass')
  assert(receipt.surface === surface, 'installed full-coverage surface mismatch')
  assert(receipt.runtime?.installedApp === true && receipt.runtime?.nativeAttached === true,
    'full-coverage receipt is not attached to an installed app')
  assert(receipt.runtime?.sourceContentManifestSha256 === source.contentManifestSha256,
    'full-coverage synchronized-content digest mismatch')
  const sourceActions = receipt.sourceActionManifest
  const runtimeActions = receipt.runtimeSourceActionManifest
  assert(sourceActions?.matchesExpected === true
    && runtimeActions?.matchesExpected === true
    && SHA256_RX.test(String(sourceActions.sha256 || ''))
    && SHA256_RX.test(String(sourceActions.expectedSha256 || ''))
    && SHA256_RX.test(String(runtimeActions.sha256 || ''))
    && SHA256_RX.test(String(runtimeActions.expectedSha256 || ''))
    && sourceActions.sha256 === sourceActions.expectedSha256
    && runtimeActions.sha256 === runtimeActions.expectedSha256
    && runtimeActions.sha256 === sourceActions.sha256
    && Number.isInteger(runtimeActions.total) && runtimeActions.total > 0,
  'full-coverage source-action inventory mismatch')
  assert(receipt.summary?.controls?.strictUnverified === 0
    && receipt.summary?.controls?.failures === 0,
  'full-coverage receipt contains failed or unverified UI actions')
  for (const actionId of [...REQUIRED_SETTINGS_ACTIONS, ...REQUIRED_LIBRARY_ACTIONS]) {
    assert(passingAction(receipt, actionId), `full-coverage receipt lacks passing '${actionId}'`)
  }
}

function validateIntegrity(integrity, artifact, source, surface) {
  assert(integrity?.schema === INSTALLED_INTEGRITY_SCHEMA && integrity.status === 'pass',
    'installed artifact integrity receipt is not a pass')
  assert(integrity.surface === surface, 'installed artifact integrity surface mismatch')
  assert(integrity.source?.gitCommit === source.gitCommit
    && integrity.source?.version === source.version
    && integrity.source?.contentManifestSha256 === source.contentManifestSha256,
  'installed artifact integrity source mismatch')
  assert(integrity.artifact?.sha256 === artifact.sha256
    && integrity.artifact?.preUseSha256 === artifact.sha256
    && integrity.artifact?.postUseSha256 === artifact.sha256,
  'installed artifact changed during qualification')
  assert(integrity.webdriverTestFeatureAbsent === true, 'shipping artifact WebDriver feature absence is unverified')
  const commands = Array.isArray(integrity.commands) ? integrity.commands : []
  const commandIds = commands.map((command) => command?.id)
  assert(JSON.stringify(commandIds) === JSON.stringify(REQUIRED_INTEGRITY_COMMANDS[surface]),
    'native artifact verification commands are incomplete or out of order')
  assert(new Set(commandIds).size === commandIds.length
    && commands.every((command) => command.status === 0
      && typeof command.executable === 'string' && command.executable.length > 0
      && Array.isArray(command.args)
      && SHA256_RX.test(String(command.outputSha256 || ''))),
  'native artifact verification commands are missing, failed, or unbound')
  if (surface === 'windows-installed') assert(integrity.signed === true, 'Windows shipping artifact is unsigned')
  if (surface === 'macos-installed') assert(integrity.notarized === true, 'macOS shipping artifact is not notarized')
}

export function buildInstalledWalkthroughReceipt({
  source,
  surface,
  artifact,
  runtimeEvidence,
  fullCoverageReceipt = null,
  integrityEvidence,
  generatedAt = new Date().toISOString(),
}) {
  assert(SURFACE_PLATFORMS[surface], `unknown installed surface: ${surface}`)
  assert(COMMIT_RX.test(String(source?.gitCommit || ''))
    && /^\d+\.\d+\.\d+/.test(String(source?.version || ''))
    && SHA256_RX.test(String(source?.contentManifestSha256 || '')), 'installed source identity is invalid')
  assert(SHA256_RX.test(String(artifact?.sha256 || '')), 'installed artifact digest is invalid')
  validateRuntime(runtimeEvidence, source, surface)
  validateFullCoverage(fullCoverageReceipt, source, surface)
  validateIntegrity(integrityEvidence, artifact, source, surface)
  if (surface === 'macos-installed') {
    for (const opened of runtimeEvidence.debugApi.opens) {
      assert(opened.visual?.ok === true, `macOS installed visual evidence missing for ${opened.panel}`)
    }
  }

  const matrixHash = fullCoverageReceipt?.actionManifest?.sha256 || null
  return {
    schema: INSTALLED_WALKTHROUGH_SCHEMA,
    generatedAt,
    status: 'pass',
    installedApp: true,
    surface,
    source: { ...source },
    artifact: {
      sha256: artifact.sha256,
      version: source.version,
      integrityVerified: true,
      webdriverTestFeatureAbsent: true,
      signed: integrityEvidence.signed === true,
      notarized: integrityEvidence.notarized === true,
    },
    rows: [
      { id: 'installed-agent-docs', status: 'pass', evidence: { files: runtimeEvidence.agentDocs.served } },
      { id: 'settings', status: 'pass', evidence: { actionManifestSha256: matrixHash, surface: 'settings-agent-control' } },
      { id: 'library', status: 'pass', evidence: { actionManifestSha256: matrixHash, surface: 'library' } },
      { id: 'about', status: 'pass', evidence: { version: source.version, surface: 'settings-about' } },
      { id: 'debug-api', status: 'pass', evidence: { verbs: runtimeEvidence.debugApi.registryVerbs, uiState: runtimeEvidence.debugApi.uiStateSchema } },
      { id: 'mcp-self-test', status: 'pass', evidence: { tools: runtimeEvidence.mcp.tools, sameEngine: true } },
    ],
  }
}
