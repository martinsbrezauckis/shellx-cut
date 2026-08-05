import { createHash } from 'node:crypto'
import { lstatSync, readFileSync, readdirSync, readlinkSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'

const ROOT_FILES = [
  '.gitignore',
  'AGENTS.md',
  'LICENSE',
  'NOTICE',
  'README.md',
  'START_HERE_FOR_AGENT.txt',
  'testdata/test_lut_invert.cube',
]
const ROOT_DIRS = ['app', 'docs', 'schema', 'scripts', 'skill', 'ui']
const EXCLUDED_DIRS = new Set([
  '.git', '.project', '.shellx-scratch', '.venv', '.worktrees',
  '__pycache__', 'dist', 'node_modules', 'target',
])
const EXCLUDED_PATHS = new Set([
  'app/desktop/src-tauri/binaries',
  'app/desktop/src-tauri/gen',
  'docs/private',
  'ui/public-tests/__evidence__',
  'ui/public-tests/__release__',
])
const EXCLUDED_FILES = new Set([
  'ui/tsconfig.tsbuildinfo',
])

function entries(root, dir, out) {
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    const path = join(dir, entry.name)
    const relativePath = relative(root, path).replaceAll('\\', '/')
    if (entry.isDirectory() && (EXCLUDED_DIRS.has(entry.name) || EXCLUDED_PATHS.has(relativePath))) continue
    if (!entry.isDirectory() && EXCLUDED_FILES.has(relativePath)) continue
    if (entry.isDirectory()) entries(root, path, out)
    else out.push({ path, relative: relativePath })
  }
}

export function sourceContentManifest(repoRoot) {
  const root = resolve(repoRoot)
  const files = ROOT_FILES.map((name) => ({ path: join(root, name), relative: name }))
  for (const name of ROOT_DIRS) entries(root, join(root, name), files)
  const rows = []
  let bytes = 0
  for (const file of files.sort((a, b) => a.relative.localeCompare(b.relative))) {
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
