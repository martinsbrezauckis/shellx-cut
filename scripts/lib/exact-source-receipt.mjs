import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path'

import { artifactInfo, collectSourceIdentity, loadIgnoredTestManifest } from './ignored-test-rig.mjs'
import { installedWalkthroughClaim, realFileDropClaim } from './installed-evidence.mjs'
import { sourceContentManifest } from './source-content-manifest.mjs'

export const EXACT_SOURCE_RECEIPT_SCHEMA = 'shellx-cut/exact-source-rig@1'
const SURFACES = new Set(['linux-control', 'windows-installed', 'macos-installed'])
const ID_RX = /^[a-z0-9][a-z0-9-]*$/
const COMMIT_RX = /^[a-f0-9]{40}$/
const SHA256_RX = /^[a-f0-9]{64}$/

function oneLine(value) {
  return String(value).replace(/[\r\n\t\0]/g, ' ')
}

function parseNamedPath(value, flag) {
  const index = String(value).indexOf('=')
  const name = index > 0 ? value.slice(0, index).trim() : ''
  const path = index > 0 ? value.slice(index + 1).trim() : ''
  if (!name || !path) throw new Error(`${flag} requires name=path`)
  return { name, path }
}

export function parseExactSourceArgs(argv) {
  const out = {
    surface: '',
    capabilities: [],
    artifacts: [],
    evidence: [],
    outPath: '',
    help: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--surface') out.surface = argv[++index] ?? ''
    else if (arg === '--capability') out.capabilities.push(argv[++index] ?? '')
    else if (arg === '--artifact') out.artifacts.push({ ...parseNamedPath(argv[++index] ?? '', arg), tree: false })
    else if (arg === '--artifact-tree') out.artifacts.push({ ...parseNamedPath(argv[++index] ?? '', arg), tree: true })
    else if (arg === '--evidence') out.evidence.push(parseNamedPath(argv[++index] ?? '', arg))
    else if (arg === '--out') out.outPath = argv[++index] ?? ''
    else if (arg === '--help' || arg === '-h') out.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return out
}

export function evidencePassed(value) {
  if (typeof value?.status === 'string') return value.status === 'pass'
  if (typeof value?.ok === 'boolean') return value.ok
  if (typeof value?.pass === 'boolean') return value.pass
  return Number.isInteger(value?.summary?.fail) && value.summary.fail === 0
    && Number.isInteger(value.summary.total) && value.summary.total > 0
}

function digestItem(repoRoot, spec, kind) {
  const path = resolve(repoRoot, spec.path)
  const info = artifactInfo(path, { tree: spec.tree === true })
  if (!info.exists) throw new Error(`${kind} '${spec.name}' does not exist`)
  if (!info.sha256) {
    throw new Error(`${kind} '${spec.name}' must be a file or use --artifact-tree for a directory`)
  }
  return {
    name: spec.name,
    kind: info.kind,
    bytes: info.bytes,
    ...(info.files == null ? {} : { files: info.files }),
    sha256: info.sha256,
  }
}

function requireUniqueNames(specs, label) {
  const names = specs.map((spec) => spec.name)
  if (names.some((name) => !ID_RX.test(name))) {
    throw new Error(`${label} names must use lowercase letters, digits, and hyphens`)
  }
  if (new Set(names).size !== names.length) throw new Error(`${label} names must be unique`)
}

function validReceiptArtifact(value) {
  return value?.exists === true
    && SHA256_RX.test(String(value.sha256 || ''))
    && Number.isInteger(value.bytes)
    && value.bytes >= 0
}

function ignoredTestRigClaim(parsed, manifest, source, evidenceName) {
  const rig = manifest.tests.find((entry) => entry.id === parsed.id)
  const errors = []
  if (!rig) errors.push(`unknown rig id '${parsed.id || ''}'`)
  if (parsed.pass !== true) errors.push('pass must be true')
  if (rig && parsed.rustTest !== rig.rustTest) errors.push('rust test does not match the manifest')
  if (rig && parsed.classification !== rig.classification) errors.push('classification does not match the manifest')
  if (rig && JSON.stringify(parsed.command) !== JSON.stringify(rig.command)) errors.push('command does not match the manifest')
  if (parsed.source?.gitDirty !== false) errors.push('source must be clean')
  if (parsed.source?.gitCommit !== source.gitCommit) errors.push('source commit does not match')
  if (parsed.source?.version !== source.version) errors.push('source version does not match')
  if (parsed.source?.cargoLock?.sha256 !== source.cargoLock?.sha256) errors.push('Cargo.lock hash does not match')
  if (parsed.preflight?.platformAllowed !== true) errors.push('platform preflight did not pass')
  if (!Array.isArray(parsed.preflight?.missing) || parsed.preflight.missing.length !== 0) errors.push('preflight has missing requirements')
  if (rig && !rig.platforms.includes(parsed.host?.platform)) errors.push('host platform is not allowed for the rig')
  if (parsed.result?.status !== 0 || parsed.result?.signal != null || parsed.result?.error != null) errors.push('test process did not exit cleanly')
  if (!validReceiptArtifact(parsed.artifacts?.testBinary)) errors.push('compiled test binary is not digest-bound')
  if (!validReceiptArtifact(parsed.artifacts?.stdout) || !validReceiptArtifact(parsed.artifacts?.stderr)) errors.push('test logs are not digest-bound')
  if (rig && (!Array.isArray(parsed.artifacts?.inputs) || parsed.artifacts.inputs.length !== rig.inputArtifacts.length
      || parsed.artifacts.inputs.some((item) => !validReceiptArtifact(item)))) errors.push('input artifacts do not match the manifest')
  if (rig && (!Array.isArray(parsed.artifacts?.outputs) || parsed.artifacts.outputs.length !== rig.outputArtifacts.length
      || parsed.artifacts.outputs.some((item) => !validReceiptArtifact(item)))) errors.push('output artifacts do not match the manifest')
  if (errors.length) throw new Error(`ignored-test evidence '${evidenceName}' is invalid: ${errors.join('; ')}`)
  return {
    id: parsed.id,
    rustTest: parsed.rustTest,
    classification: parsed.classification,
    platform: parsed.host.platform,
    gitCommit: parsed.source.gitCommit,
    version: parsed.source.version,
    cargoLockSha256: parsed.source.cargoLock.sha256,
    testBinarySha256: parsed.artifacts.testBinary.sha256,
  }
}

function updaterManifestClaim(parsed, source, sourceContent, artifacts, evidenceName) {
  if (parsed?.schema !== 'shellx-cut/updater-manifest-verify@1') return null
  const errors = []
  const requiredPlatforms = ['darwin-aarch64', 'windows-x86_64']
  if (parsed.status !== 'pass') errors.push('status must be pass')
  if (parsed.source?.gitDirty !== false) errors.push('source must be clean')
  if (parsed.source?.gitCommit !== source.gitCommit) errors.push('source commit does not match')
  if (parsed.source?.version !== source.version) errors.push('source version does not match')
  if (parsed.source?.cargoLockSha256 !== source.cargoLock?.sha256) errors.push('Cargo.lock hash does not match')
  if (parsed.source?.contentManifestSha256 !== sourceContent.sha256) errors.push('source content hash does not match')
  if (parsed.release?.repository !== 'martinsbrezauckis/shellx-cut') errors.push('release repository does not match')
  if (parsed.release?.version !== source.version || parsed.release?.tag !== `v${source.version}`) {
    errors.push('release version/tag does not match source')
  }
  if (!validReceiptArtifact({ ...parsed.manifest, exists: true })
      || !artifacts.some((item) => item.sha256 === parsed.manifest?.sha256)) {
    errors.push('latest.json digest is not bound as an exact-source artifact')
  }
  if (JSON.stringify(parsed.manifest?.platforms) !== JSON.stringify(requiredPlatforms)) {
    errors.push('manifest must contain exactly the two required updater platforms')
  }
  const verified = Array.isArray(parsed.artifacts) ? parsed.artifacts : []
  if (verified.length !== requiredPlatforms.length) errors.push('verified updater artifact count does not match')
  for (const platform of requiredPlatforms) {
    const matches = verified.filter((item) => item?.platform === platform)
    if (matches.length !== 1) {
      errors.push(`requires exactly one '${platform}' updater artifact`)
      continue
    }
    const item = matches[0]
    if (item.signatureVerified !== true) errors.push(`${platform} signature is not verified`)
    if (!SHA256_RX.test(String(item.sha256 || ''))
        || !artifacts.some((artifact) => artifact.sha256 === item.sha256)) {
      errors.push(`${platform} artifact digest is not exact-source bound`)
    }
    if (!SHA256_RX.test(String(item.signatureSha256 || ''))
        || !artifacts.some((artifact) => artifact.sha256 === item.signatureSha256)) {
      errors.push(`${platform} signature digest is not exact-source bound`)
    }
    if (!String(item.url || '').includes(`/releases/download/v${source.version}/`)) {
      errors.push(`${platform} URL is not bound to the source version`)
    }
  }
  const requiredChecks = [
    'artifact-minisign-verified-against-embedded-pubkey',
    'all-required-platforms-present',
    'release-url-version-bound',
  ]
  if (JSON.stringify(parsed.checks) !== JSON.stringify(requiredChecks)) {
    errors.push('updater verification checks are incomplete')
  }
  if (errors.length) throw new Error(`updater evidence '${evidenceName}' is invalid: ${errors.join('; ')}`)
  return {
    gitCommit: parsed.source.gitCommit,
    version: parsed.source.version,
    contentManifestSha256: parsed.source.contentManifestSha256,
    manifestSha256: parsed.manifest.sha256,
    platforms: requiredPlatforms,
    artifactSha256: Object.fromEntries(verified.map((item) => [item.platform, item.sha256])),
  }
}

function evidenceItem(repoRoot, spec, source, sourceContent, surface, artifacts, ignoredManifest) {
  const path = resolve(repoRoot, spec.path)
  let parsed
  try {
    parsed = JSON.parse(readFileSync(path, 'utf8'))
  } catch (error) {
    throw new Error(`evidence '${spec.name}' is not readable JSON: ${oneLine(error.message)}`)
  }
  if (!evidencePassed(parsed)) throw new Error(`evidence '${spec.name}' is not a passing receipt`)
  const info = artifactInfo(path)
  const item = {
    name: spec.name,
    schema: typeof parsed.schema === 'string'
      ? parsed.schema
      : typeof parsed.schemaVersion === 'string' ? parsed.schemaVersion : null,
    bytes: info.bytes,
    sha256: info.sha256,
  }
  if (parsed.schema === ignoredManifest.receiptSchema) {
    item.ignoredTestRig = ignoredTestRigClaim(parsed, ignoredManifest, source, spec.name)
  }
  const updaterManifest = updaterManifestClaim(parsed, source, sourceContent, artifacts, spec.name)
  if (updaterManifest) item.updaterManifest = updaterManifest
  const realFileDrop = realFileDropClaim(parsed, {
    source,
    surface,
    artifacts,
    evidenceName: spec.name,
  })
  if (realFileDrop) item.realFileDrop = realFileDrop
  const installedWalkthrough = installedWalkthroughClaim(parsed, {
    source,
    sourceContentManifestSha256: sourceContent.sha256,
    surface,
    artifacts,
    evidenceName: spec.name,
  })
  if (installedWalkthrough) item.installedWalkthrough = installedWalkthrough
  if (parsed.schema === 'shellx-cut/full-coverage-results@1') {
    const sourceActions = parsed.sourceActionManifest
    const runtimeActions = parsed.runtimeSourceActionManifest
    const actionInventoryMatches = sourceActions?.matchesExpected === true
      && runtimeActions?.matchesExpected === true
      && sourceActions.sha256 === sourceActions.expectedSha256
      && runtimeActions.sha256 === runtimeActions.expectedSha256
      && runtimeActions.sha256 === sourceActions.sha256
    const strictlyVerified = parsed.ok === true
      && parsed.strictAllActions === true
      && parsed.summary?.controls?.strictUnverified === 0
      && parsed.summary?.controls?.failures === 0
    item.fullUiActionMatrix = {
      strictAllActions: parsed.strictAllActions === true,
      surface: typeof parsed.surface === 'string' ? parsed.surface : null,
      installedApp: parsed.runtime?.installedApp === true,
      driver: typeof parsed.runtime?.driver === 'string' ? parsed.runtime.driver : null,
      nativeAttached: parsed.runtime?.nativeAttached === true,
      nativeProvider: typeof parsed.runtime?.nativeProvider === 'string'
        ? parsed.runtime.nativeProvider
        : null,
      sourceContentManifestSha256: typeof parsed.runtime?.sourceContentManifestSha256 === 'string'
        ? parsed.runtime.sourceContentManifestSha256
        : null,
      actionManifestSha256: typeof runtimeActions?.sha256 === 'string'
        ? runtimeActions.sha256
        : null,
      expectedActionManifestSha256: typeof sourceActions?.expectedSha256 === 'string'
        ? sourceActions.expectedSha256
        : null,
      actionManifestMatchesExpected: actionInventoryMatches,
      total: Number.isInteger(runtimeActions?.total) ? runtimeActions.total : null,
      // Probe rows may intentionally exercise the same source action in more
      // than one state or surface. Duplicate safety belongs to the canonical
      // source-action identity list, not those richer evidence rows.
      duplicateCount: Array.isArray(runtimeActions?.observed)
        ? runtimeActions.observed.length - new Set(runtimeActions.observed).size
        : null,
      fullyVerified: strictlyVerified && Number.isInteger(runtimeActions?.total)
        ? runtimeActions.total : null,
      strictUnverified: Number.isInteger(parsed.summary?.controls?.strictUnverified)
        ? parsed.summary.controls.strictUnverified
        : null,
      failures: Number.isInteger(parsed.summary?.controls?.failures)
        ? parsed.summary.controls.failures
        : null,
    }
  }
  return item
}

function receiptOutputPath(repoRoot, requested) {
  if (!requested) throw new Error('--out is required')
  const outPath = resolve(repoRoot, requested)
  const rel = relative(repoRoot, outPath)
  const insideRepo = rel === '' || (!rel.startsWith(`..${sep}`) && rel !== '..' && !isAbsolute(rel))
  if (insideRepo && rel !== '.shellx-scratch' && !rel.startsWith(`.shellx-scratch${sep}`)) {
    throw new Error('exact-source receipts must stay outside the product repo or under ignored .shellx-scratch')
  }
  if (existsSync(outPath)) throw new Error(`receipt already exists: ${outPath}`)
  return outPath
}

function normalizedSource(identity) {
  return {
    version: identity.version,
    gitCommit: identity.gitCommit,
    gitDirty: identity.gitDirty,
    cargoLockSha256: identity.cargoLock?.sha256,
  }
}

export function createExactSourceReceipt(options) {
  const repoRoot = resolve(options.repoRoot)
  const surface = String(options.surface || '').trim()
  const capabilities = [...new Set((options.capabilities || []).map((item) => String(item).trim()).filter(Boolean))]
  if (!SURFACES.has(surface)) throw new Error('--surface must be linux-control, windows-installed, or macos-installed')
  if (capabilities.length === 0) throw new Error('at least one --capability is required')
  if (capabilities.some((capability) => !ID_RX.test(capability))) {
    throw new Error('capability ids must use lowercase letters, digits, and hyphens')
  }
  if (!options.artifacts?.length) throw new Error('at least one --artifact or --artifact-tree is required')
  if (!options.evidence?.length) throw new Error('at least one --evidence is required')
  requireUniqueNames(options.artifacts, 'artifact')
  requireUniqueNames(options.evidence, 'evidence')
  const outPath = receiptOutputPath(repoRoot, options.outPath)

  const before = collectSourceIdentity(repoRoot)
  const beforeContent = sourceContentManifest(repoRoot)
  if (before.gitDirty) throw new Error('exact-source receipts require a clean product worktree')
  if (!COMMIT_RX.test(before.gitCommit || '')
      || !/^\d+\.\d+\.\d+/.test(before.version || '')
      || !SHA256_RX.test(before.cargoLock?.sha256 || '')) {
    throw new Error('product source identity is incomplete')
  }
  const ignoredManifest = loadIgnoredTestManifest(repoRoot)
  const artifacts = options.artifacts.map((spec) => digestItem(repoRoot, spec, 'artifact'))
  const evidence = options.evidence.map((spec) =>
    evidenceItem(repoRoot, spec, before, beforeContent, surface, artifacts, ignoredManifest))
  const ignoredRigIds = new Set(ignoredManifest.tests.map((rig) => rig.id))
  for (const capability of capabilities.filter((item) => ignoredRigIds.has(item))) {
    const matches = evidence.filter((item) => item.ignoredTestRig?.id === capability)
    if (matches.length !== 1) {
      throw new Error(`ignored-test capability '${capability}' requires exactly one matching rig receipt`)
    }
  }
  for (const capability of capabilities.filter((item) =>
    item === 'file-manager-drop-image' || item === 'file-manager-drop-video')) {
    const kind = capability.endsWith('-image') ? 'image' : 'video'
    const matches = evidence.filter((item) => item.realFileDrop?.cases.includes(kind))
    if (matches.length !== 1) {
      throw new Error(`capability '${capability}' requires exactly one matching real file-drop receipt`)
    }
  }
  if (capabilities.includes('full-ui-action-matrix')) {
    const matches = evidence.filter((item) =>
      item.fullUiActionMatrix?.sourceContentManifestSha256 === beforeContent.sha256)
    if (matches.length !== 1) {
      throw new Error('full-ui-action-matrix requires exactly one source-content-matched receipt')
    }
  }
  if (capabilities.includes('updater')) {
    const matches = evidence.filter((item) => item.updaterManifest)
    if (matches.length !== 1) {
      throw new Error("capability 'updater' requires exactly one matching updater verification receipt")
    }
  }
  for (const capability of capabilities.filter((item) =>
    item === 'installed-agent-docs' || item === 'settings-library-debug-mcp-walkthrough')) {
    const requiredRows = capability === 'installed-agent-docs'
      ? ['installed-agent-docs']
      : ['settings', 'library', 'about', 'debug-api', 'mcp-self-test']
    const matches = evidence.filter((item) =>
      requiredRows.every((row) => item.installedWalkthrough?.rows.includes(row)))
    if (matches.length !== 1) {
      throw new Error(`capability '${capability}' requires exactly one matching installed walkthrough receipt`)
    }
  }
  const after = collectSourceIdentity(repoRoot)
  const afterContent = sourceContentManifest(repoRoot)
  if (after.gitDirty
      || after.gitCommit !== before.gitCommit
      || after.version !== before.version
      || after.cargoLock?.sha256 !== before.cargoLock?.sha256
      || afterContent.sha256 !== beforeContent.sha256) {
    throw new Error('source identity changed while the exact-source receipt was assembled')
  }

  const receipt = {
    schema: EXACT_SOURCE_RECEIPT_SCHEMA,
    surface,
    generatedAt: options.generatedAt || new Date().toISOString(),
    status: 'pass',
    source: normalizedSource(after),
    host: { platform: process.platform, arch: process.arch },
    capabilities,
    artifacts,
    evidence,
  }
  receipt.source.contentManifestSha256 = afterContent.sha256
  receipt.source.contentManifestFiles = afterContent.files
  mkdirSync(dirname(outPath), { recursive: true })
  writeFileSync(outPath, `${JSON.stringify(receipt, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' })
  return { receipt, outPath }
}
