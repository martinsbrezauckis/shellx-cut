import { createHash } from 'node:crypto'
import {
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
  readlinkSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

export const MANIFEST_REL = 'scripts/release/ignored-test-rigs.json'

export function manifestPlatform(platform = process.platform) {
  if (platform === 'darwin') return 'macos'
  if (platform === 'win32') return 'windows'
  return platform
}

function parseJsonFile(path, label) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'))
  } catch (error) {
    throw new Error(`${label} is not valid JSON: ${error.message}`, { cause: error })
  }
}

export function loadIgnoredTestManifest(repoRoot) {
  return parseJsonFile(resolve(repoRoot, MANIFEST_REL), 'ignored-test rig manifest')
}

function rustFiles(dir, out = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'target' || entry.name === '.git') continue
    const path = join(dir, entry.name)
    if (entry.isDirectory()) rustFiles(path, out)
    else if (entry.isFile() && entry.name.endsWith('.rs')) out.push(path)
  }
  return out
}

export function discoverIgnoredRustTests(repoRoot) {
  const found = []
  const pattern = /#\[ignore(?:\s*=\s*"[^"]*")?\]\s*(?:#\[[^\]]+\]\s*)*(?:async\s+)?fn\s+([A-Za-z0-9_]+)/g
  for (const path of rustFiles(resolve(repoRoot, 'app'))) {
    const source = readFileSync(path, 'utf8')
    for (const match of source.matchAll(pattern)) {
      found.push({ rustTest: match[1], source: relative(repoRoot, path).replaceAll('\\', '/') })
    }
  }
  return found.sort((a, b) => a.rustTest.localeCompare(b.rustTest))
}

export function parseIgnoredRigArgs(argv) {
  const out = { id: '', outDir: '', allowDirty: false, list: false }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--id') out.id = argv[++i] ?? ''
    else if (arg === '--out') out.outDir = argv[++i] ?? ''
    else if (arg === '--allow-dirty') out.allowDirty = true
    else if (arg === '--list') out.list = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return out
}

function sha256File(path) {
  const hash = createHash('sha256')
  const buffer = Buffer.alloc(4 * 1024 * 1024)
  const fd = openSync(path, 'r')
  try {
    let bytesRead
    while ((bytesRead = readSync(fd, buffer, 0, buffer.length, null)) > 0) {
      hash.update(buffer.subarray(0, bytesRead))
    }
  } finally {
    closeSync(fd)
  }
  return hash.digest('hex')
}

function treeFiles(root, dir = root, out = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) treeFiles(root, path, out)
    else out.push({ path, relative: relative(root, path).replaceAll('\\', '/'), symlink: entry.isSymbolicLink() })
  }
  return out
}

export function artifactInfo(path, { tree = false } = {}) {
  if (!path || !existsSync(path)) return { path: path || null, exists: false }
  const stat = statSync(path)
  if (stat.isFile()) {
    return { path, exists: true, kind: 'file', bytes: stat.size, sha256: sha256File(path) }
  }
  if (!stat.isDirectory() || !tree) return { path, exists: true, kind: 'directory' }
  const hash = createHash('sha256')
  const files = treeFiles(path)
  let bytes = 0
  for (const file of files) {
    if (file.symlink) {
      hash.update(`link\0${file.relative}\0${readlinkSync(file.path)}\n`)
      continue
    }
    const info = statSync(file.path)
    const digest = sha256File(file.path)
    bytes += info.size
    hash.update(`file\0${file.relative}\0${info.size}\0${digest}\n`)
  }
  return { path, exists: true, kind: 'tree', files: files.length, bytes, sha256: hash.digest('hex') }
}

function git(repoRoot, args) {
  const result = spawnSync('git', args, { cwd: repoRoot, encoding: 'utf8' })
  return result.status === 0 ? result.stdout.trim() : null
}

export function collectSourceIdentity(repoRoot) {
  const tauri = parseJsonFile(
    resolve(repoRoot, 'app/desktop/src-tauri/tauri.conf.json'),
    'Tauri configuration',
  )
  const dirtyText = git(repoRoot, ['status', '--porcelain']) ?? ''
  return {
    version: tauri.version,
    gitCommit: git(repoRoot, ['rev-parse', 'HEAD']),
    gitDirty: dirtyText.length > 0,
    cargoLock: artifactInfo(resolve(repoRoot, 'app/Cargo.lock')),
  }
}

export function rigExecutionEnv(repoRoot, rig, env = process.env, platform = process.platform) {
  const executionEnv = { ...env }
  const defaults = []
  const sidecarRel = 'app/perception/py'
  const needsSidecar = rig.requiredPaths.some((path) => path === sidecarRel || path.startsWith(`${sidecarRel}/`))
  const needsVenv = rig.requiredPaths.includes(`${sidecarRel}/.venv`)

  if (needsSidecar && !executionEnv.SHELLX_CUT_SIDECAR_DIR) {
    executionEnv.SHELLX_CUT_SIDECAR_DIR = resolve(repoRoot, sidecarRel)
    defaults.push({ name: 'SHELLX_CUT_SIDECAR_DIR', path: executionEnv.SHELLX_CUT_SIDECAR_DIR })
  }
  if (needsVenv && !executionEnv.SHELLX_CUT_PYTHON) {
    const pythonRel = platform === 'win32'
      ? `${sidecarRel}/.venv/Scripts/python.exe`
      : `${sidecarRel}/.venv/bin/python`
    executionEnv.SHELLX_CUT_PYTHON = resolve(repoRoot, pythonRel)
    defaults.push({ name: 'SHELLX_CUT_PYTHON', path: executionEnv.SHELLX_CUT_PYTHON })
  }

  return { env: executionEnv, defaults }
}

/**
 * Read an environment variable without assuming the caller's capitalisation.
 *
 * Windows treats environment names case-insensitively and `process.env` is a
 * proxy that honours that, so `process.env.PATH` works even when the shell
 * spells the variable `Path`. Spreading it into a plain object — which
 * `rigExecutionEnv` does (`{ ...env }`) — DROPS that proxy behaviour, so a
 * later `env.PATH` reads `undefined` on a standard PowerShell/cmd session.
 *
 * That single lookup used to fail every `requiredCommands` check on Windows:
 * `pathParts` came out empty, so preflight reported `command:cargo,
 * command:ffprobe` missing with both plainly on PATH, and since all nine rigs
 * declare `windows` in `platforms`, NO Windows rig receipt was producible from
 * a normal shell. It only works from a shell that happens to export uppercase
 * `PATH` (Git Bash), which is why the lookup can appear to work.
 *
 * @param {Record<string, string|undefined>} env environment object (possibly a plain spread copy)
 * @param {string} name variable name in its conventional casing
 * @returns {string|undefined} the value, or undefined when no spelling matches
 */
export function envValue(env, name) {
  if (env[name] !== undefined) return env[name]
  const wanted = name.toLowerCase()
  const key = Object.keys(env).find((candidate) => candidate.toLowerCase() === wanted)
  return key === undefined ? undefined : env[key]
}

/**
 * True when `path` is a real, runnable file rather than a placeholder.
 *
 * `existsSync` alone is not sufficient on Windows. A stock profile puts
 * Microsoft-Store **app execution aliases** on PATH, ahead of any real
 * interpreter — zero-byte reparse points that exist but cannot execute. One
 * satisfied `command:python3`, so preflight passed and `perception-base-fallback`
 * then died with "Python was not found; run without arguments to install from
 * the Microsoft Store". Catching that is precisely preflight's job: it exists
 * so a rig fails BEFORE spending build and test time.
 *
 * A zero-byte check is enough to separate the two — no real executable is empty.
 *
 * @param {string} path candidate executable path
 * @returns {boolean} true when the file exists and has content
 */
function isRealExecutable(path) {
  try {
    const stat = statSync(path)
    return stat.isFile() && stat.size > 0
  } catch {
    // Missing, unreadable, or a broken link — all "not usable" for preflight.
    return false
  }
}

/**
 * Whether `command` resolves to a usable executable on the given environment's PATH.
 *
 * Exported for testing: both failure modes above are environment-shaped and
 * cannot be reproduced from the repo's own shell, so they are covered by
 * injecting a synthetic `env` rather than by trusting the host.
 *
 * @param {string} command bare command name, e.g. "cargo"
 * @param {Record<string, string|undefined>} env environment to resolve against
 * @param {string} platform node platform string; injectable so Windows semantics are testable off-Windows
 * @returns {boolean} true when a real executable is found
 */
export function commandExists(command, env, platform = process.platform) {
  const isWindows = platform === 'win32'
  const pathParts = String(envValue(env, 'PATH') ?? '').split(isWindows ? ';' : ':').filter(Boolean)
  const extensions = isWindows
    ? String(envValue(env, 'PATHEXT') ?? '.EXE;.CMD;.BAT;.COM').split(';')
    : ['']
  return pathParts.some((dir) => extensions.some((ext) => isRealExecutable(join(dir, `${command}${ext}`))))
}

function artifactPath(repoRoot, spec, env) {
  const rawBase = spec.path ?? env[spec.env]
  const base = rawBase ? resolve(repoRoot, rawBase) : rawBase
  return base && spec.join ? join(base, spec.join) : base
}

export function cargoTestBinary(repoRoot, logs, prefix) {
  for (const line of logs.split('\n').reverse()) {
    const open = line.lastIndexOf('(')
    const close = line.lastIndexOf(')')
    if (open < 0 || close <= open) continue
    const candidate = line.slice(open + 1, close)
    const forward = candidate.replaceAll('\\', '/')
    const isTestDependency = [
      'target/debug/deps/',
      'target/release/deps/',
    ].some((marker) => forward.includes(marker))
    const filename = forward.slice(forward.lastIndexOf('/') + 1)
    if (isTestDependency && filename.startsWith(prefix)) {
      return resolve(repoRoot, candidate)
    }
  }
  return null
}

export function runIgnoredTestRig({ repoRoot, id, outDir, allowDirty = false, env = process.env }) {
  const manifest = loadIgnoredTestManifest(repoRoot)
  const rig = manifest.tests.find((entry) => entry.id === id)
  if (!rig) throw new Error(`unknown ignored-test rig '${id}'`)

  const execution = rigExecutionEnv(repoRoot, rig, env)
  const executionEnv = execution.env

  const stamp = new Date().toISOString().replace(/[:.]/g, '-')
  const receiptDir = resolve(outDir || join(repoRoot, '.shellx-scratch', 'ignored-test-rigs', `${id}-${stamp}`))
  mkdirSync(receiptDir, { recursive: true })
  const stdoutPath = join(receiptDir, 'stdout.log')
  const stderrPath = join(receiptDir, 'stderr.log')
  const receiptPath = join(receiptDir, 'receipt.json')
  const source = collectSourceIdentity(repoRoot)
  const requiredPaths = rig.requiredPaths.map((path) => ({ path, exists: existsSync(resolve(repoRoot, path)) }))
  const requiredCommands = rig.requiredCommands.map((command) => ({ command, exists: commandExists(command, executionEnv) }))
  const requiredEnv = rig.requiredEnv.map((name) => ({ name, present: !!executionEnv[name] }))
  const platform = manifestPlatform()
  const platformAllowed = rig.platforms.includes(platform)
  const inputs = rig.inputArtifacts.map((spec) => artifactInfo(artifactPath(repoRoot, spec, executionEnv), spec))
  const outputPaths = rig.outputArtifacts.map((spec) => artifactPath(repoRoot, spec, executionEnv))
  const outputDirectories = outputPaths
    .filter(Boolean)
    .map((path) => ({ path: dirname(path), exists: existsSync(dirname(path)) }))
  const outputConflicts = outputPaths.filter((path) => path && existsSync(path))
  const missing = [
    ...requiredPaths.filter((entry) => !entry.exists).map((entry) => `path:${entry.path}`),
    ...requiredCommands.filter((entry) => !entry.exists).map((entry) => `command:${entry.command}`),
    ...requiredEnv.filter((entry) => !entry.present).map((entry) => `env:${entry.name}`),
    ...inputs.filter((entry) => entry.path && !entry.exists).map((entry) => `artifact:${entry.path}`),
    ...outputDirectories.filter((entry) => !entry.exists).map((entry) => `output-dir:${entry.path}`),
    ...outputConflicts.map((path) => `output-exists:${path}`),
    ...(!platformAllowed ? [`platform:${platform}`] : []),
    ...(source.gitDirty && !allowDirty ? ['git:dirty'] : []),
  ]
  const receipt = {
    schema: manifest.receiptSchema,
    id,
    rustTest: rig.rustTest,
    classification: rig.classification,
    startedAt: new Date().toISOString(),
    source,
    host: { platform, arch: process.arch },
    command: rig.command,
    requirements: rig.requirements,
    preflight: {
      platformAllowed,
      requiredPaths,
      requiredCommands,
      requiredEnv,
      executionDefaults: execution.defaults,
      outputDirectories,
      outputConflicts,
      missing,
    },
    artifacts: { inputs, outputs: [], testBinary: null, stdout: null, stderr: null },
    pass: false,
  }

  if (missing.length > 0) {
    receipt.completedAt = new Date().toISOString()
    receipt.error = `preflight failed: ${missing.join(', ')}`
    writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`)
    return { receipt, receiptPath, exitCode: 2 }
  }

  const stdoutFd = openSync(stdoutPath, 'w')
  const stderrFd = openSync(stderrPath, 'w')
  let result
  try {
    result = spawnSync(rig.command[0], rig.command.slice(1), {
      cwd: repoRoot,
      env: executionEnv,
      stdio: ['ignore', stdoutFd, stderrFd],
    })
  } finally {
    closeSync(stdoutFd)
    closeSync(stderrFd)
  }

  const logs = `${readFileSync(stdoutPath, 'utf8')}\n${readFileSync(stderrPath, 'utf8')}`
  const binary = cargoTestBinary(repoRoot, logs, rig.testBinaryPrefix)
  const outputs = rig.outputArtifacts.map((spec) => artifactInfo(artifactPath(repoRoot, spec, executionEnv), spec))
  receipt.completedAt = new Date().toISOString()
  receipt.result = { status: result.status, signal: result.signal, error: result.error?.message ?? null }
  receipt.artifacts = {
    inputs,
    outputs,
    testBinary: artifactInfo(binary),
    stdout: artifactInfo(stdoutPath),
    stderr: artifactInfo(stderrPath),
  }
  receipt.pass = result.status === 0
    && receipt.artifacts.testBinary.exists
    && outputs.every((entry) => entry.exists)
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`)
  return { receipt, receiptPath, exitCode: receipt.pass ? 0 : 1 }
}
