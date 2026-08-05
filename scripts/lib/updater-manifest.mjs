import { createHash } from 'node:crypto'
import { existsSync, readFileSync, statSync } from 'node:fs'
import { basename, join } from 'node:path'

const DEFAULT_REPO = 'martinsbrezauckis/shellx-cut'

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
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
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
    verifySignature(artifact, signaturePath)
    const url = artifactUrl(releaseBase, artifact)
    if (!new URL(url).pathname.includes(`/v${version}/`)) {
      throw new Error(`${candidate.platform} updater URL is not bound to v${version}`)
    }
    platforms[candidate.platform] = {
      signature: readFileSync(signaturePath, 'utf8').trim(),
      url,
    }
    verifiedArtifacts.push({
      platform: candidate.platform,
      name: basename(artifact),
      bytes: statSync(artifact).size,
      sha256: sha256File(artifact),
      signatureName: basename(signaturePath),
      signatureBytes: statSync(signaturePath).size,
      signatureSha256: sha256File(signaturePath),
      signatureVerified: true,
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
    },
    included,
    skipped,
    verifiedArtifacts,
  }
}
