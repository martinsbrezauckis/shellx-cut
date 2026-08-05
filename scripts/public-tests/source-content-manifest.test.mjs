import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { sourceContentManifest } from '../lib/source-content-manifest.mjs'

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'cut-source-content-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  for (const name of ['.gitignore', 'AGENTS.md', 'LICENSE', 'NOTICE', 'README.md', 'START_HERE_FOR_AGENT.txt']) {
    writeFileSync(join(root, name), `${name}\n`)
  }
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
  mkdirSync(join(root, 'ui', 'node_modules', 'ignored'), { recursive: true })
  mkdirSync(join(root, 'app', 'target'), { recursive: true })
  mkdirSync(join(root, 'app', 'desktop', 'src-tauri', 'binaries'), { recursive: true })
  mkdirSync(join(root, 'app', 'desktop', 'src-tauri', 'gen', 'schemas'), { recursive: true })
  mkdirSync(join(root, 'app', 'perception', 'py', '__pycache__'), { recursive: true })
  mkdirSync(join(root, 'docs', 'private'), { recursive: true })
  mkdirSync(join(root, 'testdata', 'real'), { recursive: true })
  mkdirSync(join(root, 'ui', 'public-tests', '__release__'), { recursive: true })
  writeFileSync(join(root, 'ui', 'node_modules', 'ignored', 'package.js'), 'ignored\n')
  writeFileSync(join(root, 'app', 'target', 'binary'), 'ignored\n')
  writeFileSync(join(root, 'app', 'desktop', 'src-tauri', 'binaries', 'cutd'), 'ignored\n')
  writeFileSync(join(root, 'app', 'desktop', 'src-tauri', 'gen', 'schemas', 'desktop.json'), 'ignored\n')
  writeFileSync(join(root, 'app', 'perception', 'py', '__pycache__', 'runner.pyc'), 'ignored\n')
  writeFileSync(join(root, 'docs', 'private', 'README.md'), 'private local notes\n')
  writeFileSync(join(root, 'testdata', 'talking_head.mp4'), 'generated media\n')
  writeFileSync(join(root, 'testdata', 'real', 'intro.png'), 'governed external media\n')
  writeFileSync(join(root, 'ui', 'public-tests', '__release__', 'surface.png'), 'ignored\n')
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
})

test('source manifest changes when synchronized source bytes change', (t) => {
  const root = fixture(t)
  const first = sourceContentManifest(root)
  writeFileSync(join(root, 'ui', 'source.txt'), 'changed\n')
  const second = sourceContentManifest(root)
  assert.notEqual(second.sha256, first.sha256)
  assert.equal(second.files, first.files)
})
