import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'

export const WINDOWS_SIGNED_ARTIFACT_RECEIPT_SCHEMA = 'shellx-cut/windows-signed-artifact@1'

const SHA256 = /^[a-f0-9]{64}$/
const COMMIT = /^[a-f0-9]{40}$/

function invariant(condition, message) {
  if (!condition) throw new Error(message)
}

export function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

function validateArtifact(item, label) {
  invariant(item && SHA256.test(String(item.sha256 || '')), `${label} hash is invalid`)
  invariant(item.signatureStatus === 'Valid', `${label} Authenticode status is not Valid`)
}

function validateSource(source) {
  invariant(COMMIT.test(String(source?.gitCommit || '')), 'artifact receipt source commit is invalid')
  invariant(COMMIT.test(String(source?.gitTree || '')), 'artifact receipt source tree is invalid')
  invariant(SHA256.test(String(source?.contentManifestSha256 || '')), 'artifact receipt source content digest is invalid')
  invariant(/^\d+\.\d+\.\d+/.test(String(source?.version || '')), 'artifact receipt source version is invalid')
}

export function validateWindowsSignedArtifactReceipt(receipt, {
  installerPath = '',
  source = null,
  installedArtifact = null,
} = {}) {
  invariant(receipt?.schema === WINDOWS_SIGNED_ARTIFACT_RECEIPT_SCHEMA, 'invalid Windows signed artifact receipt schema')
  invariant(receipt.status === 'pass', 'Windows signed artifact receipt is not a pass')
  validateSource(receipt.source)
  validateArtifact(receipt.installer, 'installer')
  validateArtifact(receipt.packaged?.shell, 'packaged shell')
  validateArtifact(receipt.packaged?.cutd, 'packaged cutd')
  validateArtifact(receipt.standalone?.shell, 'standalone shell')
  validateArtifact(receipt.standalone?.cutd, 'standalone cutd')
  invariant(receipt.installer.name === basename(receipt.installer.name || ''), 'artifact receipt installer name is unsafe')

  if (source) {
    for (const field of ['gitCommit', 'gitTree', 'contentManifestSha256', 'version']) {
      invariant(receipt.source[field] === source[field], `artifact receipt source ${field} differs from the frozen candidate`)
    }
  }
  if (installerPath) {
    const resolved = resolve(installerPath)
    invariant(basename(resolved) === receipt.installer.name, 'artifact receipt installer name differs from --installer')
    invariant(sha256File(resolved) === receipt.installer.sha256, 'artifact receipt installer hash differs from --installer')
  }
  if (installedArtifact) {
    for (const name of ['shell', 'cutd']) {
      const installed = installedArtifact[name]
      invariant(installed?.signatureStatus === 'Valid' && SHA256.test(String(installed?.sha256 || '')),
        `installed ${name} identity is incomplete`)
      invariant(installed.sha256 === receipt.packaged[name].sha256,
        `installed ${name} hash differs from the signed packaged artifact receipt`)
    }
  }
  return receipt
}

export function readWindowsSignedArtifactReceipt(path, options = {}) {
  const resolved = resolve(path)
  let receipt
  try {
    receipt = JSON.parse(readFileSync(resolved, 'utf8'))
  } catch (error) {
    throw new Error(`could not read Windows signed artifact receipt ${resolved}: ${error.message}`)
  }
  return validateWindowsSignedArtifactReceipt(receipt, options)
}

export function readWindowsSigningEvents(path) {
  const rows = readFileSync(resolve(path), 'utf8').split(/\r?\n/).filter(Boolean).map((line, index) => {
    let row
    try {
      row = JSON.parse(line)
    } catch {
      throw new Error(`Windows signing event ${index + 1} is not JSON`)
    }
    invariant(typeof row.artifactPath === 'string' && row.artifactPath, `Windows signing event ${index + 1} has no artifact path`)
    validateArtifact(row, `Windows signing event ${index + 1}`)
    return row
  })
  invariant(rows.length > 0, 'Windows signing event log is empty')
  return rows
}

function latestIndex(rows, predicate, label) {
  for (let index = rows.length - 1; index >= 0; index -= 1) {
    if (predicate(rows[index])) return index
  }
  throw new Error(`Windows signing event log has no ${label}`)
}

export function packagedArtifactsFromSigningEvents({ events, installerPath, shellName = 'shellx-cut.exe', cutdName = 'cutd.exe' }) {
  const installer = resolve(installerPath)
  const installerIndex = latestIndex(events,
    (event) => resolve(event.artifactPath) === installer,
    'final installer signature')
  const installerEvent = events[installerIndex]
  invariant(installerEvent.sha256 === sha256File(installer), 'final installer changed after its Authenticode signing event')

  const packagedShellIndex = latestIndex(events.slice(0, installerIndex),
    (event) => basename(event.artifactPath).toLowerCase() === shellName,
    'packaged shell signature before installer')
  const packagedCutdIndex = latestIndex(events.slice(0, installerIndex),
    (event) => basename(event.artifactPath).toLowerCase() === cutdName,
    'packaged cutd signature before installer')
  return {
    installer: installerEvent,
    shell: events[packagedShellIndex],
    cutd: events[packagedCutdIndex],
  }
}
