import {
  NATIVE_AUDIT_SCENARIOS,
  validateNativeAuditScenarioBinding,
} from '../../ui/public-tests/lib/nativeAuditScenarioRegistry.mjs'

const SHA256 = /^[a-f0-9]{64}$/
const COMMIT = /^[a-f0-9]{40}$/
const VERSION = /^\d+\.\d+\.\d+/
const FOCUS = 'NATIVE-FOCUS-01'
const PICKER = 'NATIVE-PICKER-01'
const EXPORT = 'NATIVE-EXPORT-01'
const COHERENCE = 'NATIVE-COHERENCE-01'
const AGENT_STUB = 'NATIVE-AGENT-STUB-01'
const DESTRUCTIVE_VERBS = new Set([
  'edit.remove', 'edit.ripple_delete', 'jobs.cancel', 'library.folder_remove',
  'library.remove', 'media.remove', 'project.delete', 'project.forget',
  'project.revert', 'screen_record.discard',
])

function present(value) {
  return typeof value === 'string' && value.trim().length > 0
}

function exact(value, expected, label, errors) {
  if (value !== expected) errors.push(`${label} must equal ${expected}`)
}

function hash(value, label, errors) {
  if (!SHA256.test(String(value || ''))) errors.push(`${label} must be a sha256`)
}

function commit(value, label, errors) {
  if (!COMMIT.test(String(value || ''))) errors.push(`${label} must be a full git hash`)
}

function allPass(row) {
  return ['present', 'render', 'click', 'result'].every((field) => row?.[field] === 'pass')
}

function requireRow(receipt, actionId, errors, { terminalOnly = false } = {}) {
  const row = receipt?.fullCoverage?.actions?.find((item) => item?.actionId === actionId)
  if ((terminalOnly ? row?.result !== 'pass' : !allPass(row))) {
    errors.push(`full coverage action ${actionId} lacks its required passing result`)
  }
}

function bindingFor(receipt) {
  return {
    candidate: receipt?.candidate,
    content: receipt?.content,
    artifact: receipt?.artifact,
    host: receipt?.host,
    fixture: receipt?.fixture,
    command: receipt?.command,
    execution: receipt?.execution,
    receipt: receipt?.binding,
  }
}

function validateIdentity(receipt, errors) {
  const candidate = receipt?.candidate
  if (!present(candidate?.id)) errors.push('missing candidate id')
  commit(candidate?.sourceCommit, 'candidate source commit', errors)
  commit(candidate?.gitTree, 'candidate git tree', errors)
  if (!present(candidate?.worktreeId)) errors.push('missing candidate worktree id')
  hash(receipt?.content?.manifestSha256, 'candidate content manifest', errors)
  if (!present(receipt?.artifact?.path)) errors.push('missing installed artifact path')
  hash(receipt?.artifact?.sha256, 'installed artifact hash', errors)
  if (!VERSION.test(String(receipt?.artifact?.version || ''))) errors.push('installed artifact version is invalid')
  if (receipt?.artifact?.installed !== true) errors.push('artifact must be installed')
  if (!present(receipt?.host?.id) || !present(receipt?.host?.session)) errors.push('host id and session are required')
  exact(receipt?.host?.platform, 'win32', 'host platform', errors)
  exact(receipt?.host?.surface, 'windows-installed', 'host surface', errors)
  if (!present(receipt?.fixture?.id)) errors.push('fixture id is required')
  hash(receipt?.fixture?.sha256, 'fixture hash', errors)
  exact(receipt?.command?.id, 'windows-installed-full-coverage', 'command id', errors)
  exact(receipt?.command?.runnerId, 'windows-installed-full-coverage', 'runner id', errors)
}

function validateInstalledExecution(receipt, errors) {
  const execution = receipt?.execution
  exact(execution?.kind, 'native', 'execution kind', errors)
  exact(execution?.observedBy, 'installed-run', 'execution provenance', errors)
  exact(execution?.surface, 'windows-installed', 'execution surface', errors)
  exact(execution?.driver, 'webview2-cdp', 'execution driver', errors)
  if (execution?.installedApp !== true) errors.push('execution is not an installed app')
  if (execution?.nativeAttached !== true) errors.push('browser-only execution is forbidden')
  if (execution?.sourceClaim === true) errors.push('direct static/source claim is forbidden')
}

function validateFullCoverage(receipt, errors) {
  const coverage = receipt?.fullCoverage
  exact(coverage?.schema, 'shellx-cut/full-coverage-results@1', 'full coverage schema', errors)
  if (coverage?.ok !== true || coverage?.full !== true || coverage?.strictAllActions !== true) {
    errors.push('full coverage receipt is not a passing strict full run')
  }
  exact(coverage?.surface, 'windows-installed', 'full coverage surface', errors)
  exact(coverage?.sourceCommit, receipt?.candidate?.sourceCommit, 'full coverage source commit', errors)
  exact(coverage?.contentManifestSha256, receipt?.content?.manifestSha256, 'full coverage content manifest', errors)
  exact(coverage?.artifactSha256, receipt?.artifact?.sha256, 'full coverage artifact hash', errors)
  if (!SHA256.test(String(coverage?.sha256 || ''))) errors.push('full coverage receipt hash is invalid')
}

function validateFocus(receipt, errors) {
  const proof = receipt?.proof?.focus
  exact(receipt?.inputRoute, 'native-keyboard-focus-menu-escape', 'focus input route', errors)
  for (const [name, command] of [['focus', 'focus'], ['escape', 'escape']]) {
    const item = proof?.[name]
    exact(item?.schema, 'shellx-cut/windows-native-focus@1', `${name} receipt schema`, errors)
    exact(item?.command, command, `${name} receipt command`, errors)
    if (item?.ok !== true) errors.push(`${name} receipt is not successful`)
    if (!Number.isInteger(item?.expectedPid) || item?.foregroundPid !== item?.expectedPid ||
        item?.foregroundProcess !== item?.expectedProcess || item?.expectedProcess !== 'shellx-cut') {
      errors.push(`${name} receipt is not bound to the foreground ShellX Cut process`)
    }
  }
  if (proof?.escape?.key !== 'Escape' || proof?.escape?.transport !== 'SendKeys') {
    errors.push('focus proof did not deliver native Escape through SendKeys')
  }
  const documentFocus = proof?.documentFocus
  if (documentFocus?.schema !== 'shellx-cut/windows-native-focus-document@1' ||
      documentFocus?.command !== 'focus-webview-document' || documentFocus?.ok !== true ||
      documentFocus?.documentFocused !== true || documentFocus?.targetFocused !== true ||
      documentFocus?.insideExportMenu !== true) {
    errors.push('focus proof does not establish focus inside the WebView document')
  }
  const documentEscape = proof?.documentEscape
  if (documentEscape?.schema !== 'shellx-cut/windows-native-focus-document@1' ||
      documentEscape?.command !== 'escape-keydown' || documentEscape?.armed !== true ||
      documentEscape?.received !== true || documentEscape?.event?.type !== 'keydown' ||
      documentEscape?.event?.key !== 'Escape' || documentEscape?.event?.isTrusted !== true) {
    errors.push('focus proof does not show native Escape reaching the WebView document')
  }
  if (proof?.menu?.opened !== true || proof?.menu?.closed !== true || proof?.menu?.trigger !== 'data-cut-export-btn') {
    errors.push('focus proof did not open and close the exact export menu')
  }
  const operations = proof?.operations
  if (!Number.isInteger(operations?.beforeCount) || !Number.isInteger(operations?.afterCount) ||
      operations.afterCount < operations.beforeCount || !Array.isArray(operations?.newVerbs) ||
      operations.newVerbs.length !== operations.afterCount - operations.beforeCount ||
      operations.newVerbs.some((verb) => typeof verb !== 'string' || !verb || DESTRUCTIVE_VERBS.has(verb))) {
    errors.push('focus proof does not prove a destructive-verb-free operation tail')
  }
  requireRow(receipt, 'native-focus-menu-escape', errors)
}

function validatePicker(receipt, errors) {
  const proof = receipt?.proof?.picker
  exact(receipt?.inputRoute, 'native-os-picker-select', 'picker input route', errors)
  exact(proof?.nativeSelection?.actionId, 'import-cta', 'picker action id', errors)
  exact(proof?.nativeSelection?.mode, 'select', 'picker action mode', errors)
  if (proof?.nativeSelection?.controller !== true || proof?.nativeSelection?.completed !== true) {
    errors.push('picker native controller proof is incomplete')
  }
  if (!present(proof?.nativeSelection?.selectedPath)) errors.push('picker selected path is missing')
  if (!present(proof?.assetId) || proof?.mediaImportOk !== true || proof?.projectHasAsset !== true) {
    errors.push('picker media.import and project-state proof is incomplete')
  }
  if (proof?.renderedAssetsCard !== true) errors.push('picker Assets render proof is incomplete')
  exact(proof?.assetId, proof?.renderedAssetId, 'rendered Assets asset id', errors)
  requireRow(receipt, 'import-cta', errors)
}

function validateExport(receipt, errors) {
  const proof = receipt?.proof?.export
  exact(receipt?.inputRoute, 'native-installed-export-ui', 'export input route', errors)
  if (proof?.progress?.rendered !== true || !present(proof?.progress?.jobId)) {
    errors.push('export progress proof is incomplete')
  }
  if (proof?.cancel?.requested !== true || proof?.cancel?.jobId !== proof?.progress?.jobId) {
    errors.push('export cancellation is not targeted to the running job')
  }
  if (proof?.cancel?.terminalState === 'done' || !present(proof?.cancel?.terminalState)) {
    errors.push('export cancellation finished successfully or has no terminal state')
  }
  if (proof?.cancel?.outputPath) errors.push('cancelled export reported an output path')
  if (proof?.predecessor?.unchanged !== true || !SHA256.test(String(proof?.predecessor?.sha256 || ''))) {
    errors.push('same-name predecessor proof is incomplete')
  }
  if (proof?.fresh?.terminalState !== 'done' || proof?.fresh?.exactPath !== true ||
      !present(proof?.fresh?.path) || proof?.fresh?.path !== proof?.fresh?.expectedPath ||
      !SHA256.test(String(proof?.fresh?.sha256 || '')) || proof.fresh.sha256 === proof.predecessor?.sha256) {
    errors.push('fresh same-name output identity is incomplete')
  }
  requireRow(receipt, 'e2e-export-lifecycle-01-start-visible-progress', errors)
  requireRow(receipt, 'e2e-export-lifecycle-01-targeted-cancel-terminal', errors)
  requireRow(receipt, 'e2e-export-lifecycle-01-exact-new-output-identity', errors, { terminalOnly: true })
}

function validateCoherence(receipt, errors) {
  const proof = receipt?.proof?.coherence
  exact(receipt?.inputRoute, 'installed-native-shell-observation', 'coherence input route', errors)
  if (proof?.signedFinal !== true) errors.push('coherence proof is not from a signed-final installed candidate')
  exact(proof?.candidateId, receipt?.candidate?.id, 'coherence candidate id', errors)
  for (const [field, value] of [
    ['gitCommit', receipt?.candidate?.sourceCommit],
    ['contentManifestSha256', receipt?.content?.manifestSha256],
    ['version', receipt?.artifact?.version],
  ]) exact(proof?.source?.[field], value, `coherence source ${field}`, errors)
  exact(proof?.shell?.sha256, receipt?.artifact?.sha256, 'coherence shell hash', errors)
  exact(proof?.shell?.version, receipt?.artifact?.version, 'coherence shell version', errors)
  exact(proof?.cutd?.sha256, receipt?.candidate?.cutdSha256, 'coherence cutd hash', errors)
  exact(proof?.cutd?.version, receipt?.artifact?.version, 'coherence cutd version', errors)
  exact(proof?.doctor?.appVersion, receipt?.artifact?.version, 'Doctor version', errors)
  exact(proof?.updater?.current, receipt?.artifact?.version, 'updater current version', errors)
  if (!present(proof?.updater?.status)) errors.push('updater status is missing')
  requireRow(receipt, 'native-coherence-observation', errors)
}

function validateAgentStub(receipt, errors) {
  const proof = receipt?.proof?.agent
  exact(receipt?.inputRoute, 'native-agent-chat-deterministic-local-stub', 'agent input route', errors)
  exact(proof?.fixture?.provider, 'claude', 'agent fixture provider', errors)
  if (proof?.fixture?.deterministicLocalStub !== true ||
      JSON.stringify(proof?.fixture?.mcpAllowlist) !== JSON.stringify(['cutd'])) {
    errors.push('agent proof does not retain the deterministic cutd-only fixture')
  }
  if (proof?.policy?.providerAuthUsed !== false || proof?.policy?.canonicalAuthTouched !== false ||
      proof?.policy?.network !== 'loopback-only' || proof?.policy?.turns !== 1) {
    errors.push('agent proof permits provider auth, canonical auth, provider network, or extra turns')
  }
  if (proof?.chat?.visible !== true || proof?.chat?.agentReplyVisible !== true || proof?.chat?.reviewVisible !== true) {
    errors.push('agent chat or review update is not visibly rendered')
  }
  if (proof?.tool?.route !== 'cutd' || proof?.tool?.invoked !== true ||
      JSON.stringify(proof?.tool?.exposed) !== JSON.stringify(['cutd'])) {
    errors.push('agent proof does not show one restricted Cut MCP tool route')
  }
  if (!present(proof?.review?.baseline) || proof?.review?.tip !== proof?.appliedEdit?.opId ||
      proof?.review?.revertSafe !== true || !present(proof?.review?.turnId) ||
      proof?.appliedEdit?.visible !== true || proof?.appliedEdit?.actor?.name !== proof?.review?.turnId ||
      proof?.appliedEdit?.actor?.via !== 'agent.chat') {
    errors.push('agent review and applied edit are not bound to the same turn')
  }
  requireRow(receipt, 'native-agent-stub-review-update', errors)
}

export function validateNativeAuditScenarioReceipt(receipt) {
  const errors = []
  const scenario = NATIVE_AUDIT_SCENARIOS.find((item) => item.id === receipt?.scenarioId)
  if (!scenario || ![FOCUS, PICKER, EXPORT, COHERENCE, AGENT_STUB].includes(receipt?.scenarioId)) {
    return ['unsupported native audit scenario receipt']
  }
  exact(receipt?.schema, 'shellx-cut/full-coverage-results@1', 'scenario receipt schema', errors)
  exact(receipt?.kind, 'native-audit-scenario', 'scenario receipt kind', errors)
  exact(receipt?.status, 'passed', 'scenario receipt status', errors)
  validateIdentity(receipt, errors)
  validateInstalledExecution(receipt, errors)
  validateFullCoverage(receipt, errors)
  for (const error of validateNativeAuditScenarioBinding(scenario, bindingFor(receipt))) errors.push(`registry binding: ${error}`)
  if (receipt?.scenarioId === FOCUS) validateFocus(receipt, errors)
  if (receipt?.scenarioId === PICKER) validatePicker(receipt, errors)
  if (receipt?.scenarioId === EXPORT) validateExport(receipt, errors)
  if (receipt?.scenarioId === COHERENCE) validateCoherence(receipt, errors)
  if (receipt?.scenarioId === AGENT_STUB) validateAgentStub(receipt, errors)
  return errors
}
