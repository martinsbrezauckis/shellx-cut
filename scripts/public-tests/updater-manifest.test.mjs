import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { buildUpdaterManifest } from '../lib/updater-manifest.mjs'
import { buildUpdaterVerificationReceipt, updaterVerifierEnvironment } from '../release/generate-updater-manifest.mjs'

const ARTIFACT_IDENTITY_SCHEMA = 'shellx-cut/updater-artifact-identity@1'
const WINDOWS_SIGNATURE = Buffer.from('windows-signature').toString('base64')
const MACOS_SIGNATURE = Buffer.from('macos-signature').toString('base64')
const ACTUAL_TAURI_IDENTITY_SIGNATURE = 'dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSd005M21KaFd5WW1vaFVHSVFGb0cwajFLUlJ0NWNZc1dxc0ludThsbXU5NjR2QzBFcGhsellOVTVJVjZRYnZqTnl6RXNRMUtSSUNmSHc4b2lXZnd4WjJuRWRWN1Ayc3dFPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzg2NTM4MTE3CWZpbGU6U2hlbGxYIEN1dF8wLjYuMTA5X3g2NC1zZXR1cC5leGUuaWRlbnRpdHkKczJick8wSGZnUERnc05UekVuRWlKVmFKM1BSUWhnRnBFa1cvUnNRUnBHY2xwUEV0Wk9FL0ZzNnFlSWduTUlEeWRXRDVwYy9VM3BGTk1QYS9lYlNhQ2c9PQo='

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

function writeArtifactIdentity({ artifact, platform, version }) {
  const identity = `${artifact}.identity`
  writeFileSync(identity, [
    ARTIFACT_IDENTITY_SCHEMA,
    `version=${version}`,
    `platform=${platform}`,
    `sha256=${sha256(artifact)}`,
    '',
  ].join('\n'))
  writeFileSync(`${identity}.sig`, `${Buffer.from(`${platform}-identity-signature`).toString('base64')}\n`)
  return identity
}

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'shellx-cut-updater-manifest-'))
  mkdirSync(join(root, 'windows'), { recursive: true })
  mkdirSync(join(root, 'macos'), { recursive: true })
  const windows = join(root, 'windows', 'ShellX Cut_0.6.105_x64-setup.exe')
  const macos = join(root, 'macos', 'ShellX Cut.app.tar.gz')
  writeFileSync(windows, 'windows artifact')
  writeFileSync(`${windows}.sig`, `${WINDOWS_SIGNATURE}\n`)
  writeFileSync(macos, 'macOS artifact')
  writeFileSync(`${macos}.sig`, `${MACOS_SIGNATURE}\n`)
  writeArtifactIdentity({ artifact: windows, platform: 'windows-x86_64', version: '0.6.105' })
  writeArtifactIdentity({ artifact: macos, platform: 'darwin-aarch64', version: '0.6.105' })
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
    assert.equal(verified.length, 4)
    assert.equal(manifest.platforms['windows-x86_64'].signature, WINDOWS_SIGNATURE)
    assert.match(manifest.platforms['windows-x86_64'].url, /\/releases\/download\/v0\.6\.105\//)
    // GitHub serves release assets with spaces converted to dots; the manifest
    // must carry that served name, never a percent-encoded space.
    assert.match(manifest.platforms['windows-x86_64'].url, /\/ShellX\.Cut_0\.6\.105_x64-setup\.exe$/)
    assert.doesNotMatch(manifest.platforms['windows-x86_64'].url, /%20/)
    assert.match(manifest.platforms['darwin-aarch64'].url, /\/ShellX\.Cut\.app\.tar\.gz$/)
    assert.equal(manifest.platforms['darwin-aarch64'].signature, MACOS_SIGNATURE)
    assert.deepEqual(
      buildUpdaterManifest(options(root, () => {})).verifiedArtifacts.map((item) => item.platform).sort(),
      ['darwin-aarch64', 'windows-x86_64'],
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('manifest extracts the labelled payload from actual multiline cargo-tauri signer output', () => {
  const { root, windows } = fixture()
  try {
    writeFileSync(`${windows}.identity.sig`, [
      'Your file was signed successfully, You can find the signature here:',
      'C:\\build\\ShellX Cut_0.6.109_x64-setup.exe.identity.sig',
      '',
      'Public signature:',
      ACTUAL_TAURI_IDENTITY_SIGNATURE,
      '',
      'Make sure to include this into the signature field of your update server.',
      '',
    ].join('\n'))
    const { manifest } = buildUpdaterManifest(options(root))
    assert.equal(
      manifest.shellx_cut_artifact_identities.platforms['windows-x86_64'].signature,
      ACTUAL_TAURI_IDENTITY_SIGNATURE,
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('manifest binds every advertised platform version to signed artifact bytes', () => {
  const { root, windows, macos } = fixture()
  try {
    const { manifest } = buildUpdaterManifest(options(root))
    assert.deepEqual(manifest.shellx_cut_artifact_identities, {
      schema: ARTIFACT_IDENTITY_SCHEMA,
      platforms: {
        'windows-x86_64': {
          version: '0.6.105',
          sha256: sha256(windows),
          signature: Buffer.from('windows-x86_64-identity-signature').toString('base64'),
        },
        'darwin-aarch64': {
          version: '0.6.105',
          sha256: sha256(macos),
          signature: Buffer.from('darwin-aarch64-identity-signature').toString('base64'),
        },
      },
    })
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('an older signed artifact identity cannot masquerade as a newer release', () => {
  const { root, windows } = fixture()
  try {
    writeArtifactIdentity({ artifact: windows, platform: 'windows-x86_64', version: '0.6.104' })
    assert.throws(
      () => buildUpdaterManifest(options(root)),
      /windows-x86_64 artifact identity version must match 0[.]6[.]105/,
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

test('missing signed artifact identity fails closed', () => {
  const { root, windows } = fixture()
  try {
    rmSync(`${windows}.identity.sig`)
    assert.throws(
      () => buildUpdaterManifest(options(root)),
      /Missing required verified updater platform\(s\): windows-x86_64/,
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

test('artifact verification uses no updater private key or host external sidecar', () => {
  const env = updaterVerifierEnvironment({
    PATH: '/safe/path',
    TAURI_SIGNING_PRIVATE_KEY: 'must-not-reach-verifier',
    TAURI_SIGNING_PRIVATE_KEY_PATH: '/private/updater.key',
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'must-not-reach-verifier',
  })
  assert.equal(env.PATH, '/safe/path')
  assert.equal(env.TAURI_CONFIG, '{"bundle":{"externalBin":[]}}')
  assert.equal(Object.hasOwn(env, 'TAURI_SIGNING_PRIVATE_KEY'), false)
  assert.equal(Object.hasOwn(env, 'TAURI_SIGNING_PRIVATE_KEY_PATH'), false)
  assert.equal(Object.hasOwn(env, 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD'), false)
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
