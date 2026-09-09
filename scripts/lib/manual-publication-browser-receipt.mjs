import { createHash } from 'node:crypto'
import { writeFileSync } from 'node:fs'
import { isDeepStrictEqual } from 'node:util'

export const MANUAL_PUBLICATION_BROWSER_RECEIPT_SCHEMA = 'shellx-cut/manual-publication-browser@1'

const SHA256 = /^[a-f0-9]{64}$/
const GIT_COMMIT = /^[a-f0-9]{40}$/
const RUN_ID = /^[A-Za-z0-9][A-Za-z0-9._-]{2,127}$/
const VERSION = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/
const ACTION_IDS = ['manual-feature', 'manual-search']
const MANUAL_SEARCH_FIELDS = ['emptyQuery', 'emptyResultObserved', 'query', 'resultCount', 'featureIds', 'unrelatedFeatureAbsent']
const MANUAL_FEATURE_FIELDS = ['previousFeatureId', 'previousHash', 'previousHighlightCleared', 'previousTargetClosed', 'featureId', 'explanation', 'highlightTarget', 'settingsBodyVisible', 'exportMenuClosed']

function sha256(value) {
  return createHash('sha256').update(value).digest('hex')
}

function required(environment, name, pattern) {
  const value = String(environment[name] || '').trim()
  if (!pattern.test(value)) throw new Error(`Manual browser receipt requires valid ${name}`)
  return value
}

function exactKeys(value, label, keys) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label} must be an object`)
  const actual = Object.keys(value).sort()
  const expected = [...keys].sort()
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new Error(`${label} keys must be exactly ${expected.join(', ')}`)
  }
}

function descriptive(value, label) {
  const outcome = String(value || '').trim()
  if (!outcome || /[\r\n\0]/.test(outcome) || ['pass', 'fail', 'failed', 'error', 'unavailable'].includes(outcome.toLowerCase())) {
    throw new Error(`${label} must be a descriptive visible outcome`)
  }
  return outcome
}

function visibleSearch(value) {
  exactKeys(value, 'Manual search visible effect', MANUAL_SEARCH_FIELDS)
  if (typeof value.emptyQuery !== 'string' || !value.emptyQuery.trim() || typeof value.query !== 'string' || !value.query.trim()
      || value.emptyResultObserved !== true || value.resultCount !== 1 || !Array.isArray(value.featureIds)
      || value.featureIds.length !== 1 || value.featureIds[0] !== 'cut.top.settings' || value.unrelatedFeatureAbsent !== true) {
    throw new Error('Manual search visible effect must prove the Settings-only result')
  }
  return { ...value, emptyQuery: value.emptyQuery.trim(), query: value.query.trim(), featureIds: [...value.featureIds] }
}

function visibleFeature(value) {
  exactKeys(value, 'Manual feature visible effect', MANUAL_FEATURE_FIELDS)
  if (value.previousFeatureId !== 'cut.top.export' || value.previousHash !== '#cut.top.export'
      || value.previousHighlightCleared !== true || value.previousTargetClosed !== true || value.featureId !== 'cut.top.settings'
      || typeof value.explanation !== 'string' || !value.explanation.trim()
      || value.highlightTarget !== '[data-cut-settings-body="overview"]' || value.settingsBodyVisible !== true || value.exportMenuClosed !== true) {
    throw new Error('Manual feature visible effect must prove the Settings transition')
  }
  return { ...value, explanation: value.explanation.trim() }
}

function parsePublicationManifestBytes(publicationManifestBytes) {
  if (typeof publicationManifestBytes !== 'string' && !Buffer.isBuffer(publicationManifestBytes)) {
    throw new Error('Manual publication manifest bytes are required')
  }
  try {
    return JSON.parse(Buffer.isBuffer(publicationManifestBytes)
      ? publicationManifestBytes.toString('utf8')
      : publicationManifestBytes)
  } catch {
    throw new Error('Manual publication manifest bytes must contain valid JSON')
  }
}

export function manualPublicationBrowserBinding(environment = process.env) {
  if (!environment.SHELLX_CUT_MANUAL_BROWSER_RECEIPT) return null
  const version = required(environment, 'FCV_SOURCE_VERSION', VERSION)
  return {
    outputPath: String(environment.SHELLX_CUT_MANUAL_BROWSER_RECEIPT),
    runId: required(environment, 'SHELLX_CUT_MANUAL_BROWSER_RUN_ID', RUN_ID),
    candidate: {
      source: {
        gitCommit: required(environment, 'FCV_SOURCE_GIT_COMMIT', GIT_COMMIT),
        gitTree: required(environment, 'FCV_SOURCE_GIT_TREE', GIT_COMMIT),
        contentManifestSha256: required(environment, 'FCV_SOURCE_CONTENT_MANIFEST_SHA256', SHA256),
        actionManifestSha256: required(environment, 'FCV_ACTION_MANIFEST_SHA256', SHA256),
        version,
      },
      version,
    },
    packageSha256: required(environment, 'FCV_PACKAGE_SHA256', SHA256),
    platform: String(environment.SHELLX_CUT_MANUAL_BROWSER_PLATFORM || 'unknown').trim() || 'unknown',
  }
}

export function buildManualPublicationBrowserReceipt({ binding, publicationManifest, publicationManifestBytes, rows, generatedAt = new Date().toISOString() }) {
  if (!binding) throw new Error('Manual browser receipt binding is required')
  const parsedPublicationManifest = parsePublicationManifestBytes(publicationManifestBytes)
  if (!isDeepStrictEqual(parsedPublicationManifest, publicationManifest)) {
    throw new Error('Manual publication manifest bytes differ from the supplied manifest')
  }
  exactKeys(publicationManifest, 'Manual publication manifest', [
    'architecture', 'artifacts', 'authority', 'embedUrl', 'excludedLegacyInputs',
    'identity', 'legacyScreenshotHotspotAuthority', 'readOnly', 'relativeAssetBase',
    'route', 'schema', 'source',
  ])
  if (publicationManifest.schema !== 'shellx-cut/manual-publication@1') throw new Error('Manual publication manifest schema is invalid')
  if (publicationManifest.source?.gitHead !== binding.candidate.source.gitCommit) {
    throw new Error('Manual publication source Git commit differs from receipt candidate')
  }
  if (binding.candidate.source.version !== binding.candidate.version) {
    throw new Error('Manual browser receipt candidate versions differ')
  }
  if (publicationManifest.identity?.artifactClosureSha256 === undefined || !SHA256.test(publicationManifest.identity.artifactClosureSha256)) {
    throw new Error('Manual publication artifact closure hash is invalid')
  }
  if (!Array.isArray(rows) || rows.length !== ACTION_IDS.length) throw new Error('Manual browser receipt must contain both delegated action rows')
  const normalizedRows = rows.map((row) => {
    exactKeys(row, `Manual browser row ${row?.id || 'unknown'}`, ['id', 'interaction', 'outcome', 'visible'])
    if (!ACTION_IDS.includes(row.id) || row.interaction !== 'native') {
      throw new Error('Manual browser receipt action row is invalid')
    }
    const visible = row.id === 'manual-search' ? visibleSearch(row.visible) : visibleFeature(row.visible)
    return { id: row.id, interaction: 'native', outcome: descriptive(row.outcome, `Manual browser row ${row.id} outcome`), visible }
  }).sort((left, right) => left.id.localeCompare(right.id))
  if (normalizedRows.some((row, index) => row.id !== ACTION_IDS[index])) throw new Error('Manual browser receipt action rows must be unique and sorted')
  return {
    schema: MANUAL_PUBLICATION_BROWSER_RECEIPT_SCHEMA,
    status: 'pass',
    runId: binding.runId,
    generatedAt,
    candidate: binding.candidate,
    artifact: {
      packageSha256: binding.packageSha256,
      manualClosureSha256: publicationManifest.identity.artifactClosureSha256,
      version: binding.candidate.version,
    },
    ui: {
      platform: binding.platform,
      browser: 'chromium',
      rows: normalizedRows,
    },
    outputSha256: sha256(publicationManifestBytes),
  }
}

export function writeManualPublicationBrowserReceipt(path, receipt) {
  if (!path || typeof path !== 'string') throw new Error('Manual browser receipt output path is required')
  if (receipt?.schema !== MANUAL_PUBLICATION_BROWSER_RECEIPT_SCHEMA || receipt.status !== 'pass') {
    throw new Error('Manual browser receipt is not valid for writing')
  }
  writeFileSync(path, `${JSON.stringify(receipt, null, 2)}\n`, { encoding: 'utf8', flag: 'wx', mode: 0o600 })
  return sha256(JSON.stringify(receipt, null, 2) + '\n')
}
