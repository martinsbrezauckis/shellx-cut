import { execFileSync, spawnSync } from 'node:child_process'
import { lstatSync, readFileSync } from 'node:fs'
import { basename, isAbsolute, join, relative, resolve } from 'node:path'

import { sourceContentManifest } from './source-content-manifest.mjs'

export const WINDOWS_SIGNING_CANDIDATE_SCHEMA = 'shellx-cut/windows-signing-candidate@1'

const SHA256 = /^[a-f0-9]{64}$/
const COMMIT = /^[a-f0-9]{40}$/
const TARGET = 'x86_64-pc-windows-msvc'
const MODES = new Set(['debug', 'release'])

function invariant(condition, message) {
  if (!condition) throw new Error(message)
}

function git(root, args) {
  return execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim()
}

function regularFile(path, label) {
  let stat
  try {
    stat = lstatSync(path)
  } catch {
    throw new Error(`${label} does not exist: ${path}`)
  }
  invariant(stat.isFile() && !stat.isSymbolicLink(), `${label} must be a regular non-symlink file: ${path}`)
}

function containedPath(root, candidate, label) {
  const resolvedRoot = resolve(root)
  const resolvedCandidate = resolve(candidate)
  const pathRelative = relative(resolvedRoot, resolvedCandidate)
  invariant(
    pathRelative !== '' && !pathRelative.startsWith('../') && pathRelative !== '..' && !isAbsolute(pathRelative),
    `${label} must be beneath ${resolvedRoot}`,
  )
  return resolvedCandidate
}

function sourceVersion(root) {
  const configPath = join(root, 'app/desktop/src-tauri/tauri.conf.json')
  const config = JSON.parse(readFileSync(configPath, 'utf8'))
  invariant(typeof config.version === 'string' && /^\d+\.\d+\.\d+/.test(config.version), 'Cut Tauri version is invalid')
  invariant(config.productName === 'ShellX Cut', 'Cut Windows signing only supports the ShellX Cut product identity')
  return config.version
}

function validateSource(source) {
  invariant(COMMIT.test(String(source?.gitCommit || '')), 'signing candidate source commit is invalid')
  invariant(COMMIT.test(String(source?.gitTree || '')), 'signing candidate source tree is invalid')
  invariant(SHA256.test(String(source?.contentManifestSha256 || '')), 'signing candidate source hash is invalid')
  invariant(/^\d+\.\d+\.\d+/.test(String(source?.version || '')), 'signing candidate source version is invalid')
}

export function windowsSigningSourceIdentity(root) {
  const canonicalRoot = resolve(root)
  return {
    gitCommit: git(canonicalRoot, ['rev-parse', 'HEAD']),
    gitTree: git(canonicalRoot, ['rev-parse', 'HEAD^{tree}']),
    contentManifestSha256: sourceContentManifest(canonicalRoot).sha256,
    version: sourceVersion(canonicalRoot),
  }
}

export function candidateCoordinatesFromPath(root, path) {
  const canonicalRoot = resolve(root)
  const candidatePath = containedPath(canonicalRoot, path, 'signing candidate')
  const pathRelative = relative(canonicalRoot, candidatePath).replaceAll('\\', '/')
  const match = /^app\/desktop\/src-tauri\/target\/(x86_64-pc-windows-msvc)\/(debug|release)\/windows-signing-candidate\.json$/.exec(pathRelative)
  invariant(match, 'signing candidate path is not the Cut-owned Windows target manifest')
  return { target: match[1], mode: match[2] }
}

export function windowsSigningCandidatePath(root, { target = TARGET, mode }) {
  invariant(target === TARGET, `unsupported Cut Windows signing target: ${target}`)
  invariant(MODES.has(mode), `unsupported Cut Windows signing mode: ${mode}`)
  return join(resolve(root), 'app/desktop/src-tauri/target', target, mode, 'windows-signing-candidate.json')
}

export function createWindowsSigningCandidate({ source, target = TARGET, mode }) {
  validateSource(source)
  invariant(target === TARGET, `unsupported Cut Windows signing target: ${target}`)
  invariant(MODES.has(mode), `unsupported Cut Windows signing mode: ${mode}`)

  const artifactRoot = `app/desktop/src-tauri/target/${target}/${mode}`
  const version = source.version
  return {
    schema: WINDOWS_SIGNING_CANDIDATE_SCHEMA,
    source: { ...source },
    target,
    mode,
    artifactRoot,
    artifacts: [
      { id: 'cutd', path: `app/target/${target}/${mode}/cutd.exe` },
      { id: 'shell', path: `${artifactRoot}/shellx-cut.exe` },
      { id: 'installer', path: `${artifactRoot}/bundle/nsis/ShellX Cut_${version}_x64-setup.exe` },
    ],
  }
}

export function createCurrentWindowsSigningCandidate(root, { target = TARGET, mode } = {}) {
  return createWindowsSigningCandidate({
    source: windowsSigningSourceIdentity(root),
    target,
    mode,
  })
}

export function validateWindowsSigningCandidate(candidate, { source, target, mode }) {
  const expected = createWindowsSigningCandidate({ source, target, mode })
  invariant(
    JSON.stringify(candidate) === JSON.stringify(expected),
    'signing candidate does not exactly match the Cut source hash and controlled artifact layout',
  )
  return expected
}

export function readCurrentWindowsSigningCandidate(root, candidatePath) {
  const canonicalRoot = resolve(root)
  const coordinates = candidateCoordinatesFromPath(canonicalRoot, candidatePath)
  const expectedPath = windowsSigningCandidatePath(canonicalRoot, coordinates)
  invariant(resolve(candidatePath) === expectedPath, 'signing candidate must use its exact Cut-owned target path')
  regularFile(expectedPath, 'signing candidate')

  let candidate
  try {
    candidate = JSON.parse(readFileSync(expectedPath, 'utf8'))
  } catch (error) {
    throw new Error(`could not read signing candidate ${expectedPath}: ${error.message}`)
  }
  return validateWindowsSigningCandidate(candidate, {
    source: windowsSigningSourceIdentity(canonicalRoot),
    ...coordinates,
  })
}

export function assertControlledWindowsPeArtifact(root, candidate, artifactPath) {
  const canonicalRoot = resolve(root)
  const artifact = resolve(artifactPath)
  const expectedPaths = new Map(candidate.artifacts.map((entry) => [resolve(canonicalRoot, entry.path), entry.id]))
  const id = expectedPaths.get(artifact)
  invariant(id, `Windows signing artifact is outside the exact Cut candidate boundary: ${artifact}`)
  invariant(/\.(exe|msi)$/i.test(artifact), `Windows signing artifact is not a supported PE path: ${artifact}`)
  regularFile(artifact, 'Windows signing artifact')
  const header = readFileSync(artifact, { encoding: null }).subarray(0, 2).toString('ascii')
  invariant(header === 'MZ', `Windows signing artifact is not a PE file: ${artifact}`)
  return { id, path: artifact, name: basename(artifact) }
}

export function assertCleanTrackedSource(root) {
  const canonicalRoot = resolve(root)
  for (const args of [['diff', '--quiet', '--'], ['diff', '--cached', '--quiet', '--']]) {
    const result = spawnSync('git', args, { cwd: canonicalRoot, encoding: 'utf8' })
    if (result.status === 1) throw new Error('Cut Windows signing candidate requires a clean tracked source checkout')
    if (result.status !== 0) throw new Error(`could not inspect Cut source state: ${(result.stderr || result.stdout).trim()}`)
  }
  const untracked = git(canonicalRoot, ['status', '--porcelain=v1', '--untracked-files=all'])
  invariant(!untracked, 'Cut Windows signing candidate requires no untracked source files')
}
