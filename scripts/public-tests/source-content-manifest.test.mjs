import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { assertPortableSourcePaths, sourceContentManifest } from '../lib/source-content-manifest.mjs'

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'cut-source-content-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  for (const name of ['.gitignore', 'LICENSE', 'NOTICE', 'README.md', 'SECURITY.md', 'START_HERE_FOR_AGENT.txt']) {
    writeFileSync(join(root, name), `${name}\n`)
  }
  mkdirSync(join(root, '.github', 'workflows'), { recursive: true })
  writeFileSync(join(root, '.github', 'workflows', 'ci.yml'), 'ci workflow\n')
  writeFileSync(join(root, '.github', 'workflows', 'release.yml'), 'release workflow\n')
  for (const dir of ['app', 'docs', 'schema', 'scripts', 'skill', 'testdata', 'ui']) {
    mkdirSync(join(root, dir))
    if (dir !== 'testdata') writeFileSync(join(root, dir, 'source.txt'), `${dir}\n`)
  }
  writeFileSync(join(root, 'testdata', 'test_lut_invert.cube'), 'tracked LUT fixture\n')
  return root
}

test('source manifest is deterministic and ignores rebuildable output', (t) => {
  const root = fixture(t)
  const first = sourceContentManifest(root)
  const modules = mkdtempSync(join(tmpdir(), 'cut-source-modules-'))
  t.after(() => rmSync(modules, { recursive: true, force: true }))
  mkdirSync(join(modules, 'ignored'), { recursive: true })
  symlinkSync(modules, join(root, 'ui', 'node_modules'), process.platform === 'win32' ? 'junction' : 'dir')
  mkdirSync(join(root, 'app', 'target'), { recursive: true })
  mkdirSync(join(root, 'app', 'desktop', 'src-tauri', 'binaries'), { recursive: true })
  mkdirSync(join(root, 'app', 'desktop', 'src-tauri', 'gen', 'schemas'), { recursive: true })
  mkdirSync(join(root, 'app', 'perception', 'py', '__pycache__'), { recursive: true })
  mkdirSync(join(root, 'docs', 'private'), { recursive: true })
  mkdirSync(join(root, 'testdata', 'real'), { recursive: true })
  mkdirSync(join(root, 'ui', 'private-tests', '__release__'), { recursive: true })
  mkdirSync(join(root, 'ui', '.playwright-cli'), { recursive: true })
  mkdirSync(join(root, 'ui', '.scratch', 'playwright'), { recursive: true })
  mkdirSync(join(root, 'ui', 'logs'), { recursive: true })
  writeFileSync(join(modules, 'ignored', 'package.js'), 'ignored\n')
  writeFileSync(join(root, 'app', 'target', 'binary'), 'ignored\n')
  writeFileSync(join(root, 'app', 'desktop', 'src-tauri', 'binaries', 'cutd'), 'ignored\n')
  writeFileSync(join(root, 'app', 'desktop', 'src-tauri', 'gen', 'schemas', 'desktop.json'), 'ignored\n')
  writeFileSync(join(root, 'app', 'perception', 'py', '__pycache__', 'runner.pyc'), 'ignored\n')
  writeFileSync(join(root, 'docs', 'private', 'README.md'), 'private local notes\n')
  writeFileSync(join(root, 'testdata', 'talking_head.mp4'), 'generated media\n')
  writeFileSync(join(root, 'testdata', 'real', 'intro.png'), 'governed external media\n')
  writeFileSync(join(root, 'ui', 'private-tests', '__release__', 'surface.png'), 'ignored\n')
  writeFileSync(join(root, 'ui', '.playwright-cli', 'page.yml'), 'ignored browser state\n')
  writeFileSync(join(root, 'ui', '.scratch', 'playwright', 'vite.log'), 'ignored browser scratch\n')
  writeFileSync(join(root, 'ui', 'logs', 'wdio-run.log'), 'generated native-driver log\n')
  writeFileSync(join(root, 'ui', 'tsconfig.tsbuildinfo'), 'ignored\n')
  const second = sourceContentManifest(root)
  assert.equal(second.sha256, first.sha256)
  assert.deepEqual(
    second.rows.map((row) => row.path),
    [...second.rows.map((row) => row.path)].sort((a, b) => a.localeCompare(b)),
  )
  assert.equal(second.rows.some((row) => row.path === 'testdata/test_lut_invert.cube'), true)
  assert.equal(second.rows.some((row) => row.path === 'testdata/talking_head.mp4'), false)
  assert.equal(second.rows.some((row) => row.path === 'testdata/real/intro.png'), false)
  assert.equal(second.rows.some((row) => row.path === 'docs/private/README.md'), false)
  assert.equal(second.rows.some((row) => row.path === 'ui/logs/wdio-run.log'), false)
})

test('source manifest changes when synchronized source bytes change', (t) => {
  const root = fixture(t)
  const first = sourceContentManifest(root)
  writeFileSync(join(root, 'ui', 'source.txt'), 'changed\n')
  const second = sourceContentManifest(root)
  assert.notEqual(second.sha256, first.sha256)
  assert.equal(second.files, first.files)
})

test('source manifest binds root security and workflow policy bytes', (t) => {
  const root = fixture(t)
  const first = sourceContentManifest(root)
  assert.equal(first.rows.some((row) => row.path === 'SECURITY.md'), true)
  assert.equal(first.rows.some((row) => row.path === '.github/workflows/release.yml'), true)
  assert.equal(first.rows.some((row) => row.path === '.github/workflows/ci.yml'), true)

  writeFileSync(join(root, 'SECURITY.md'), 'changed security policy\n')
  const securityChanged = sourceContentManifest(root)
  assert.notEqual(securityChanged.sha256, first.sha256)

  writeFileSync(join(root, '.github', 'workflows', 'release.yml'), 'changed release workflow\n')
  const releaseWorkflowChanged = sourceContentManifest(root)
  assert.notEqual(releaseWorkflowChanged.sha256, securityChanged.sha256)

  writeFileSync(join(root, '.github', 'workflows', 'ci.yml'), 'changed CI workflow\n')
  const ciWorkflowChanged = sourceContentManifest(root)
  assert.notEqual(ciWorkflowChanged.sha256, releaseWorkflowChanged.sha256)
})

test('source manifest rejects paths that collide on macOS and Windows filesystems', () => {
  assert.throws(
    () => assertPortableSourcePaths([
      'ui/src/panels/Transcript/ChapterNavigation.tsx',
      'ui/src/panels/Transcript/chapterNavigation.ts',
    ]),
    /collide under case-insensitive resolution.*ChapterNavigation[.]tsx.*chapterNavigation[.]ts/,
  )
})
