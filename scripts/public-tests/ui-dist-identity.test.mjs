import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import {
  UI_DIST_IDENTITY_FILE,
  UI_DIST_IDENTITY_GUARD,
  checkUiDistIdentity,
  writeUiDistIdentity,
} from '../lib/ui-dist-identity.mjs'

function write(root, path, content = `${path}\n`) {
  mkdirSync(join(root, path, '..'), { recursive: true })
  writeFileSync(join(root, path), content)
}

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'cut-ui-dist-identity-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  for (const path of ['.gitignore', 'LICENSE', 'NOTICE', 'README.md', 'SECURITY.md', 'START_HERE_FOR_AGENT.txt', 'testdata/test_lut_invert.cube']) write(root, path)
  for (const directory of ['.github', 'app', 'docs', 'schema', 'scripts', 'skill', 'ui']) write(root, `${directory}/source.txt`)
  write(root, 'app/Cargo.toml', '[workspace.package]\nversion = "2.4.6"\n')
  write(root, 'schema/verbs.json', JSON.stringify({ schema: 'fixture', verbs: [] }))
  write(root, 'ui/package.json', JSON.stringify({ version: '2.4.6' }))
  write(root, 'ui/package-lock.json', JSON.stringify({ version: '2.4.6', packages: { '': { version: '2.4.6' } } }))
  write(root, 'ui/dist/index.html', '<!doctype html>\n')
  return root
}

test('UI dist identity binds the built frontend to current backend, schema, package, lock, and whole source manifest', (t) => {
  const root = fixture(t)
  const written = writeUiDistIdentity({ repoRoot: root })
  assert.equal(written.version, '2.4.6')
  assert.equal(written.source.content_manifest.sha256.length, 64)
  assert.equal(checkUiDistIdentity({ repoRoot: root }).source.verb_schema_sha256, written.source.verb_schema_sha256)
})

test('UI dist identity rejects a stale schema with the exact rebuild diagnostic', (t) => {
  const root = fixture(t)
  writeUiDistIdentity({ repoRoot: root })
  writeFileSync(join(root, 'schema/verbs.json'), JSON.stringify({ schema: 'changed', verbs: [] }))
  assert.throws(
    () => checkUiDistIdentity({ repoRoot: root }),
    new RegExp(`${UI_DIST_IDENTITY_GUARD}: schema/verbs\\.json sha256 mismatch \\(built "[a-f0-9]{64}", current "[a-f0-9]{64}"\\); rebuild with npm --prefix ui run build`),
  )
})

test('UI dist identity refuses output without its generated sidecar', (t) => {
  const root = fixture(t)
  assert.throws(
    () => checkUiDistIdentity({ repoRoot: root }),
    new RegExp(`${UI_DIST_IDENTITY_GUARD}: ui/dist is missing ${UI_DIST_IDENTITY_FILE}; rebuild with npm --prefix ui run build`),
  )
})

test('developer and release UI consumers rebuild or verify an identity-bound dist', () => {
  const root = join(import.meta.dirname, '..', '..')
  const read = (path) => readFileSync(join(root, path), 'utf8')
  for (const path of ['scripts/build-linux.sh', 'scripts/build-macos.sh', 'scripts/build-windows.sh']) {
    assert.match(read(path), /npm run build/, `${path} must rebuild ui\/dist instead of trusting an inherited bundle`)
  }
  const developerLaunch = read('scripts/dev.sh')
  assert.match(developerLaunch, /npm run build && npm run check:dist-identity/, 'scripts/dev.sh must verify the newly built identity before cutd serves ui/dist')
  for (const path of ['scripts/release/dual-surface-job-gate.mjs', 'scripts/release/full-coverage-gate.mjs']) {
    const source = read(path)
    assert.match(source, /checkUiDistIdentity/, `${path} must refuse a stale existing ui\/dist`)
    assert.match(source, /BUILD-UI-IDENTITY-01|error\.message/, `${path} must preserve the identity guard diagnostic`)
  }
})
