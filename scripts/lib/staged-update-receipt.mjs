import { existsSync, writeFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'

const SHA256_RX = /^[a-f0-9]{64}$/
const ID_RX = /^[a-z0-9][a-z0-9-]*$/
const ROWS = new Map([
  ['about-check-updates', 'available'],
  ['update-btn', 'available'],
  ['about-install-update', 'declined'],
  ['restart-safe', 'running'],
])

function text(value, label) {
  if (typeof value !== 'string' || !value.trim() || /[\r\n\0]/.test(value)) throw new Error(`${label} must be one line of text`)
  return value.trim()
}

function digest(value, label) {
  const normalized = text(value, label)
  if (!SHA256_RX.test(normalized)) throw new Error(`${label} must be a SHA-256 digest`)
  return normalized
}

function safeArtifact(value, label) {
  const name = text(value?.name, `${label} name`)
  if (name !== basename(name)) throw new Error(`${label} name must not contain a path`)
  if (!Number.isInteger(value?.bytes) || value.bytes < 0) throw new Error(`${label} bytes must be a non-negative integer`)
  return { name, bytes: value.bytes, sha256: digest(value.sha256, `${label} SHA-256`) }
}

function safeEvidence(value) {
  if (!Array.isArray(value)) return []
  return value.map((item) => {
    const kind = text(item?.kind, 'UI evidence kind')
    if (!ID_RX.test(kind)) throw new Error('UI evidence kind must be a path-safe identifier')
    return { kind, sha256: digest(item?.sha256, 'UI evidence SHA-256') }
  })
}

/** Normalize only the native observations the private receipt is allowed to retain. */
export function validateStagedUpdateUiResults(value, { runId, feedUrl, installedVersion, candidateVersion }) {
  if (value?.schema !== 'shellx-cut/staged-update-ui@1' || value.status !== 'pass') {
    throw new Error('UI results must be a passing shellx-cut/staged-update-ui@1 record')
  }
  if (value.runId !== runId || value.feedUrl !== feedUrl || value.installedVersion !== installedVersion
      || value.candidateVersion !== candidateVersion) {
    throw new Error('UI results do not bind this staged update run, feed, and versions')
  }
  if (value.usedJsBridgeFixture !== false || value.downloaded !== false || value.installed !== false
      || value.appRunningAfterDecline !== true) {
    throw new Error('UI results must prove native bridge use, decline, no download/install, and a running app')
  }
  const rows = Array.isArray(value.rows) ? value.rows : []
  if (rows.length !== ROWS.size || new Set(rows.map((row) => row?.id)).size !== ROWS.size) {
    throw new Error('UI results must report every staged update row exactly once')
  }
  const normalizedRows = rows.map((row) => {
    const id = text(row?.id, 'UI row id')
    const expected = ROWS.get(id)
    if (!expected || row?.outcome !== expected || row?.interaction !== 'native') {
      throw new Error(`UI row ${id} does not report its required native outcome`)
    }
    const version = id === 'restart-safe' ? installedVersion : candidateVersion
    if (row.version !== version) throw new Error(`UI row ${id} does not bind the expected version`)
    return { id, outcome: expected, interaction: 'native', version }
  }).sort((left, right) => left.id.localeCompare(right.id))
  return {
    rows: normalizedRows,
    evidence: safeEvidence(value.evidence),
    noJsBridgeFixture: true,
    downloaded: false,
    installed: false,
    appRunningAfterDecline: true,
  }
}

export function buildStagedUpdateReceipt({
  generatedAt = new Date().toISOString(),
  runId,
  rigSource,
  installed,
  candidate,
  tls,
  ui,
  uiResultSha256,
  server,
  cleanup,
}) {
  if (installed.before.sha256 !== installed.after.sha256 || installed.before.bytes !== installed.after.bytes) {
    throw new Error('installed application bytes changed during detection/decline coverage')
  }
  if (server.artifactRequests !== 0) throw new Error('detection/decline coverage fetched an updater artifact')
  if (server.manifestRequests < 1) throw new Error('the installed app never requested the staged update feed')
  const feedUrl = text(candidate.feedUrl, 'feed URL')
  if (!/^https:\/\/127[.]0[.]0[.]1:\d+\/v\d+[.]\d+[.]\d+\/latest[.]json$/.test(feedUrl)) {
    throw new Error('receipt feed URL must remain a version-bound loopback HTTPS URL')
  }
  return {
    schema: 'shellx-cut/staged-update-rig@1',
    generatedAt,
    status: 'pass',
    runId: text(runId, 'run id'),
    rigSource: {
      gitCommit: text(rigSource.gitCommit, 'rig source commit'),
      version: text(rigSource.version, 'rig source version'),
      cargoLockSha256: digest(rigSource.cargoLockSha256, 'rig Cargo.lock SHA-256'),
      contentManifestSha256: digest(rigSource.contentManifestSha256, 'rig content manifest SHA-256'),
    },
    installed: {
      version: text(installed.version, 'installed version'),
      app: safeArtifact(installed.before, 'installed app'),
      unchangedAfterDecline: true,
    },
    candidate: {
      version: text(candidate.version, 'candidate version'),
      platform: text(candidate.platform, 'candidate platform'),
      source: {
        gitCommit: text(candidate.source.gitCommit, 'candidate source commit'),
        cargoLockSha256: digest(candidate.source.cargoLockSha256, 'candidate Cargo.lock SHA-256'),
        contentManifestSha256: digest(candidate.source.contentManifestSha256, 'candidate content manifest SHA-256'),
      },
      manifest: safeArtifact(candidate.manifest, 'candidate manifest'),
      artifact: safeArtifact(candidate.artifact, 'candidate artifact'),
      signature: safeArtifact(candidate.signature, 'candidate signature'),
    },
    feed: {
      url: feedUrl,
      tlsCertificateSha256: digest(tls.certificateSha256, 'TLS certificate SHA-256'),
      tlsCaSha256: digest(tls.caSha256, 'TLS CA SHA-256'),
      manifestRequests: server.manifestRequests,
      artifactRequests: server.artifactRequests,
      rejectedRequests: server.rejectedRequests,
    },
    ui: { ...ui, resultSha256: digest(uiResultSha256, 'UI results SHA-256') },
    cleanup: {
      app: text(cleanup.app, 'app cleanup result'),
      server: text(cleanup.server, 'server cleanup result'),
    },
    checks: [
      'candidate-signature-verified-against-embedded-updater-key',
      'candidate-receipt-manifest-artifact-and-platform-bound',
      'loopback-https-ca-verified-without-insecure-tls-bypass',
      'native-update-available-decline-and-restart-safe-rows',
      'detection-decline-did-not-download-install-or-change-baseline-bytes',
    ],
  }
}

export function writeStagedUpdateReceipt(outDir, receipt) {
  const path = resolve(outDir, 'staged-update-receipt.json')
  if (existsSync(path)) throw new Error('refusing to overwrite staged update receipt')
  writeFileSync(path, `${JSON.stringify(receipt, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' })
  return path
}
