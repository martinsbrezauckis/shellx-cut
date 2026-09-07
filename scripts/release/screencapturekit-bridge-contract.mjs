import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const MINIMUM_VERSION = [8, 0, 1]
const MANIFEST_PATH = ['app', 'recorder', 'record-capture', 'Cargo.toml']
const LOCK_PATH = ['app', 'Cargo.lock']

function parseVersion(value) {
  const match = String(value).match(/^(\d+)\.(\d+)\.(\d+)$/)
  return match ? match.slice(1).map(Number) : null
}

function compareVersions(left, right) {
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index]
  }
  return 0
}

function declaredMinimum(requirement) {
  const match = String(requirement).match(/^(?:\^)?(8\.\d+\.\d+)$/)
  return match ? parseVersion(match[1]) : null
}

function packageVersions(lockText, packageName) {
  return String(lockText).split(/^\[\[package\]\]$/m).slice(1).flatMap((block) => {
    const name = block.match(/^name\s*=\s*"([^"]+)"/m)?.[1]
    const version = block.match(/^version\s*=\s*"([^"]+)"/m)?.[1]
    return name === packageName && version ? [version] : []
  })
}

export function evaluateMacosScreenCaptureKitBridgeContract({ manifestText, lockText }) {
  const failures = []
  const dependency = String(manifestText).match(/^screencapturekit\s*=\s*\{([^}]*)\}/m)?.[1]
  const requirement = dependency?.match(/\bversion\s*=\s*"([^"]+)"/)?.[1]
  const minimum = declaredMinimum(requirement)
  if (!minimum || compareVersions(minimum, MINIMUM_VERSION) < 0) {
    failures.push(
      `record-capture must declare screencapturekit 8.x with a minimum of 8.0.1; found ${requirement || 'none'}`,
    )
  }
  if (!dependency?.match(/\bfeatures\s*=\s*\[[^\]]*"macos_15_0"/)) {
    failures.push('record-capture must retain screencapturekit macos_15_0 feature')
  }

  const resolved = packageVersions(lockText, 'screencapturekit')
  const version = resolved.length === 1 ? parseVersion(resolved[0]) : null
  if (resolved.length !== 1) {
    failures.push(`Cargo.lock must resolve exactly one screencapturekit package; found ${resolved.length}`)
  } else if (!version || version[0] !== 8 || compareVersions(version, MINIMUM_VERSION) < 0) {
    failures.push(`Cargo.lock resolves unsupported screencapturekit ${resolved[0]}; require 8.0.1 or newer in major 8`)
  }
  return { failures, resolved }
}

export function macosScreenCaptureKitBridgeContract({ repo, readFile = readFileSync }) {
  try {
    return evaluateMacosScreenCaptureKitBridgeContract({
      manifestText: readFile(resolve(repo, ...MANIFEST_PATH), 'utf8'),
      lockText: readFile(resolve(repo, ...LOCK_PATH), 'utf8'),
    })
  } catch (error) {
    return { failures: [`could not read ScreenCaptureKit bridge contract: ${error.message}`], resolved: [] }
  }
}
