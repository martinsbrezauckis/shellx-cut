import { createHash } from 'node:crypto'
import { existsSync, lstatSync, readFileSync } from 'node:fs'
import { readFile, readdir } from 'node:fs/promises'
import { dirname, join, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

export const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
export const uiRoot = resolve(repoRoot, 'ui')
export const legacyManualRoot = resolve(repoRoot, 'docs/public/site/manual')
export const publicationManifestName = 'publication-manifest.json'

const UI_BUILD_INPUTS = [
  'ui/package.json',
  'ui/package-lock.json',
  'ui/index.html',
  'ui/manual.html',
  'ui/vite.config.ts',
  'ui/tsconfig.json',
  'ui/tsconfig.app.json',
  'ui/tsconfig.node.json',
]
const UI_SOURCE_ROOTS = ['ui/src', 'ui/public']

export const LEGACY_PUBLICATION_INPUTS = [
  'docs/public/site/manual/cut/index.html',
  'docs/public/site/manual/manual.js',
  'docs/public/site/manual/manual.css',
  'docs/public/site/manual/assets/cut/cut-main-editor-current.png',
  'docs/public/site/manual/assets/cut/cut-recording-studio-current.png',
]

export function fail(message) {
  throw new Error(`Cut manual staging: ${message}`)
}

export function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex')
}

export function normalizedRelative(from, candidate) {
  return relative(from, candidate).split(sep).join('/')
}

export function isInside(parent, candidate) {
  const path = relative(parent, candidate)
  return path === '' || (!path.startsWith(`..${sep}`) && path !== '..')
}

export function assertMatch(value, pattern, description) {
  if (!pattern.test(value)) fail(description)
}

export function assertNoMatch(value, pattern, description) {
  if (pattern.test(value)) fail(description)
}

export function assertRegularFile(path, description) {
  if (!existsSync(path)) fail(`${description} is missing: ${normalizedRelative(repoRoot, path)}`)
  const stat = lstatSync(path)
  if (!stat.isFile() || stat.isSymbolicLink()) fail(`${description} must be a regular file: ${normalizedRelative(repoRoot, path)}`)
}

export function readText(path, description) {
  assertRegularFile(path, description)
  return readFileSync(path, 'utf8')
}

export async function listFiles(root) {
  if (!existsSync(root)) return []
  const entries = await readdir(root, { withFileTypes: true })
  const files = []
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    const path = join(root, entry.name)
    if (entry.isDirectory()) files.push(...await listFiles(path))
    else if (entry.isFile()) files.push(path)
    else fail(`source tree contains a non-regular entry: ${normalizedRelative(repoRoot, path)}`)
  }
  return files
}

async function fingerprintPaths(paths) {
  const entries = []
  for (const path of paths.sort((a, b) => a.localeCompare(b))) {
    assertRegularFile(path, 'build input')
    entries.push({ path: normalizedRelative(repoRoot, path), sha256: sha256(await readFile(path)) })
  }
  const canonical = entries.map((entry) => `${entry.path}\0${entry.sha256}\n`).join('')
  return { fileCount: entries.length, sha256: sha256(canonical), entries }
}

export async function sourceSnapshot() {
  const directInputs = UI_BUILD_INPUTS.map((path) => resolve(repoRoot, path)).filter((path) => existsSync(path))
  const sourceFiles = (await Promise.all(UI_SOURCE_ROOTS.map((path) => listFiles(resolve(repoRoot, path))))).flat()
  const fingerprint = await fingerprintPaths([...new Set([...directInputs, ...sourceFiles])])
  return { ...fingerprint, directInputs: (await fingerprintPaths(directInputs)).entries }
}

export function validateSourceContract() {
  const manualEntry = readText(resolve(uiRoot, 'manual.html'), 'Vite manual entry')
  const viteConfig = readText(resolve(uiRoot, 'vite.config.ts'), 'Vite configuration')
  const main = readText(resolve(uiRoot, 'src/main.tsx'), 'main UI entrypoint')
  const shell = readText(resolve(uiRoot, 'src/manual/ManualShell.tsx'), 'manual shell')
  const bridge = readText(resolve(uiRoot, 'src/manual/useManualFrontendBridge.ts'), 'manual frontend bridge')
  const mock = readText(resolve(uiRoot, 'src/panels/Review/mock.ts'), 'manual mock transport')

  assertMatch(manualEntry, /data-cut-manual-shell=["']true["']/, 'manual.html must select the real manual shell')
  assertMatch(manualEntry, /<script\s+type=["']module["']\s+src=["']\/src\/main\.tsx["']/, 'manual.html must enter through the real Vite main.tsx frontend')
  assertMatch(viteConfig, /input:\s*\{[\s\S]*?manual:\s*fileURLToPath\(new URL\(['"]\.\/manual\.html['"]/, 'Vite must declare manual.html as a production input')
  assertMatch(main, /isManualShell\(\)\s*\?\s*<ManualShell\s*\/>\s*:\s*<App\s*\/>/, 'the Vite entrypoint must mount the real app inside the manual embed')
  assertMatch(shell, /<iframe\b/, 'the manual must embed the real frontend rather than a static capture')
  assertMatch(shell, /searchParams\.set\(['"]manual['"],\s*['"]embed['"]\)/, 'manual iframe must opt into embed mode')
  assertMatch(shell, /searchParams\.set\(['"]mock['"],\s*['"]1['"]\)/, 'manual iframe must opt into the deterministic mock')
  assertMatch(bridge, /openSurface/, 'manual reveals must use the real surface opener')
  assertMatch(bridge, /PASSIVE_REVEAL_OPENERS/, 'manual menus must retain their safe real-menu reveal path')
  assertMatch(bridge, /cut:local-highlight/, 'manual reveals must use the frontend highlight bridge')
  assertMatch(mock, /MOCK_EMBEDDED_MANUAL_READ_ONLY[\s\S]*?manual['"]\)\s*===\s*['"]embed['"][\s\S]*?mock['"]\)\s*===\s*['"]1['"]/, 'manual mock must require ?manual=embed&mock=1')
  assertMatch(mock, /MOCK_EMBEDDED_MANUAL_READ_ONLY\s*&&\s*!MANUAL_READ_ONLY_VERBS\.has\(name\)/, 'manual mock must reject mutating verbs')

  for (const legacyPath of LEGACY_PUBLICATION_INPUTS) {
    if (!existsSync(resolve(repoRoot, legacyPath))) fail(`expected historical manual input is missing: ${legacyPath}`)
  }
  const frontend = [manualEntry, main, shell, bridge].join('\n')
  assertNoMatch(frontend, /data-manual-highlight|manual-highlight|cut-main-editor-current|cut-recording-studio-current/i, 'the real-frontend publication source must not use legacy screenshot hotspots')
  assertNoMatch(frontend, /<img\b|\.(?:png|jpe?g|webp|gif)\b/i, 'the real-frontend publication source must not use screenshot assets')
  return { architecture: 'vite-real-frontend', embedUrl: '?manual=embed&mock=1', legacyPublicationInputs: LEGACY_PUBLICATION_INPUTS }
}

export async function readViteManifest(buildDir) {
  const path = [join(buildDir, '.vite/manifest.json'), join(buildDir, 'manifest.json')].find((candidate) => existsSync(candidate))
  if (!path) fail('Vite did not produce a manifest')
  const manifest = JSON.parse(await readFile(path, 'utf8'))
  if (!manifest || typeof manifest !== 'object' || Array.isArray(manifest)) fail('Vite manifest has an invalid shape')
  return manifest
}

function assetPath(buildDir, value) {
  if (typeof value !== 'string' || value.length === 0 || value.startsWith('/') || value.includes('\\')) fail(`invalid Vite artifact path: ${String(value)}`)
  const path = resolve(buildDir, value)
  if (!isInside(buildDir, path)) fail(`Vite artifact escaped the build directory: ${value}`)
  assertRegularFile(path, 'Vite artifact')
  return path
}

export function manualClosure(manifest, buildDir) {
  // Vite 7 can coalesce two HTML inputs that share one TypeScript entrypoint.
  // In that shape the manifest has chunk records but no HTML `isEntry` record,
  // so seed the closure from the actual built manual.html and use the manifest
  // only to expand those exact static and dynamic chunk dependencies.
  const manualHtml = readText(resolve(buildDir, 'manual.html'), 'built manual HTML')
  const seedFiles = new Set([...manualHtml.matchAll(/(?:src|href)="\.\/(assets\/[^"?#]+)(?:[?#][^"]*)?"/g)]
    .map((match) => match[1]))
  if (seedFiles.size === 0) fail('built manual HTML has no relative Vite asset seeds')
  const keyByFile = new Map()
  for (const [key, entry] of Object.entries(manifest)) {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry) || typeof entry.file !== 'string') continue
    if (keyByFile.has(entry.file)) fail(`Vite manifest has ambiguous artifact ownership: ${entry.file}`)
    keyByFile.set(entry.file, key)
  }
  const closure = new Set()
  const visited = new Set()
  const collect = (key) => {
    if (visited.has(key)) return
    visited.add(key)
    const entry = manifest[key]
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) fail(`manifest import is invalid: ${key}`)
    if (typeof entry.file === 'string') closure.add(entry.file)
    for (const field of ['css', 'assets']) {
      if (entry[field] === undefined) continue
      if (!Array.isArray(entry[field])) fail(`manifest ${field} is invalid for ${key}`)
      for (const file of entry[field]) closure.add(file)
    }
    for (const field of ['imports', 'dynamicImports']) {
      if (entry[field] === undefined) continue
      if (!Array.isArray(entry[field])) fail(`manifest ${field} is invalid for ${key}`)
      for (const imported of entry[field]) collect(imported)
    }
  }
  for (const file of seedFiles) {
    const key = keyByFile.get(file)
    if (!key) fail(`built manual asset is absent from the Vite manifest: ${file}`)
    collect(key)
  }
  return [...closure].sort().map((file) => assetPath(buildDir, file))
}

export async function outputFiles(root) {
  const files = await listFiles(root)
  return Promise.all(files.filter((path) => path !== join(root, publicationManifestName))
    .map(async (path) => ({ path: normalizedRelative(root, path), sha256: sha256(await readFile(path)) })))
}
