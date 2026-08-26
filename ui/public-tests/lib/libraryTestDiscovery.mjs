import { readFile, readdir } from 'node:fs/promises'
import { dirname, join, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
export const DEFAULT_PUBLIC_TESTS_ROOT = resolve(here, '..')
export const DEFAULT_EXCLUSIONS = resolve(DEFAULT_PUBLIC_TESTS_ROOT, 'library-test-exclusions.json')
const TEST_SOURCE = /[.]test[.](?:ts|mjs|js)$/

function portable(relativePath) {
  return relativePath.split(sep).join('/')
}

async function walk(root, directory = root) {
  const out = []
  const entries = await readdir(directory, { withFileTypes: true })
  entries.sort((left, right) => left.name.localeCompare(right.name, 'en'))
  for (const entry of entries) {
    const path = join(directory, entry.name)
    if (entry.isSymbolicLink()) {
      throw new Error(`library test discovery refuses symbolic links: ${portable(relative(root, path))}`)
    }
    if (entry.isDirectory()) out.push(...await walk(root, path))
    else if (entry.isFile() && TEST_SOURCE.test(entry.name)) out.push(portable(relative(root, path)))
  }
  return out
}

function validateExclusion(entry, candidates) {
  if (!entry || typeof entry !== 'object') throw new Error('each library test exclusion must be an object')
  if (typeof entry.path !== 'string' || !entry.path || entry.path.startsWith('/') || entry.path.includes('\\') || entry.path.split('/').includes('..')) {
    throw new Error(`invalid library test exclusion path: ${JSON.stringify(entry.path)}`)
  }
  if (!candidates.has(entry.path) && entry.allowAbsentFromPositiveExport !== true) {
    throw new Error(`library test exclusion does not name a discovered test: ${entry.path}`)
  }
  if (entry.allowAbsentFromPositiveExport !== undefined && entry.allowAbsentFromPositiveExport !== true) {
    throw new Error(`allowAbsentFromPositiveExport must be true when present: ${entry.path}`)
  }
  if (typeof entry.reason !== 'string' || entry.reason.trim().length < 20) {
    throw new Error(`library test exclusion needs a concrete reason: ${entry.path}`)
  }
  if (typeof entry.ownerCommand !== 'string' || !entry.ownerCommand.trim()) {
    throw new Error(`library test exclusion needs an owner command: ${entry.path}`)
  }
}

export async function discoverLibraryTests({
  publicTestsRoot = DEFAULT_PUBLIC_TESTS_ROOT,
  exclusionsPath = DEFAULT_EXCLUSIONS,
} = {}) {
  const root = resolve(publicTestsRoot)
  const candidates = new Set((await walk(root)).sort())
  const registry = JSON.parse(await readFile(exclusionsPath, 'utf8'))
  if (registry.schema !== 'shellx-cut/library-test-exclusions@1' || !Array.isArray(registry.exclusions)) {
    throw new Error('library test exclusions must use shellx-cut/library-test-exclusions@1')
  }

  const excluded = new Map()
  for (const entry of registry.exclusions) {
    validateExclusion(entry, candidates)
    if (excluded.has(entry.path)) throw new Error(`duplicate library test exclusion: ${entry.path}`)
    excluded.set(entry.path, entry)
  }
  const included = [...candidates].filter((path) => !excluded.has(path))
  if (included.length === 0) throw new Error('library test discovery found no eligible tests')
  return {
    included,
    excluded: [...excluded.values()].sort((left, right) => left.path.localeCompare(right.path, 'en')),
  }
}
