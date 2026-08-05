import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { buildUpdaterManifest } from '../lib/updater-manifest.mjs'
import { buildUpdaterVerificationReceipt } from '../release/generate-updater-manifest.mjs'

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'shellx-cut-updater-manifest-'))
  mkdirSync(join(root, 'windows'), { recursive: true })
  mkdirSync(join(root, 'macos'), { recursive: true })
  const windows = join(root, 'windows', 'ShellX Cut_0.6.105_x64-setup.exe')
  const macos = join(root, 'macos', 'ShellX Cut.app.tar.gz')
  writeFileSync(windows, 'windows artifact')
  writeFileSync(`${windows}.sig`, 'windows-signature\n')
  writeFileSync(macos, 'macOS artifact')
  writeFileSync(`${macos}.sig`, 'macos-signature\n')
  return { root, windows, macos }
}

function options(root, verifySignature = () => {}) {
  return {
    version: '0.6.105',
    artifactRoot: root,
    repo: 'martinsbrezauckis/shellx-cut',
    tag: 'v0.6.105',
    pubDate: '2026-08-01T00:00:00.000Z',
    notes: 'Release notes',
    verifySignature,
  }
}

test('manifest includes both release platforms and version-bound GitHub URLs', () => {
  const { root } = fixture()
  try {
    const verified = []
    const { manifest } = buildUpdaterManifest(options(root, (artifact, signature) => {
      verified.push([artifact, signature])
    }))
    assert.equal(manifest.version, '0.6.105')
    assert.deepEqual(Object.keys(manifest.platforms).sort(), ['darwin-aarch64', 'windows-x86_64'])
    assert.equal(verified.length, 2)
    assert.equal(manifest.platforms['windows-x86_64'].signature, 'windows-signature')
    assert.match(manifest.platforms['windows-x86_64'].url, /\/releases\/download\/v0\.6\.105\//)
    // GitHub serves release assets with spaces converted to dots; the manifest
    // must carry that served name, never a percent-encoded space.
    assert.match(manifest.platforms['windows-x86_64'].url, /\/ShellX\.Cut_0\.6\.105_x64-setup\.exe$/)
    assert.doesNotMatch(manifest.platforms['windows-x86_64'].url, /%20/)
    assert.match(manifest.platforms['darwin-aarch64'].url, /\/ShellX\.Cut\.app\.tar\.gz$/)
    assert.equal(manifest.platforms['darwin-aarch64'].signature, 'macos-signature')
    assert.deepEqual(
      buildUpdaterManifest(options(root, () => {})).verifiedArtifacts.map((item) => item.platform).sort(),
      ['darwin-aarch64', 'windows-x86_64'],
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('missing required artifact or signature fails closed', () => {
  const { root, macos } = fixture()
  try {
    rmSync(`${macos}.sig`)
    assert.throws(
      () => buildUpdaterManifest(options(root)),
      /Missing required verified updater platform\(s\): darwin-aarch64/,
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('signature verification failure aborts manifest generation', () => {
  const { root } = fixture()
  try {
    assert.throws(
      () => buildUpdaterManifest(options(root, () => { throw new Error('signature mismatch') })),
      /signature mismatch/,
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('manifest rejects a release tag or base URL that is not bound to its version', () => {
  const { root } = fixture()
  try {
    assert.throws(
      () => buildUpdaterManifest({ ...options(root), tag: 'latest' }),
      /Updater tag must be v0\.6\.105/,
    )
    assert.throws(
      () => buildUpdaterManifest({
        ...options(root),
        baseUrl: 'https://github.com/martinsbrezauckis/shellx-cut/releases/download/v0.6.104',
      }),
      /base URL must be bound to release v0\.6\.105/,
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('private verification receipt binds clean source, manifest, and both signed artifacts', () => {
  const { root } = fixture()
  try {
    const result = buildUpdaterManifest(options(root, () => {}))
    const output = join(root, 'latest.json')
    writeFileSync(output, `${JSON.stringify(result.manifest)}\n`)
    const receipt = buildUpdaterVerificationReceipt({
      options: {
        version: '0.6.105',
        repo: 'martinsbrezauckis/shellx-cut',
        tag: 'v0.6.105',
        output,
      },
      result,
      source: {
        gitCommit: 'a'.repeat(40),
        gitDirty: false,
        version: '0.6.105',
        cargoLock: { sha256: 'b'.repeat(64) },
      },
      sourceContent: { sha256: 'c'.repeat(64) },
      generatedAt: '2026-08-01T00:00:00.000Z',
    })
    assert.equal(receipt.schema, 'shellx-cut/updater-manifest-verify@1')
    assert.equal(receipt.source.gitCommit, 'a'.repeat(40))
    assert.deepEqual(receipt.manifest.platforms, ['darwin-aarch64', 'windows-x86_64'])
    assert.equal(receipt.artifacts.length, 2)
    assert.ok(receipt.artifacts.every((item) => item.signatureVerified === true))
    assert.deepEqual(receipt.checks, [
      'artifact-minisign-verified-against-embedded-pubkey',
      'all-required-platforms-present',
      'release-url-version-bound',
    ])
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
