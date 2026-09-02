import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

export const RELEASE_DOC_TRUTH_GUARD = 'RELEASE-DOC-TRUTH-01'
export const RELEASE_TRUTH_SCHEMA = 'shellx-cut/release-truth@1'

const MARKER_PATHS = [
  'README.md',
  'SECURITY.md',
  'docs/public/FEATURES.md',
  'docs/public/DEBUG_API.md',
  'docs/public/shellx-cut-threat-model.md',
  'skill/shellx-cut/SKILL.md',
  'skill/shellx-cut/reference.md',
]

function truthError(message) {
  throw new Error(`${RELEASE_DOC_TRUTH_GUARD}: ${message}`)
}

function read(root, path) {
  try {
    return readFileSync(join(root, path), 'utf8')
  } catch (error) {
    truthError(`cannot read ${path}: ${error.message}`)
  }
}

function requireMatch(source, pattern, path, expectation) {
  if (!pattern.test(source)) truthError(`${path} ${expectation}`)
}

function semver(value, path) {
  if (typeof value !== 'string' || !/^\d+\.\d+\.\d+$/.test(value)) truthError(`${path} must be an exact major.minor.patch version`)
  return value.split('.').map(Number)
}

function compareVersions(left, right) {
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index]
  }
  return 0
}

function packageLockVersion(source) {
  const lock = JSON.parse(source)
  return [lock.version, lock.packages?.['']?.version]
}

function cargoPackageVersion(lockSource, name) {
  const blocks = lockSource.split('[[package]]').slice(1)
  const versions = blocks
    .filter((block) => new RegExp(`^name = ${JSON.stringify(name)}$`, 'm').test(block))
    .map((block) => block.match(/^version = "([^"]+)"$/m)?.[1])
    .filter(Boolean)
  if (versions.length !== 1) truthError(`app/Cargo.lock must contain exactly one ${name} package entry`)
  return versions[0]
}

function workspacePackageNames(root, appManifest) {
  const membersBlock = appManifest.match(/members\s*=\s*\[([\s\S]*?)\]/)?.[1]
  if (!membersBlock) truthError('app/Cargo.toml must declare workspace members')
  const memberPaths = [...membersBlock.matchAll(/"([^"]+)"/g)].map((match) => match[1])
  return memberPaths.flatMap((member) => {
    const manifest = read(root, `app/${member}/Cargo.toml`)
    if (!/version\.workspace\s*=\s*true/.test(manifest)) return []
    const name = manifest.match(/\[package\][\s\S]*?^name\s*=\s*"([^"]+)"/m)?.[1]
    if (!name) truthError(`app/${member}/Cargo.toml must declare a package name`)
    return [name]
  })
}

export function checkReleaseDocTruth({ repoRoot } = {}) {
  const root = resolve(repoRoot || '.')
  const schema = JSON.parse(read(root, 'schema/verbs.json'))
  const base = JSON.parse(read(root, 'schema/verbs/base.json'))
  const truth = schema.release_truth
  if (!truth || typeof truth !== 'object' || Array.isArray(truth)) truthError('schema/verbs.json must declare release_truth')
  if (truth.schema !== RELEASE_TRUTH_SCHEMA) truthError(`schema/verbs.json release_truth.schema must be ${RELEASE_TRUTH_SCHEMA}`)
  if (JSON.stringify(base.root?.release_truth) !== JSON.stringify(truth)) {
    truthError('generated schema/verbs.json release_truth differs from schema/verbs/base.json')
  }
  const version = semver(truth.version, 'schema/verbs.json release_truth.version')
  const published = semver(truth.published_version, 'schema/verbs.json release_truth.published_version')
  if (!['candidate', 'published'].includes(truth.status)) truthError('schema/verbs.json release_truth.status must be candidate or published')
  if (truth.status === 'candidate' && compareVersions(version, published) <= 0) {
    truthError(`candidate version ${truth.version} must be newer than published version ${truth.published_version}`)
  }
  if (truth.status === 'published' && compareVersions(version, published) !== 0) {
    truthError(`published version ${truth.version} must equal published_version ${truth.published_version}`)
  }

  const appManifest = read(root, 'app/Cargo.toml')
  const desktopManifest = read(root, 'app/desktop/src-tauri/Cargo.toml')
  const tauri = JSON.parse(read(root, 'app/desktop/src-tauri/tauri.conf.json'))
  const uiPackage = JSON.parse(read(root, 'ui/package.json'))
  const versioned = [
    ['app/Cargo.toml workspace.package.version', appManifest.match(/\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1]],
    ['app/desktop/src-tauri/Cargo.toml package.version', desktopManifest.match(/\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1]],
    ['app/desktop/src-tauri/tauri.conf.json version', tauri.version],
    ['ui/package.json version', uiPackage.version],
  ]
  for (const [path, actual] of versioned) {
    if (actual !== truth.version) truthError(`${path} is ${JSON.stringify(actual)}; expected ${truth.version}`)
  }

  const [lockVersion, rootLockVersion] = packageLockVersion(read(root, 'ui/package-lock.json'))
  for (const [path, actual] of [
    ['ui/package-lock.json version', lockVersion],
    ['ui/package-lock.json packages[""] version', rootLockVersion],
  ]) {
    if (actual !== truth.version) truthError(`${path} is ${JSON.stringify(actual)}; expected ${truth.version}`)
  }
  const appLock = read(root, 'app/Cargo.lock')
  for (const name of workspacePackageNames(root, appManifest)) {
    const actual = cargoPackageVersion(appLock, name)
    if (actual !== truth.version) truthError(`app/Cargo.lock ${name} version is ${actual}; expected ${truth.version}`)
  }
  const desktopLockVersion = cargoPackageVersion(read(root, 'app/desktop/src-tauri/Cargo.lock'), 'shellx-cut')
  if (desktopLockVersion !== truth.version) truthError(`app/desktop/src-tauri/Cargo.lock shellx-cut version is ${desktopLockVersion}; expected ${truth.version}`)

  const marker = `<!-- shellx-cut-release-truth: ${truth.status}; version=${truth.version}; published=${truth.published_version} -->`
  for (const path of MARKER_PATHS) {
    if (!read(root, path).includes(marker)) truthError(`${path} is missing exact release truth marker ${marker}`)
  }

  const readme = read(root, 'README.md')
  requireMatch(readme, new RegExp(`STATUS — v${truth.version.replaceAll('.', '\\.')} ${truth.status}; v${truth.published_version.replaceAll('.', '\\.')} is the latest published release`), 'README.md', 'must state the current candidate/published truth')
  if (truth.status === 'candidate' && new RegExp(`STATUS — ?v?${truth.version.replaceAll('.', '\\.')} release`).test(readme)) {
    truthError(`README.md must not describe candidate ${truth.version} as a published release`)
  }
  const features = read(root, 'docs/public/FEATURES.md')
  requireMatch(features, new RegExp(`## v${truth.version.replaceAll('.', '\\.')} ${truth.status}`), 'docs/public/FEATURES.md', 'must name the current source status')
  requireMatch(features, new RegExp(`v${truth.published_version.replaceAll('.', '\\.')} remains the latest published release`), 'docs/public/FEATURES.md', 'must name the latest published release')
  const manual = read(root, 'docs/public/site/manual/cut/index.html')
  requireMatch(manual, new RegExp(`data-app-version="${truth.version}"`), 'docs/public/site/manual/cut/index.html', 'must bind data-app-version to the candidate version')
  requireMatch(manual, new RegExp(`data-release-status="${truth.status}"`), 'docs/public/site/manual/cut/index.html', 'must expose the source status')
  requireMatch(manual, new RegExp(`data-published-version="${truth.published_version}"`), 'docs/public/site/manual/cut/index.html', 'must expose the latest published version')
  const manualContent = JSON.parse(read(root, 'ui/src/manual/content.generated.json'))
  if (manualContent.appVersion !== truth.version) truthError(`ui/src/manual/content.generated.json appVersion is ${JSON.stringify(manualContent.appVersion)}; expected ${truth.version}`)

  return { version: truth.version, status: truth.status, publishedVersion: truth.published_version }
}
