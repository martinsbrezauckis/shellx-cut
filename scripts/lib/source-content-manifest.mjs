import { createHash } from 'node:crypto'
import { lstatSync, readFileSync, readdirSync, readlinkSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'

const ROOT_FILES = [
  '.gitignore',
  'LICENSE',
  'NOTICE',
  'README.md',
  'SECURITY.md',
  'START_HERE_FOR_AGENT.txt',
  'testdata/test_lut_invert.cube',
]
const ROOT_DIRS = ['.github', 'app', 'docs', 'schema', 'scripts', 'skill', 'ui']
const EXCLUDED_DIRS = new Set([
  '.git', '.playwright-cli', '.project', '.scratch', '.shellx-scratch', '.venv', '.worktrees',
  '__pycache__', 'dist', 'node_modules', 'target',
])
const EXCLUDED_PATHS = new Set([
  'app/desktop/src-tauri/binaries',
  'app/desktop/src-tauri/gen',
  'docs/private',
  'ui/logs',
  'ui/private-tests/__evidence__',
  'ui/private-tests/__release__',
])
const EXCLUDED_FILES = new Set([
  'ui/tsconfig.tsbuildinfo',
])

// Source identities must agree across OS locale settings. JavaScript's
// localeCompare uses the host locale; relational string comparison is a stable
// UTF-16 code-unit order and does not normalize or fold distinct names.
export function compareSourcePath(left, right) {
  if (left < right) return -1
  if (left > right) return 1
  return 0
}

function entries(root, dir, out) {
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => compareSourcePath(a.name, b.name))) {
    const path = join(dir, entry.name)
    const relativePath = relative(root, path).replaceAll('\\', '/')
    if ((entry.isDirectory() || entry.isSymbolicLink())
      && (EXCLUDED_DIRS.has(entry.name) || EXCLUDED_PATHS.has(relativePath))) continue
    if (!entry.isDirectory() && EXCLUDED_FILES.has(relativePath)) continue
    if (entry.isDirectory()) entries(root, path, out)
    else out.push({ path, relative: relativePath })
  }
}

export function assertPortableSourcePaths(paths) {
  const portable = new Map()
  for (const path of paths) {
    const normalized = path.normalize('NFD').toLowerCase()
    const keys = [normalized]
    if (/[.](?:[cm]?[jt]sx?)$/i.test(path)) keys.push(`module:${normalized.replace(/[.](?:[cm]?[jt]sx?)$/i, '')}`)
    for (const key of keys) {
      const existing = portable.get(key)
      if (existing && existing !== path) {
        throw new Error(`source modules collide under case-insensitive resolution: ${existing} and ${path}`)
      }
      portable.set(key, path)
    }
  }
}

export function sourceContentManifest(repoRoot) {
  const root = resolve(repoRoot)
  const files = ROOT_FILES.map((name) => ({ path: join(root, name), relative: name }))
  for (const name of ROOT_DIRS) entries(root, join(root, name), files)
  assertPortableSourcePaths(files.map((file) => file.relative))
  const rows = []
  let bytes = 0
  for (const file of files.sort((a, b) => compareSourcePath(a.relative, b.relative))) {
    const stat = lstatSync(file.path)
    if (stat.isSymbolicLink()) {
      const target = readlinkSync(file.path)
      rows.push({ path: file.relative, kind: 'symlink', bytes: 0, sha256: createHash('sha256').update(target).digest('hex') })
      continue
    }
    if (!stat.isFile()) throw new Error(`source manifest entry is not a file: ${file.relative}`)
    const content = readFileSync(file.path)
    bytes += content.length
    rows.push({
      path: file.relative,
      kind: 'file',
      bytes: content.length,
      sha256: createHash('sha256').update(content).digest('hex'),
    })
  }
  const sha256 = createHash('sha256')
    .update(rows.map((row) => `${row.kind}\0${row.path}\0${row.bytes}\0${row.sha256}\n`).join(''))
    .digest('hex')
  return { schema: 'shellx-cut/source-content-manifest@1', files: rows.length, bytes, sha256, rows }
}
