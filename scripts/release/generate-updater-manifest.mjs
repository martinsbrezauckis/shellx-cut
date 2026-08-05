#!/usr/bin/env node
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildUpdaterManifest } from '../lib/updater-manifest.mjs'
import { collectSourceIdentity } from '../lib/ignored-test-rig.mjs'
import { sourceContentManifest } from '../lib/source-content-manifest.mjs'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')

function parseArgs(argv) {
  const parsed = {}
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (!arg?.startsWith('--')) continue
    const next = argv[index + 1]
    if (!next || next.startsWith('--')) parsed[arg.slice(2)] = true
    else {
      parsed[arg.slice(2)] = next
      index += 1
    }
  }
  return parsed
}

function option(parsed, name) {
  const value = parsed[name]
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}

function expandHome(path) {
  if (path === '~') return homedir()
  if (path.startsWith('~/')) return join(homedir(), path.slice(2))
  return path
}

function configuredVersion() {
  const config = JSON.parse(readFileSync(join(ROOT, 'app/desktop/src-tauri/tauri.conf.json'), 'utf8'))
  if (typeof config.version !== 'string') throw new Error('Tauri config has no release version')
  return config.version
}

function configuredPublicKey() {
  const config = JSON.parse(readFileSync(join(ROOT, 'app/desktop/src-tauri/tauri.conf.json'), 'utf8'))
  const publicKey = config.plugins?.updater?.pubkey
  if (typeof publicKey !== 'string' || publicKey.trim().length < 40) {
    throw new Error('Tauri config has no updater public key')
  }
  return publicKey.trim()
}

function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

function verifyUpdaterSignature(artifactPath, signaturePath) {
  const result = spawnSync('cargo', [
    'run',
    '--quiet',
    '--manifest-path',
    join(ROOT, 'app/desktop/src-tauri/Cargo.toml'),
    '--example',
    'verify-updater-signature',
    '--',
    '--public-key',
    configuredPublicKey(),
    '--artifact',
    resolve(artifactPath),
    '--signature',
    resolve(signaturePath),
  ], {
    cwd: ROOT,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || 'unknown verification failure').trim()
    throw new Error(`Updater signature does not match ${artifactPath}: ${detail}`)
  }
}

export function buildUpdaterVerificationReceipt({
  options,
  result,
  source,
  sourceContent,
  generatedAt = new Date().toISOString(),
}) {
  return {
    schema: 'shellx-cut/updater-manifest-verify@1',
    generatedAt,
    status: 'pass',
    source: {
      gitCommit: source.gitCommit,
      gitDirty: false,
      version: source.version,
      cargoLockSha256: source.cargoLock.sha256,
      contentManifestSha256: sourceContent.sha256,
    },
    release: {
      repository: options.repo,
      tag: options.tag,
      version: options.version,
    },
    manifest: {
      name: options.output.split(/[\\/]/).pop(),
      bytes: statSync(options.output).size,
      sha256: sha256File(options.output),
      platforms: Object.keys(result.manifest.platforms).sort(),
    },
    artifacts: result.verifiedArtifacts,
    checks: [
      'artifact-minisign-verified-against-embedded-pubkey',
      'all-required-platforms-present',
      'release-url-version-bound',
    ],
  }
}

export function optionsFromArgv(argv) {
  const parsed = parseArgs(argv)
  const version = option(parsed, 'version') ?? configuredVersion()
  const artifactRoot = resolve(expandHome(
    option(parsed, 'artifact-root') ?? `~/shellx-cut-builds/v${version}`,
  ))
  const requiredPlatforms = (option(parsed, 'platforms') ?? 'windows-x86_64,darwin-aarch64')
    .split(',')
    .map((platform) => platform.trim())
    .filter(Boolean)
  return {
    version,
    artifactRoot,
    repo: option(parsed, 'repo') ?? 'martinsbrezauckis/shellx-cut',
    tag: option(parsed, 'tag') ?? `v${version}`,
    baseUrl: option(parsed, 'base-url'),
    pubDate: option(parsed, 'pub-date') ?? new Date().toISOString(),
    notes: option(parsed, 'notes') ?? `See ShellX Cut v${version} release notes on GitHub.`,
    requiredPlatforms,
    output: resolve(expandHome(
      option(parsed, 'output') ?? join(artifactRoot, 'latest.json'),
    )),
    receipt: resolve(expandHome(
      option(parsed, 'receipt') ?? join(artifactRoot, 'updater-manifest-verify.json'),
    )),
  }
}

export function generateUpdaterManifest(argv) {
  const options = optionsFromArgv(argv)
  if (existsSync(options.output)) throw new Error(`Refusing to overwrite updater manifest: ${options.output}`)
  if (existsSync(options.receipt)) throw new Error(`Refusing to overwrite updater receipt: ${options.receipt}`)
  const source = collectSourceIdentity(ROOT)
  if (source.gitDirty) throw new Error('Updater release receipts require a clean product worktree')
  if (source.version !== options.version) {
    throw new Error(`Updater version ${options.version} does not match source ${source.version}`)
  }
  const sourceContent = sourceContentManifest(ROOT)
  const result = buildUpdaterManifest({ ...options, verifySignature: verifyUpdaterSignature })
  mkdirSync(dirname(options.output), { recursive: true })
  writeFileSync(options.output, `${JSON.stringify(result.manifest, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' })
  const receipt = buildUpdaterVerificationReceipt({ options, result, source, sourceContent })
  mkdirSync(dirname(options.receipt), { recursive: true })
  writeFileSync(options.receipt, `${JSON.stringify(receipt, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' })
  return { options, result, receipt }
}

function main() {
  const { options, result } = generateUpdaterManifest(process.argv.slice(2))
  console.log(`wrote ${options.output}`)
  console.log(`wrote ${options.receipt}`)
  for (const item of result.included) console.log(`included ${item}`)
  for (const item of result.skipped) console.log(`skipped ${item}`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exit(1)
  }
}
