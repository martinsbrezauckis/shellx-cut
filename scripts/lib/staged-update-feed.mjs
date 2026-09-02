import { createHash } from 'node:crypto'
import { existsSync, readFileSync, statSync } from 'node:fs'
import { basename, resolve } from 'node:path'

const VERSION_RX = /^\d+\.\d+\.\d+$/
const SHA256_RX = /^[a-f0-9]{64}$/
const RECEIPT_SCHEMA = 'shellx-cut/updater-manifest-verify@1'

export function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

function requireFile(path, label) {
  const resolved = resolve(path || '')
  if (!path || !existsSync(resolved) || !statSync(resolved).isFile() || statSync(resolved).size <= 0) {
    throw new Error(`${label} must be a non-empty regular file`)
  }
  return resolved
}

function readJson(path, label) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'))
  } catch (error) {
    throw new Error(`${label} is not valid JSON: ${error.message}`)
  }
}

export function parseReleaseVersion(value, label = 'version') {
  if (typeof value !== 'string' || !VERSION_RX.test(value)) {
    throw new Error(`${label} must use major.minor.patch`)
  }
  return value.split('.').map(Number)
}

export function isHigherVersion(candidate, installed) {
  const left = parseReleaseVersion(candidate, 'candidate version')
  const right = parseReleaseVersion(installed, 'installed version')
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] > right[index]) return true
    if (left[index] < right[index]) return false
  }
  return false
}

function requiredReceiptCheck(receipt, value) {
  return Array.isArray(receipt.checks) && receipt.checks.includes(value)
}

function requireVersionBoundLoopbackUrl(value, version, port) {
  let url
  try {
    url = new URL(value)
  } catch {
    throw new Error('candidate platform URL is not a valid URL')
  }
  if (url.protocol !== 'https:' || url.hostname !== '127.0.0.1' || Number(url.port) !== port) {
    throw new Error('candidate platform URL must use the configured 127.0.0.1 HTTPS feed')
  }
  if (!url.pathname.startsWith(`/v${version}/`) || url.search || url.hash) {
    throw new Error(`candidate platform URL must be bound to /v${version}/ without query or fragment`)
  }
  return url
}

function receiptSource(receipt, version) {
  const source = receipt.source
  if (source?.version !== version || !/^[a-f0-9]{40}$/.test(String(source?.gitCommit || ''))
      || !SHA256_RX.test(String(source?.cargoLockSha256 || ''))
      || !SHA256_RX.test(String(source?.contentManifestSha256 || ''))) {
    throw new Error('candidate receipt source identity does not bind the candidate version and hashes')
  }
  return {
    gitCommit: source.gitCommit,
    cargoLockSha256: source.cargoLockSha256,
    contentManifestSha256: source.contentManifestSha256,
  }
}

function verifyReceipt({ receipt, manifestPath, manifest, artifactPath, signaturePath, platform, artifactUrl }) {
  if (receipt?.schema !== RECEIPT_SCHEMA || receipt.status !== 'pass') {
    throw new Error('candidate receipt must be a passing updater-manifest verification receipt')
  }
  if (receipt.release?.version !== manifest.version || receipt.release?.tag !== `v${manifest.version}`) {
    throw new Error('candidate receipt release version does not match latest.json')
  }
  const source = receiptSource(receipt, manifest.version)
  const manifestHash = sha256File(manifestPath)
  if (receipt.manifest?.sha256 !== manifestHash || receipt.manifest?.bytes !== statSync(manifestPath).size) {
    throw new Error('candidate receipt does not bind the supplied latest.json bytes')
  }
  if (!Array.isArray(receipt.manifest?.platforms) || !receipt.manifest.platforms.includes(platform)) {
    throw new Error(`candidate receipt does not include target platform ${platform}`)
  }
  const records = Array.isArray(receipt.artifacts)
    ? receipt.artifacts.filter((item) => item?.platform === platform)
    : []
  if (records.length !== 1) throw new Error(`candidate receipt must bind exactly one ${platform} artifact`)
  const record = records[0]
  if (record.signatureVerified !== true || record.sha256 !== sha256File(artifactPath)
      || record.signatureSha256 !== sha256File(signaturePath)
      || record.name !== basename(artifactPath) || record.signatureName !== basename(signaturePath)
      || record.url !== artifactUrl) {
    throw new Error('candidate receipt artifact, signature, or platform binding does not match supplied bytes')
  }
  for (const check of [
    'artifact-minisign-verified-against-embedded-pubkey',
    'all-required-platforms-present',
    'release-url-version-bound',
  ]) {
    if (!requiredReceiptCheck(receipt, check)) throw new Error(`candidate receipt is missing check ${check}`)
  }
  return source
}

/**
 * Validate exactly the bytes a local staged updater server may expose. The
 * caller injects the product's minisign verifier; a manifest receipt alone is
 * never accepted as a substitute for fresh cryptographic verification.
 */
export function preflightStagedUpdateFeed({
  manifestPath,
  receiptPath,
  artifactPath,
  signaturePath,
  platform,
  installedVersion,
  port,
  verifySignature,
}) {
  if (!Number.isInteger(port) || port < 1 || port > 65_535) throw new Error('port must be an integer from 1 to 65535')
  if (!['windows-x86_64', 'darwin-aarch64'].includes(platform)) throw new Error('target platform is not a supported updater platform')
  if (typeof verifySignature !== 'function') throw new Error('staged update preflight requires cryptographic signature verification')
  const paths = {
    manifest: requireFile(manifestPath, 'candidate latest.json'),
    receipt: requireFile(receiptPath, 'candidate updater receipt'),
    artifact: requireFile(artifactPath, 'candidate updater artifact'),
    signature: requireFile(signaturePath, 'candidate updater signature'),
  }
  const manifest = readJson(paths.manifest, 'candidate latest.json')
  const version = typeof manifest.version === 'string' ? manifest.version : ''
  parseReleaseVersion(version, 'candidate version')
  if (!isHigherVersion(version, installedVersion)) {
    throw new Error(`candidate version ${version} must be higher than installed version ${installedVersion}`)
  }
  const entry = manifest.platforms?.[platform]
  if (!entry || typeof entry.url !== 'string' || typeof entry.signature !== 'string' || !entry.signature.trim()) {
    throw new Error(`candidate latest.json has no signed ${platform} platform entry`)
  }
  const entries = Object.entries(manifest.platforms || {})
  if (!entries.length) throw new Error('candidate latest.json has no platform entries')
  for (const [entryPlatform, candidate] of entries) {
    if (!['windows-x86_64', 'darwin-aarch64'].includes(entryPlatform)
        || typeof candidate?.url !== 'string' || typeof candidate?.signature !== 'string' || !candidate.signature.trim()) {
      throw new Error(`candidate latest.json has an invalid platform entry ${entryPlatform}`)
    }
    requireVersionBoundLoopbackUrl(candidate.url, version, port)
  }
  const artifactUrl = requireVersionBoundLoopbackUrl(entry.url, version, port)
  if (decodeURIComponent(artifactUrl.pathname.split('/').at(-1) || '') !== basename(paths.artifact)) {
    throw new Error('candidate artifact filename does not match its manifest URL')
  }
  if (readFileSync(paths.signature, 'utf8').trim() !== entry.signature.trim()) {
    throw new Error('candidate signature file does not match latest.json')
  }
  try {
    verifySignature(paths.artifact, paths.signature)
  } catch (error) {
    throw new Error(`candidate artifact signature verification failed: ${error.message || String(error)}`)
  }
  const receipt = readJson(paths.receipt, 'candidate updater receipt')
  const source = verifyReceipt({
    receipt,
    manifestPath: paths.manifest,
    manifest,
    artifactPath: paths.artifact,
    signaturePath: paths.signature,
    platform,
    artifactUrl: artifactUrl.toString(),
  })
  return {
    feedUrl: `https://127.0.0.1:${port}/v${version}/latest.json`,
    version,
    platform,
    paths,
    source,
    manifest: { name: basename(paths.manifest), bytes: statSync(paths.manifest).size, sha256: sha256File(paths.manifest) },
    artifact: { name: basename(paths.artifact), bytes: statSync(paths.artifact).size, sha256: sha256File(paths.artifact) },
    signature: { name: basename(paths.signature), bytes: statSync(paths.signature).size, sha256: sha256File(paths.signature) },
  }
}
