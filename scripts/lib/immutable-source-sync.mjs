import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { isAbsolute, join, resolve } from 'node:path'
import { sourceContentManifest } from './source-content-manifest.mjs'

const SYNC_INCLUDE = [
  '.gitignore', 'LICENSE', 'NOTICE', 'README.md', 'SECURITY.md', 'START_HERE_FOR_AGENT.txt',
  '.github/***', 'app/***', 'docs/***', 'schema/***', 'scripts/***', 'skill/***', 'testdata/***', 'ui/***',
]
const SYNC_EXCLUDE = [
  '.git/***', 'app/target/***', 'app/desktop/src-tauri/target/***', 'ui/dist/***', 'ui/node_modules/***',
]

function command(commandName, args, options = {}) {
  const result = spawnSync(commandName, args, {
    cwd: options.cwd,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  if (result.error) throw result.error
  if (result.status !== 0) {
    throw new Error(
      `${commandName} ${args.join(' ')} failed: ${String(result.stderr || result.stdout || '').trim()}`,
    )
  }
  return String(result.stdout || '').trim()
}

function gitText(repoRoot, args) {
  return command('git', args, { cwd: repoRoot })
}

export function materializeImmutableGitSource(repoRoot, { tempRoot = tmpdir() } = {}) {
  const root = resolve(repoRoot)
  const sourceCommit = gitText(root, ['rev-parse', 'HEAD'])
  const sourceDirty = Boolean(gitText(root, ['status', '--porcelain']))
  const objectRoot = mkdtempSync(join(tempRoot, 'shellx-cut-source-object-'))
  const archivePath = join(objectRoot, 'source.tar')
  const sourceDir = join(objectRoot, 'tree')
  try {
    mkdirSync(sourceDir, { mode: 0o700 })
    command('git', ['archive', '--format=tar', '--output', archivePath, sourceCommit], { cwd: root })
    command('tar', ['-xf', archivePath, '-C', sourceDir])
  } catch (error) {
    rmSync(objectRoot, { recursive: true, force: true })
    throw error
  }
  return {
    sourceCommit,
    sourceDirty,
    sourceDir,
    dispose() {
      rmSync(objectRoot, { recursive: true, force: true })
    },
  }
}

export function assertSourceContentIdentity(expected, observed, label = 'remote') {
  if (!expected || !observed || expected !== observed) {
    throw new Error(
      `immutable source identity mismatch: expected=${expected || 'missing'} ${label}=${observed || 'missing'}`,
    )
  }
  return expected
}

export function resolveImmutableSourceIdentityRoot({ repoRoot, configured = '', testControl = false }) {
  if (!configured) return resolve(repoRoot)
  if (!testControl || !isAbsolute(configured)) {
    throw new Error('a separate immutable source identity root requires test-control and an absolute path')
  }
  return resolve(configured)
}

export function prepareImmutableSourceSync({ repoRoot, host, remoteDir }) {
  const source = materializeImmutableGitSource(repoRoot)
  try {
    const manifest = sourceContentManifest(source.sourceDir)
    return {
      sourceCommit: source.sourceCommit,
      sourceDirty: source.sourceDirty,
      sourceContentManifestSha256: manifest.sha256,
      rsyncArgs: [
        '-az', '--delete',
        ...SYNC_EXCLUDE.flatMap((pattern) => ['--exclude', pattern]),
        ...SYNC_INCLUDE.flatMap((pattern) => ['--include', pattern]),
        '--exclude', '*', `${source.sourceDir}/`, `${host}:${remoteDir}/`,
      ],
      dispose: source.dispose,
    }
  } catch (error) {
    source.dispose()
    throw error
  }
}

export const IMMUTABLE_SOURCE_IDENTITY_SHELL = String.raw`
REMOTE_SOURCE_CONTENT_MANIFEST_SHA256="$(node scripts/source-content-manifest.mjs --out "$WDIO_OUT_RESOLVED/source-content-manifest.json" --sha256)"
if [ "$REMOTE_SOURCE_CONTENT_MANIFEST_SHA256" != "$SOURCE_CONTENT_MANIFEST_SHA256" ]; then
  echo "immutable source identity mismatch: commit=$SOURCE_COMMIT expected=$SOURCE_CONTENT_MANIFEST_SHA256 remote=$REMOTE_SOURCE_CONTENT_MANIFEST_SHA256" >&2
  exit 1
fi
SOURCE_CONTENT_MANIFEST_SHA256="$REMOTE_SOURCE_CONTENT_MANIFEST_SHA256"`
