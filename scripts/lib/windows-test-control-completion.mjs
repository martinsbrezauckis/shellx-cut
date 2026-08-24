import { createHash } from 'node:crypto'
import { existsSync, lstatSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, isAbsolute, join, resolve } from 'node:path'

import { NATIVE_AUDIT_SCENARIO_IDS } from '../../ui/public-tests/lib/nativeAuditScenarioRegistry.mjs'
import { validateNativeAuditScenarioReceipt } from './native-audit-scenario-receipt-validator.mjs'

export const WINDOWS_TEST_CONTROL_BINDING_SCHEMA = 'shellx-cut/windows-test-control-binding@2'
export const WINDOWS_TEST_CONTROL_COMPLETION_SCHEMA = 'shellx-cut/windows-installed-test-control-completion@2'

const SHA256 = /^[a-f0-9]{64}$/
const GIT = /^[a-f0-9]{40}$/
const COMPUTER_NAME = /^[A-Z][A-Z0-9-]{0,14}$/
const SESSION_ID = /^[1-9][0-9]{0,8}$/
const REGISTERED_WINDOWS_CONTROLS = Object.freeze({
  'grok-windows': { moduleId: 'grok-windows-full-coverage', label: 'GROK/laptop' },
  'workstation-windows': { moduleId: 'workstation-windows-full-coverage', label: 'operator-authorized workstation' },
})
// The control completion is an importer, not an alternate native-scenario
// registry. Keep its required receipt set coupled to the declarative registry.
export const WINDOWS_TEST_CONTROL_NATIVE_AUDIT_SCENARIO_IDS = NATIVE_AUDIT_SCENARIO_IDS

function fail(message) { throw new Error(`Windows test-control receipt: ${message}`) }
function hash(path) { return createHash('sha256').update(readFileSync(path)).digest('hex') }
function read(path, label) {
  let stat
  try { stat = lstatSync(path) } catch { fail(`${label} is missing: ${path}`) }
  if (!stat.isFile() || stat.isSymbolicLink()) fail(`${label} must be a regular file: ${path}`)
  try { return JSON.parse(readFileSync(path, 'utf8')) } catch (error) { fail(`${label} is not JSON: ${error.message}`) }
}
function keys(value, expected, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).sort().join('|') !== [...expected].sort().join('|')) fail(`${label} has unsupported or missing fields`)
}
function safeAbsolute(value, label) {
  if (typeof value !== 'string' || !isAbsolute(value) || resolve(value) !== value) fail(`${label} must be an exact absolute path`)
}
function safeInside(root, path, label) {
  const expected = resolve(root); const actual = resolve(path)
  if (actual === expected || !actual.startsWith(`${expected}/`)) fail(`${label} must stay below the immutable receipt root`)
  return actual
}

export function assertWindowsTestControlMode({ diagnosticSection = '', bindingPath = '' }) {
  const focused = Boolean(String(diagnosticSection).trim())
  const controlled = Boolean(String(bindingPath).trim())
  if (focused && controlled) fail('--test-control-binding is reserved for the unfiltered registered matrix; focused diagnostics must remain non-qualifying')
  if (!focused && !controlled) fail('unfiltered Windows qualification requires --test-control-binding from a registered Windows control panel')
}

export function loadWindowsTestControlAuthority({ bindingPath = '' }) {
  if (!bindingPath) return null
  const resolved = resolve(bindingPath)
  const binding = read(resolved, 'test-control binding')
  keys(binding, ['schema', 'releaseStudioBindingSha256', 'testControlManifestSha256', 'candidate', 'host', 'receiptRoot', 'module'], 'test-control binding')
  if (binding.schema !== WINDOWS_TEST_CONTROL_BINDING_SCHEMA || !SHA256.test(binding.releaseStudioBindingSha256 || '') || !SHA256.test(binding.testControlManifestSha256 || '')) fail('binding schema or immutable hashes are invalid')
  keys(binding.candidate, ['id', 'releaseId', 'source'], 'binding candidate')
  keys(binding.candidate.source, ['checkoutPath', 'branch', 'commit', 'tree', 'contentManifestSha256', 'actionManifestSha256'], 'binding candidate source')
  if (!binding.candidate.id || !GIT.test(binding.candidate.source.commit || '') || !GIT.test(binding.candidate.source.tree || '') || !SHA256.test(binding.candidate.source.contentManifestSha256 || '') || !SHA256.test(binding.candidate.source.actionManifestSha256 || '')) fail('binding candidate identity is invalid')
  keys(binding.host, ['id', 'computerName', 'desktopSession'], 'binding host')
  const registered = REGISTERED_WINDOWS_CONTROLS[binding.host.id]
  if (!registered || !COMPUTER_NAME.test(binding.host.computerName || '') || !SESSION_ID.test(binding.host.desktopSession || '')) fail('binding host is not a registered Windows qualification desktop')
  safeAbsolute(binding.receiptRoot, 'binding receiptRoot')
  keys(binding.module, ['id', 'receiptRoot'], 'binding module')
  if (binding.module.id !== registered.moduleId || binding.module.receiptRoot !== 'windows-installed-full-coverage') fail(`binding module is not the registered ${registered.label} coverage module`)
  return { ...binding, bindingPath: resolved, bindingFileSha256: hash(resolved) }
}

export function assertWindowsTestControlHost({ authority, interactiveSession }) {
  if (!authority) return null
  const actualHost = String(interactiveSession?.hostId || '').toUpperCase()
  const actualSession = String(interactiveSession?.sessionId || '')
  if (actualHost !== authority.host.computerName || actualSession !== authority.host.desktopSession) {
    fail(`registered Windows host mismatch: expected ${authority.host.computerName} session ${authority.host.desktopSession}; observed ${actualHost || 'unknown'} session ${actualSession || 'unknown'}`)
  }
  return authority
}

export function loadWindowsTestControlBinding({ bindingPath = '', authority = null, out }) {
  const binding = authority || loadWindowsTestControlAuthority({ bindingPath })
  if (!binding) return null
  if (hash(binding.bindingPath) !== binding.bindingFileSha256) fail('test-control binding changed after host admission')
  const expectedOut = safeInside(binding.receiptRoot, join(binding.receiptRoot, binding.module.receiptRoot), 'binding module receipt root')
  if (resolve(out) !== expectedOut) fail('--out differs from the immutable test-control module receipt root')
  if (existsSync(expectedOut)) fail('controlled --out already exists; refusing stale or borrowed evidence')
  return { ...binding, out: expectedOut }
}

export function prepareWindowsEvidenceOutput({ bindingPath = '', authority = null, out, finalResumeRequest, assertFinalResumeOutput }) {
  if (typeof assertFinalResumeOutput !== 'function') fail('evidence output preparation requires the registered final-resume assertion')
  assertFinalResumeOutput(finalResumeRequest, out)
  const binding = loadWindowsTestControlBinding({ bindingPath, authority, out })
  if (binding) mkdirSync(dirname(out), { recursive: true })
  mkdirSync(out, binding ? { recursive: false } : { recursive: true })
  return binding
}

function validateSource(source, binding) {
  if (source?.schema !== 'shellx-cut/windows-installed-source@1') fail('source receipt schema is invalid')
  if (source.head !== binding.candidate.source.commit || source.gitTree !== binding.candidate.source.tree || source.contentManifest?.sha256 !== binding.candidate.source.contentManifestSha256 || source.worktree?.id !== binding.candidate.source.checkoutPath) fail('source receipt is borrowed from another test-control candidate')
  if (String(source.interactiveSession?.hostId || '').toUpperCase() !== binding.host.computerName || String(source.interactiveSession?.sessionId || '') !== binding.host.desktopSession) fail('source receipt is borrowed from another Windows host or desktop session')
}
function validateMatrix(matrix, binding) {
  if (matrix?.schema !== 'shellx-cut/full-coverage-results@1' || matrix.ok !== true || matrix.full !== true || matrix.strictAllActions !== true || matrix.surface !== 'windows-installed') fail('matrix receipt is not a strict Windows installed pass')
  if (matrix.source?.gitCommit !== binding.candidate.source.commit || matrix.source?.contentManifestSha256 !== binding.candidate.source.contentManifestSha256 || matrix.runtime?.sourceGitCommit !== binding.candidate.source.commit || matrix.runtime?.sourceContentManifestSha256 !== binding.candidate.source.contentManifestSha256) fail('matrix receipt is borrowed from another test-control candidate')
}
function validateNative(receipt, id, binding) {
  if (receipt?.schema !== 'shellx-cut/full-coverage-results@1' || receipt.kind !== 'native-audit-scenario' || receipt.status !== 'passed' || receipt.scenarioId !== id) fail(`${id} native receipt is invalid`)
  if (receipt.candidate?.sourceCommit !== binding.candidate.source.commit || receipt.candidate?.gitTree !== binding.candidate.source.tree || receipt.candidate?.worktreeId !== binding.candidate.source.checkoutPath || receipt.content?.manifestSha256 !== binding.candidate.source.contentManifestSha256) fail(`${id} native receipt is borrowed from another test-control candidate`)
  const errors = validateNativeAuditScenarioReceipt(receipt)
  if (errors.length) fail(`${id} native receipt does not satisfy the native scenario validator: ${errors.join('; ')}`)
}

export function writeWindowsTestControlCompletionReceipt({ out, binding }) {
  if (!binding) return null
  const sourcePath = join(out, 'source-receipt.json')
  const matrixPath = join(out, 'full-coverage-receipt.json')
  const nativePaths = WINDOWS_TEST_CONTROL_NATIVE_AUDIT_SCENARIO_IDS.map((id) => [
    id,
    join(out, 'native-audit-receipts', `${id}.json`),
  ])
  const source = read(sourcePath, 'source receipt'); validateSource(source, binding)
  const matrix = read(matrixPath, 'full coverage receipt'); validateMatrix(matrix, binding)
  const native = nativePaths.map(([id, path]) => { const receipt = read(path, `${id} native receipt`); validateNative(receipt, id, binding); return { scenarioId: id, file: `native-audit-receipts/${id}.json`, sha256: hash(path) } })
  const receipt = {
    schema: WINDOWS_TEST_CONTROL_COMPLETION_SCHEMA,
    status: 'pass',
    binding: {
      releaseStudioBindingSha256: binding.releaseStudioBindingSha256,
      testControlManifestSha256: binding.testControlManifestSha256,
      candidateId: binding.candidate.id,
      sourceCommit: binding.candidate.source.commit,
      sourceTree: binding.candidate.source.tree,
      contentManifestSha256: binding.candidate.source.contentManifestSha256,
      actionManifestSha256: binding.candidate.source.actionManifestSha256,
      hostId: binding.host.id,
      computerName: binding.host.computerName,
      desktopSession: binding.host.desktopSession,
      receiptRoot: binding.receiptRoot,
      moduleId: binding.module.id,
      moduleReceiptRoot: binding.module.receiptRoot,
    },
    matrix: {
      sourceReceipt: { file: 'source-receipt.json', sha256: hash(sourcePath) },
      fullCoverageReceipt: { file: 'full-coverage-receipt.json', sha256: hash(matrixPath) },
    },
    native,
  }
  const target = join(out, 'test-control-completion.json')
  writeFileSync(target, `${JSON.stringify(receipt, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' })
  return receipt
}
