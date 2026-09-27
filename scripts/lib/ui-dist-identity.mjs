import { createHash } from 'node:crypto'
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

import { sourceContentManifest } from './source-content-manifest.mjs'

export const UI_DIST_IDENTITY_SCHEMA = 'shellx-cut/ui-dist-identity@1'
export const UI_DIST_IDENTITY_GUARD = 'BUILD-UI-IDENTITY-01'
export const UI_DIST_IDENTITY_FILE = '.shellx-cut-ui-identity.json'
export const UI_DIST_SOURCE_CONTENT_ATTRIBUTE = 'data-cut-ui-source-content-manifest-sha256'

const SHA256 = /^[a-f0-9]{64}$/

function identityError(message) {
  throw new Error(`${UI_DIST_IDENTITY_GUARD}: ${message}`)
}

function digest(path, label) {
  try {
    return createHash('sha256').update(readFileSync(path)).digest('hex')
  } catch (error) {
    identityError(`cannot read ${label} (${path}): ${error.message}`)
  }
}

function packagedDistFiles(dist) {
  const files = []
  function visit(directory, relative = '') {
    for (const name of readdirSync(directory).sort()) {
      if (!relative && name === UI_DIST_IDENTITY_FILE) continue
      const path = join(directory, name)
      const entry = lstatSync(path)
      const packagePath = relative ? `${relative}/${name}` : name
      if (entry.isDirectory()) visit(path, packagePath)
      else if (entry.isFile()) files.push({ path: packagePath, sha256: digest(path, packagePath) })
      else identityError(`ui/dist contains a non-regular package entry: ${packagePath}`)
    }
  }
  visit(dist)
  if (!files.some((entry) => entry.path === 'index.html')) identityError('ui/dist file manifest is missing index.html')
  return files
}

function packageVersion(root) {
  try {
    const value = JSON.parse(readFileSync(join(root, 'ui/package.json'), 'utf8')).version
    if (typeof value !== 'string' || !/^\d+\.\d+\.\d+$/.test(value)) identityError('ui/package.json version must be an exact major.minor.patch version')
    return value
  } catch (error) {
    if (error.message.startsWith(`${UI_DIST_IDENTITY_GUARD}:`)) throw error
    identityError(`cannot parse ui/package.json: ${error.message}`)
  }
}

export function currentUiDistIdentity({ repoRoot } = {}) {
  const root = resolve(repoRoot || '.')
  const source = sourceContentManifest(root)
  return {
    schema: UI_DIST_IDENTITY_SCHEMA,
    version: packageVersion(root),
    source: {
      content_manifest: {
        schema: source.schema,
        files: source.files,
        bytes: source.bytes,
        sha256: source.sha256,
      },
      backend_cargo_toml_sha256: digest(join(root, 'app/Cargo.toml'), 'app/Cargo.toml'),
      verb_schema_sha256: digest(join(root, 'schema/verbs.json'), 'schema/verbs.json'),
      ui_package_sha256: digest(join(root, 'ui/package.json'), 'ui/package.json'),
      ui_lock_sha256: digest(join(root, 'ui/package-lock.json'), 'ui/package-lock.json'),
    },
  }
}

export function writeUiDistIdentity({ repoRoot, distPath } = {}) {
  const root = resolve(repoRoot || '.')
  const dist = resolve(distPath || join(root, 'ui/dist'))
  const indexPath = join(dist, 'index.html')
  if (!existsSync(indexPath)) {
    identityError(`ui/dist is missing index.html at ${dist}; run npm --prefix ui run build before writing identity`)
  }
  const identity = currentUiDistIdentity({ repoRoot: root })
  stampUiDistIndexIdentity(indexPath, identity.source.content_manifest.sha256)
  mkdirSync(dist, { recursive: true })
  identity.dist = { files: packagedDistFiles(dist) }
  writeFileSync(join(dist, UI_DIST_IDENTITY_FILE), `${JSON.stringify(identity, null, 2)}\n`)
  return identity
}

function stampUiDistIndexIdentity(indexPath, sourceContentManifestSha256) {
  if (!SHA256.test(sourceContentManifestSha256)) {
    identityError('ui/dist index identity requires a source-content manifest sha256')
  }
  let html
  try {
    html = readFileSync(indexPath, 'utf8')
  } catch (error) {
    identityError(`cannot read ui/dist index ${indexPath}: ${error.message}`)
  }
  const openingHtml = /<html\b[^>]*>/i.exec(html)
  if (!openingHtml) identityError(`ui/dist index ${indexPath} has no opening html element`)
  const marker = new RegExp(`\\s${UI_DIST_SOURCE_CONTENT_ATTRIBUTE}=(?:"[^"]*"|'[^']*'|[^\\s>]+)`, 'i')
  const stamped = `${openingHtml[0].replace(marker, '').slice(0, -1)} ${UI_DIST_SOURCE_CONTENT_ATTRIBUTE}="${sourceContentManifestSha256}">`
  writeFileSync(indexPath, `${html.slice(0, openingHtml.index)}${stamped}${html.slice(openingHtml.index + openingHtml[0].length)}`)
}

function readUiDistIndexIdentity(indexPath) {
  let html
  try {
    html = readFileSync(indexPath, 'utf8')
  } catch (error) {
    identityError(`cannot read ui/dist index ${indexPath}: ${error.message}`)
  }
  const value = new RegExp(`\\s${UI_DIST_SOURCE_CONTENT_ATTRIBUTE}="([^"]*)"`, 'i').exec(html)?.[1]
  if (!SHA256.test(value || '')) {
    identityError(`ui/dist index is missing a valid ${UI_DIST_SOURCE_CONTENT_ATTRIBUTE}; rebuild with npm --prefix ui run build`)
  }
  return value
}

function readIdentity(path) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'))
  } catch (error) {
    identityError(`cannot parse ui/dist identity ${path}: ${error.message}`)
  }
}

function same(path, built, current) {
  if (built !== current) identityError(`${path} mismatch (built ${JSON.stringify(built)}, current ${JSON.stringify(current)}); rebuild with npm --prefix ui run build`)
}

export function checkUiDistIdentity({ repoRoot, distPath } = {}) {
  const root = resolve(repoRoot || '.')
  const dist = resolve(distPath || join(root, 'ui/dist'))
  if (!existsSync(join(dist, 'index.html'))) identityError(`ui/dist is missing index.html at ${dist}; run npm --prefix ui run build`)
  const identityPath = join(dist, UI_DIST_IDENTITY_FILE)
  if (!existsSync(identityPath)) identityError(`ui/dist is missing ${UI_DIST_IDENTITY_FILE}; rebuild with npm --prefix ui run build`)
  const built = readIdentity(identityPath)
  const current = currentUiDistIdentity({ repoRoot: root })
  if (built.schema !== UI_DIST_IDENTITY_SCHEMA) identityError(`ui/dist identity schema is ${JSON.stringify(built.schema)}; expected ${UI_DIST_IDENTITY_SCHEMA}`)
  same('UI version', built.version, current.version)
  same('backend app/Cargo.toml sha256', built.source?.backend_cargo_toml_sha256, current.source.backend_cargo_toml_sha256)
  same('schema/verbs.json sha256', built.source?.verb_schema_sha256, current.source.verb_schema_sha256)
  same('ui/package.json sha256', built.source?.ui_package_sha256, current.source.ui_package_sha256)
  same('ui/package-lock.json sha256', built.source?.ui_lock_sha256, current.source.ui_lock_sha256)
  same('source-content manifest schema', built.source?.content_manifest?.schema, current.source.content_manifest.schema)
  same('source-content manifest files', built.source?.content_manifest?.files, current.source.content_manifest.files)
  same('source-content manifest bytes', built.source?.content_manifest?.bytes, current.source.content_manifest.bytes)
  same('source-content manifest sha256', built.source?.content_manifest?.sha256, current.source.content_manifest.sha256)
  same('ui/dist index source-content manifest sha256', readUiDistIndexIdentity(join(dist, 'index.html')), current.source.content_manifest.sha256)
  if (JSON.stringify(built.dist?.files) !== JSON.stringify(packagedDistFiles(dist))) {
    identityError('ui/dist file manifest mismatch; rebuild with npm --prefix ui run build')
  }
  return current
}
