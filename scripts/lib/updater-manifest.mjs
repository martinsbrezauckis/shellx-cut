import { createHash } from 'node:crypto'
import { existsSync, readFileSync, statSync } from 'node:fs'
import { basename, join } from 'node:path'
import { normalizeTauriUpdaterSignature } from './tauri-updater-signature.mjs'

const DEFAULT_REPO = 'martinsbrezauckis/shellx-cut'
export const ARTIFACT_IDENTITY_SCHEMA = 'shellx-cut/updater-artifact-identity@1'
const VERSION_RX = /^\d+\.\d+\.\d+$/
const SHA256_RX = /^[a-f0-9]{64}$/

function firstExisting(root, names) {
  for (const name of names) {
    const path = join(root, name)
    if (existsSync(path)) return path
  }
  return null
}

function artifactUrl(baseUrl, artifactPath) {
  // GitHub renames uploaded release assets: spaces become dots (verified on
  // the live v0.6.105 draft — "ShellX Cut_…" is served as "ShellX.Cut_…").
  // The manifest must point at the name GitHub actually serves; a
  // percent-encoded space would 404 the updater on every installed build.
  const githubAssetName = basename(artifactPath).replace(/ /g, '.')
  return `${baseUrl.replace(/\/$/, '')}/${encodeURIComponent(githubAssetName)}`
}

function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

export function updaterArtifactIdentityText({ version, platform, sha256 }) {
  if (!VERSION_RX.test(version)) throw new Error(`Invalid updater artifact identity version: ${version}`)
  if (typeof platform !== 'string' || !platform) throw new Error('Updater artifact identity platform is required')
  if (!SHA256_RX.test(sha256)) throw new Error('Updater artifact identity SHA-256 must be lowercase hexadecimal')
  return [
    ARTIFACT_IDENTITY_SCHEMA,
    `version=${version}`,
    `platform=${platform}`,
    `sha256=${sha256}`,
    '',
  ].join('\n')
}

function parseArtifactIdentity(text, platform, version, artifactSha256) {
  const lines = text.split('\n')
  if (lines.length !== 5 || lines[4] !== '') {
    throw new Error(`${platform} artifact identity has an invalid canonical format`)
  }
  if (lines[0] !== ARTIFACT_IDENTITY_SCHEMA) {
    throw new Error(`${platform} artifact identity has an unknown schema`)
  }
  const fields = Object.fromEntries(lines.slice(1, 4).map((line) => line.split('=')))
  if (fields.version !== version) {
    throw new Error(`${platform} artifact identity version must match ${version}`)
  }
  if (fields.platform !== platform) {
    throw new Error(`${platform} artifact identity platform must match ${platform}`)
  }
  if (fields.sha256 !== artifactSha256) {
    throw new Error(`${platform} artifact identity SHA-256 must match the artifact bytes`)
  }
  const canonical = updaterArtifactIdentityText({ version, platform, sha256: artifactSha256 })
  if (text !== canonical) {
    throw new Error(`${platform} artifact identity is not canonical`)
  }
  return fields
}

export function updaterCandidates(version) {
  return [
    {
      platform: 'windows-x86_64',
      artifactNames: [
        `windows/ShellX Cut_${version}_x64-setup.exe`,
        `ShellX Cut_${version}_x64-setup.exe`,
      ],
      label: 'Windows NSIS updater installer',
    },
    {
      platform: 'darwin-aarch64',
      artifactNames: [
        'macos/ShellX Cut.app.tar.gz',
        'ShellX Cut.app.tar.gz',
      ],
      label: 'macOS Tauri updater archive',
    },
  ]
}

export function buildUpdaterManifest({
  version,
  artifactRoot,
  repo = DEFAULT_REPO,
  tag = `v${version}`,
  baseUrl,
  pubDate,
  notes,
  requiredPlatforms = ['windows-x86_64', 'darwin-aarch64'],
  verifySignature,
}) {
  if (!VERSION_RX.test(version)) {
    throw new Error(`Invalid release version: ${version}`)
  }
  if (tag !== `v${version}`) {
    throw new Error(`Updater tag must be v${version}; received ${tag}`)
  }
  if (typeof verifySignature !== 'function') {
    throw new Error('Updater manifest generation requires cryptographic signature verification')
  }

  const candidates = updaterCandidates(version)
  const supported = new Set(candidates.map((candidate) => candidate.platform))
  const unknownRequired = requiredPlatforms.filter((platform) => !supported.has(platform))
  if (unknownRequired.length > 0) {
    throw new Error(`Unknown required updater platform(s): ${unknownRequired.join(', ')}`)
  }

  const releaseBase = baseUrl ?? `https://github.com/${repo}/releases/download/${tag}`
  if (!new URL(releaseBase).pathname.includes(`/v${version}`)) {
    throw new Error(`Updater base URL must be bound to release v${version}`)
  }

  const platforms = {}
  const included = []
  const skipped = []
  const verifiedArtifacts = []
  const artifactIdentities = {}
  for (const candidate of candidates) {
    const artifact = firstExisting(artifactRoot, candidate.artifactNames)
    if (!artifact) {
      skipped.push(`${candidate.platform}: missing ${candidate.label}`)
      continue
    }
    const signaturePath = `${artifact}.sig`
    if (!existsSync(signaturePath)) {
      skipped.push(`${candidate.platform}: missing ${signaturePath}`)
      continue
    }
    const identityPath = `${artifact}.identity`
    const identitySignaturePath = `${identityPath}.sig`
    if (!existsSync(identityPath) || !existsSync(identitySignaturePath)) {
      skipped.push(`${candidate.platform}: missing signed artifact identity`)
      continue
    }
    verifySignature(artifact, signaturePath)
    verifySignature(identityPath, identitySignaturePath)
    const artifactSha256 = sha256File(artifact)
    const identity = parseArtifactIdentity(
      readFileSync(identityPath, 'utf8'),
      candidate.platform,
      version,
      artifactSha256,
    )
    const url = artifactUrl(releaseBase, artifact)
    if (!new URL(url).pathname.includes(`/v${version}/`)) {
      throw new Error(`${candidate.platform} updater URL is not bound to v${version}`)
    }
    platforms[candidate.platform] = {
      signature: normalizeTauriUpdaterSignature(readFileSync(signaturePath, 'utf8')),
      url,
    }
    artifactIdentities[candidate.platform] = {
      version: identity.version,
      sha256: identity.sha256,
      signature: normalizeTauriUpdaterSignature(readFileSync(identitySignaturePath, 'utf8'), `${candidate.platform} artifact identity signature`),
    }
    verifiedArtifacts.push({
      platform: candidate.platform,
      name: basename(artifact),
      bytes: statSync(artifact).size,
      sha256: artifactSha256,
      signatureName: basename(signaturePath),
      signatureBytes: statSync(signaturePath).size,
      signatureSha256: sha256File(signaturePath),
      signatureVerified: true,
      identityName: basename(identityPath),
      identityBytes: statSync(identityPath).size,
      identitySha256: sha256File(identityPath),
      identitySignatureName: basename(identitySignaturePath),
      identitySignatureBytes: statSync(identitySignaturePath).size,
      identitySignatureSha256: sha256File(identitySignaturePath),
      identitySignatureVerified: true,
      url,
    })
    included.push(`${candidate.platform}: ${basename(artifact)}`)
  }

  const missing = requiredPlatforms.filter((platform) => !platforms[platform])
  if (missing.length > 0) {
    throw new Error(
      `Missing required verified updater platform(s): ${missing.join(', ')}. ${skipped.join('; ')}`,
    )
  }

  return {
    manifest: {
      version,
      notes,
      pub_date: pubDate,
      platforms,
      shellx_cut_artifact_identities: {
        schema: ARTIFACT_IDENTITY_SCHEMA,
        platforms: artifactIdentities,
      },
    },
    included,
    skipped,
    verifiedArtifacts,
  }
}
