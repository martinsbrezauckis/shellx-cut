import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import { RELEASE_DOC_TRUTH_GUARD, checkReleaseDocTruth } from '../lib/release-doc-truth.mjs'

const truth = {
  schema: 'shellx-cut/release-truth@1',
  version: '2.4.6',
  status: 'candidate',
  published_version: '2.4.5',
}
const marker = '<!-- shellx-cut-release-truth: candidate; version=2.4.6; published=2.4.5 -->'

function write(root, path, content) {
  mkdirSync(join(root, path, '..'), { recursive: true })
  writeFileSync(join(root, path), content)
}

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'cut-release-doc-truth-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  write(root, 'app/Cargo.toml', '[workspace]\nmembers = ["core"]\n\n[workspace.package]\nversion = "2.4.6"\n')
  write(root, 'app/core/Cargo.toml', '[package]\nname = "cut-core"\nversion.workspace = true\n')
  write(root, 'app/Cargo.lock', 'version = 4\n\n[[package]]\nname = "cut-core"\nversion = "2.4.6"\n')
  write(root, 'app/desktop/src-tauri/Cargo.toml', '[package]\nname = "shellx-cut"\nversion = "2.4.6"\n')
  write(root, 'app/desktop/src-tauri/Cargo.lock', 'version = 4\n\n[[package]]\nname = "shellx-cut"\nversion = "2.4.6"\n')
  write(root, 'app/desktop/src-tauri/tauri.conf.json', JSON.stringify({ version: '2.4.6' }))
  write(root, 'ui/package.json', JSON.stringify({ version: '2.4.6' }))
  write(root, 'ui/package-lock.json', JSON.stringify({ version: '2.4.6', packages: { '': { version: '2.4.6' } } }))
  write(root, 'schema/verbs/base.json', JSON.stringify({ root: { release_truth: truth } }))
  write(root, 'schema/verbs.json', JSON.stringify({ release_truth: truth }))
  write(root, 'README.md', `${marker}\nSTATUS — v2.4.6 candidate; v2.4.5 is the latest published release\n`)
  write(root, 'SECURITY.md', marker)
  write(root, 'docs/public/FEATURES.md', `${marker}\n## v2.4.6 candidate\nv2.4.5 remains the latest published release\n`)
  write(root, 'docs/public/DEBUG_API.md', marker)
  write(root, 'docs/public/shellx-cut-threat-model.md', marker)
  write(root, 'skill/shellx-cut/SKILL.md', marker)
  write(root, 'skill/shellx-cut/reference.md', marker)
  write(root, 'docs/public/site/manual/cut/index.html', '<span data-app-version="2.4.6" data-release-status="candidate" data-published-version="2.4.5">2.4.6</span>')
  write(root, 'ui/src/manual/content.generated.json', JSON.stringify({ appVersion: '2.4.6' }))
  return root
}

test('release-document truth keeps candidate and published state distinct without a tree hash', (t) => {
  const root = fixture(t)
  assert.deepEqual(checkReleaseDocTruth({ repoRoot: root }), {
    version: '2.4.6', status: 'candidate', publishedVersion: '2.4.5',
  })
})

test('release-document truth rejects a mismatched package-lock fixture with an exact diagnostic', (t) => {
  const root = fixture(t)
  writeFileSync(join(root, 'ui/package-lock.json'), JSON.stringify({ version: '2.4.5', packages: { '': { version: '2.4.5' } } }))
  assert.throws(
    () => checkReleaseDocTruth({ repoRoot: root }),
    new RegExp(`${RELEASE_DOC_TRUTH_GUARD}: ui/package-lock\\.json version is "2\\.4\\.5"; expected 2\\.4\\.6`),
  )
})

test('release-document truth rejects a document that presents a candidate as published', (t) => {
  const root = fixture(t)
  writeFileSync(join(root, 'README.md'), `${marker}\nSTATUS — v2.4.6 release\n`)
  assert.throws(
    () => checkReleaseDocTruth({ repoRoot: root }),
    new RegExp(`${RELEASE_DOC_TRUTH_GUARD}: README\\.md must state the current candidate/published truth`),
  )
})
