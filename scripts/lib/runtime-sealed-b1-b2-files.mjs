import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import {
  chmodSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync,
  realpathSync, readlinkSync, rmSync, rmdirSync, unlinkSync, writeFileSync,
} from 'node:fs'
import { homedir } from 'node:os'
import { basename, dirname, isAbsolute, join, relative, resolve, win32 } from 'node:path'

function samePath(first, second) {
  const a = resolve(first)
  const b = resolve(second)
  return process.platform === 'win32' ? a.toLowerCase() === b.toLowerCase() : a === b
}

function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

export function assertRealPath(path, label) {
  const requested = resolve(path)
  if (!existsSync(requested)) throw new Error(`${label} does not exist: ${requested}`)
  const actual = resolve(realpathSync.native(requested))
  if (!samePath(requested, actual)) throw new Error(`${label} must not traverse a symbolic link: ${requested}`)
  return actual
}

export function assertContainedRealPath(root, path, label) {
  const realRoot = assertRealPath(root, `${label} root`)
  const realPath = assertRealPath(path, label)
  const rel = relative(realRoot, realPath)
  if (!rel || rel === '..' || rel.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`) || isAbsolute(rel)) {
    throw new Error(`${label} must stay below its owned root`)
  }
  return realPath
}

function assertEntry(path, label) {
  const entry = lstatSync(path)
  if (entry.isSymbolicLink()) throw new Error(`${label} contains a symbolic link: ${path}`)
  return entry
}

export function sealedRegularFile(path, label, { executable = false } = {}) {
  const real = assertRealPath(path, label)
  const entry = assertEntry(real, label)
  if (!entry.isFile() || entry.size <= 0) throw new Error(`${label} must be a non-empty regular file: ${real}`)
  if (executable && process.platform !== 'win32' && (entry.mode & 0o111) === 0) throw new Error(`${label} must be executable: ${real}`)
  return { kind: 'file', path: real, bytes: entry.size, sha256: sha256File(real) }
}

function walkSealed(root, path, files) {
  const entry = assertEntry(path, 'sealed tree')
  if (entry.isDirectory()) {
    for (const child of readdirSync(path, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      walkSealed(root, join(path, child.name), files)
    }
    return
  }
  if (!entry.isFile()) throw new Error(`sealed tree contains a non-regular entry: ${path}`)
  files.push({ path: relative(root, path).replaceAll('\\', '/'), bytes: entry.size, sha256: sha256File(path) })
}

export function sealedTree(path, label, { allowEmpty = false } = {}) {
  const real = assertRealPath(path, label)
  const entry = assertEntry(real, label)
  if (!entry.isDirectory()) throw new Error(`${label} must be a directory: ${real}`)
  const files = []
  walkSealed(real, real, files)
  if (!allowEmpty && files.length === 0) throw new Error(`${label} must not be empty: ${real}`)
  const sha256 = createHash('sha256').update(files.map((file) => `${file.path}\0${file.bytes}\0${file.sha256}\n`).join('')).digest('hex')
  return { kind: 'tree', path: real, files: files.length, bytes: files.reduce((sum, file) => sum + file.bytes, 0), sha256, entries: files }
}

export function assertStableSealedArtifact(label, before, after) {
  for (const key of ['kind', 'path', 'sha256', 'bytes', 'files']) {
    if ((before[key] ?? null) !== (after[key] ?? null)) throw new Error(`${label} drifted during qualification (${key})`)
  }
  return true
}

export function createOwnedOutputRoot(repoRoot, scratchRelative, requested) {
  const repo = assertRealPath(repoRoot, 'repository root')
  const segments = scratchRelative.split('/').filter(Boolean)
  let scratch = repo
  for (const segment of segments) {
    scratch = join(scratch, segment)
    if (!existsSync(scratch)) mkdirSync(scratch, { mode: 0o700 })
    const entry = assertEntry(scratch, 'qualification scratch root')
    if (!entry.isDirectory()) throw new Error(`qualification scratch root is not a directory: ${scratch}`)
    scratch = assertRealPath(scratch, 'qualification scratch root')
  }
  const candidate = requested ? resolve(repo, requested) : join(scratch, `run-${Date.now()}-${Math.random().toString(16).slice(2)}`)
  const rel = relative(scratch, candidate)
  if (!rel || rel.includes('/') || rel.includes('\\') || rel === '..' || rel.startsWith('..') || isAbsolute(rel)) {
    throw new Error(`--out must name one new directory directly below ${scratchRelative}`)
  }
  if (existsSync(candidate)) throw new Error(`--out must not already exist: ${candidate}`)
  if (!samePath(dirname(candidate), scratch)) throw new Error('--out parent does not resolve to the owned scratch root')
  mkdirSync(candidate, { mode: 0o700 })
  const entry = assertEntry(candidate, 'qualification output')
  if (!entry.isDirectory()) throw new Error(`qualification output is not a directory: ${candidate}`)
  return assertContainedRealPath(scratch, candidate, 'qualification output')
}

function removeOwnedTreeWithLinks(path, label) {
  const entry = lstatSync(path)
  if (entry.isFile() || entry.isSymbolicLink()) {
    unlinkSync(path)
    return
  }
  if (!entry.isDirectory()) throw new Error(`${label} contains a non-file, non-directory entry: ${path}`)
  for (const child of readdirSync(path, { withFileTypes: true })) {
    removeOwnedTreeWithLinks(join(path, child.name), label)
  }
  rmdirSync(path)
}

export function removeOwnedTree(root, path, label, { allowOwnedLinks = false } = {}) {
  const real = assertContainedRealPath(root, path, label)
  if (!samePath(dirname(real), assertRealPath(root, `${label} root`))) throw new Error(`${label} must be a direct owned child`)
  const entry = assertEntry(real, label)
  if (!entry.isDirectory()) throw new Error(`${label} is not a directory`)
  if (allowOwnedLinks) removeOwnedTreeWithLinks(real, label)
  else {
    sealedTree(real, label, { allowEmpty: true })
    rmSync(real, { recursive: true, force: false })
  }
  if (existsSync(real)) throw new Error(`${label} remained after cleanup`)
  return { status: 'removed', path: relative(root, real).replaceAll('\\', '/'), allowOwnedLinks }
}

export function resolveExecutable(command, env, label) {
  const requested = String(command || '')
  if (!requested) throw new Error(`${label} command is missing`)
  const direct = requested.includes('/') || requested.includes('\\')
  const names = process.platform === 'win32'
    ? [requested, ...String(env.PATHEXT || '.EXE;.CMD;.BAT;.COM').split(';').filter(Boolean).map((ext) => `${requested}${ext}`)]
    : [requested]
  const candidates = direct ? names.map((name) => resolve(name)) : String(env.PATH || '').split(process.platform === 'win32' ? ';' : ':').filter(Boolean).flatMap((dir) => names.map((name) => join(dir, name)))
  for (const path of candidates) {
    if (!existsSync(path)) continue
    try { return sealedRegularFile(path, label, { executable: true }) } catch { /* continue search */ }
  }
  throw new Error(`${label} is not a resolvable executable: ${requested}`)
}

function realDirectory(path, label) {
  const real = assertRealPath(path, label)
  const entry = assertEntry(real, label)
  if (!entry.isDirectory()) throw new Error(`${label} must be a real directory: ${real}`)
  return real
}

function dispatcherInvocation(path, command, cargoBin) {
  const requested = resolve(path)
  const entry = lstatSync(requested)
  if (entry.isFile()) return { kind: 'file', path: requested, bytes: entry.size, sha256: sha256File(requested) }
  if (!entry.isSymbolicLink()) throw new Error(`${command} Rustup invocation is not a file or symbolic dispatcher: ${requested}`)
  const target = readlinkSync(requested)
  const expected = join(cargoBin, process.platform === 'win32' ? 'rustup.exe' : 'rustup')
  const targetPath = resolve(dirname(requested), target)
  if (!samePath(targetPath, expected)) throw new Error(`${command} Rustup dispatcher must be the direct cargo-bin rustup target`)
  const dispatcher = sealedRegularFile(targetPath, `${command} Rustup dispatcher`, { executable: true })
  return { kind: 'rustup-dispatcher-link', path: requested, linkTarget: target, bytes: Buffer.byteLength(target), sha256: createHash('sha256').update(target).digest('hex'), dispatcher }
}

function runRustup(dispatcher, args, env, label) {
  const result = spawnSync(dispatcher.path, args, { encoding: 'utf8', timeout: 15_000, env })
  if (result.status !== 0) throw new Error(`${label} failed: ${String(result.stderr || result.error || '').trim()}`)
  return String(result.stdout).trim()
}

export function resolveRustupToolchain(env = process.env) {
  const cargoHome = realDirectory(env.CARGO_HOME || join(homedir(), '.cargo'), 'governed CARGO_HOME')
  const rustupHome = realDirectory(env.RUSTUP_HOME || join(homedir(), '.rustup'), 'governed RUSTUP_HOME')
  const cargoBin = realDirectory(join(cargoHome, 'bin'), 'governed Cargo bin')
  const rustupEnv = { PATH: cargoBin, CARGO_HOME: cargoHome, RUSTUP_HOME: rustupHome }
  const names = process.platform === 'win32' ? { cargo: 'cargo.exe', rustc: 'rustc.exe' } : { cargo: 'cargo', rustc: 'rustc' }
  const invocations = Object.fromEntries(Object.entries(names).map(([command, name]) => {
    const path = join(cargoBin, name)
    if (!existsSync(path)) throw new Error(`governed ${command} Rustup entry point is missing: ${path}`)
    return [command, dispatcherInvocation(path, command, cargoBin)]
  }))
  const dispatcher = invocations.cargo.dispatcher || sealedRegularFile(invocations.cargo.path, 'cargo Rustup dispatcher', { executable: true })
  const rustcDispatcher = invocations.rustc.dispatcher || sealedRegularFile(invocations.rustc.path, 'rustc Rustup dispatcher', { executable: true })
  if (dispatcher.sha256 !== rustcDispatcher.sha256 || dispatcher.bytes !== rustcDispatcher.bytes) throw new Error('cargo and rustc must share the governed Rustup dispatcher')
  const toolchain = runRustup(dispatcher, ['show', 'active-toolchain'], rustupEnv, 'Rustup active-toolchain').split(/\s+/)[0]
  if (!toolchain || /[\\/]/.test(toolchain)) throw new Error('Rustup returned an unsafe active toolchain name')
  const toolchainPath = realDirectory(join(rustupHome, 'toolchains', toolchain), 'governed Rustup toolchain')
  const resolved = Object.fromEntries(Object.keys(names).map((command) => {
    const path = runRustup(dispatcher, ['which', command], { ...rustupEnv, RUSTUP_TOOLCHAIN: toolchain }, `Rustup resolved ${command}`)
    return [command, sealedRegularFile(assertContainedRealPath(toolchainPath, path, `resolved ${command}`), `resolved ${command}`, { executable: true })]
  }))
  return {
    cargoHome, rustupHome, toolchain: { name: toolchain, path: toolchainPath }, dispatcher,
    cargo: { invocation: invocations.cargo, resolved: resolved.cargo }, rustc: { invocation: invocations.rustc, resolved: resolved.rustc },
  }
}

function executableCandidates(command, env) {
  const direct = command.includes('/') || command.includes('\\')
  const names = process.platform === 'win32'
    ? [command, ...String(env.PATHEXT || '.EXE;.CMD;.BAT;.COM').split(';').filter(Boolean).map((ext) => `${command}${ext}`)]
    : [command]
  return direct ? names.map((name) => resolve(name)) : String(env.PATH || '').split(process.platform === 'win32' ? ';' : ':').filter(Boolean).flatMap((dir) => names.map((name) => join(dir, name)))
}

function nativeInvocation(path, label, command) {
  const requested = resolve(path)
  let current = requested
  const links = []
  for (let depth = 0; depth < 5; depth += 1) {
    const entry = lstatSync(current)
    if (!entry.isSymbolicLink()) {
      const resolved = resolve(realpathSync.native(current))
      return { command, invocation: { kind: links.length ? 'link-chain' : 'file', path: requested, links }, resolved: sealedRegularFile(resolved, label, { executable: true }) }
    }
    const target = readlinkSync(current)
    links.push({ path: current, target, sha256: createHash('sha256').update(target).digest('hex') })
    current = resolve(dirname(current), target)
  }
  throw new Error(`${label} symbolic-link chain exceeds the bounded depth`)
}

function resolveUniqueNativeTool(command, env, label) {
  const found = []
  const invalid = []
  for (const path of executableCandidates(command, env)) {
    if (!existsSync(path)) continue
    try { found.push(nativeInvocation(path, label, command)) } catch (error) { invalid.push(`${path}: ${error.message || error}`) }
  }
  if (invalid.length) throw new Error(`${label} has an invalid PATH candidate: ${invalid[0]}`)
  const unique = [...new Map(found.map((tool) => [tool.resolved.path, tool])).values()]
  if (unique.length !== 1) throw new Error(`${label} must resolve to exactly one regular target (found ${unique.length})`)
  return unique[0]
}

function cargoUsesPkgConfig(repoRoot) {
  return [join(repoRoot, 'app/Cargo.lock'), join(repoRoot, 'app/desktop/src-tauri/Cargo.lock')]
    .some((path) => existsSync(path) && readFileSync(path, 'utf8').includes('name = "pkg-config"'))
}

export function resolveNativeBuildTools(repoRoot, env = process.env) {
  const names = { cc: 'cc', ar: 'ar', ld: 'ld', as: 'as', ...(cargoUsesPkgConfig(repoRoot) ? { pkgConfig: 'pkg-config' } : {}) }
  return Object.fromEntries(Object.entries(names).map(([id, command]) => [id, resolveUniqueNativeTool(command, env, `native build tool ${command}`)]))
}

export function resolveNpmScriptShell(env = process.env) {
  const command = process.platform === 'win32' ? 'cmd.exe' : 'sh'
  const shell = resolveUniqueNativeTool(command, env, 'npm script shell')
  return { ...shell, npm: npmScriptShellSemantics(shell.resolved.path) }
}

export function npmScriptShellSemantics(path, platform = process.platform) {
  const paths = platform === 'win32' ? win32 : { basename, isAbsolute }
  if (!paths.isAbsolute(path)) throw new Error('npm script shell must be an absolute path')
  if (platform === 'win32') {
    if (paths.basename(path).toLowerCase() !== 'cmd.exe') throw new Error('Windows npm script shell must resolve to cmd.exe')
    return { path, npmArgs: ['/d', '/s', '/c'] }
  }
  return { path, npmArgs: ['-c'] }
}

export function assertStableNpmScriptShell(before, after) {
  if (JSON.stringify(before) !== JSON.stringify(after)) throw new Error('npm script shell drifted during qualification')
  return true
}

export function recheckNpmScriptShell(expected, env = process.env) {
  const current = resolveNpmScriptShell(env)
  assertStableNpmScriptShell(expected, current)
  return current
}

export function assertPosixShebangPath(path) {
  if (!isAbsolute(path) || /[\s\x00-\x1f\x7f]/.test(path) || Buffer.byteLength(path) > 127) {
    throw new Error('POSIX wrapper shell path is not representable in a shebang')
  }
  return path
}

function quotedShellPath(path) { return `'${path.replaceAll("'", "'\\\"'\\\"'")}'` }

function wrapperText(path, shell) {
  return process.platform === 'win32' ? `@echo off\r\n"${path.replaceAll('"', '""')}" %*\r\n` : `#!${shell}\nexec ${quotedShellPath(path)} "$@"\n`
}

export function materializeBuildToolBin(nativeTools, node, destination, posixShell = null) {
  const parent = assertRealPath(dirname(destination), 'governed build-tool parent')
  const bin = resolve(destination)
  if (existsSync(bin)) throw new Error(`governed build-tool bin already exists: ${bin}`)
  let shell = null
  if (process.platform !== 'win32') {
    if (!posixShell?.resolved) throw new Error('governed POSIX wrappers require a sealed shell')
    const current = sealedRegularFile(posixShell.resolved.path, 'POSIX wrapper shell', { executable: true })
    assertStableSealedArtifact('POSIX wrapper shell', posixShell.resolved, current)
    shell = assertPosixShebangPath(current.path)
  }
  mkdirSync(bin, { mode: 0o700 })
  const realBin = assertContainedRealPath(parent, bin, 'governed build-tool bin')
  const source = { node: { command: 'node', resolved: node }, ...nativeTools }
  const tools = {}
  for (const [id, tool] of Object.entries(source)) {
    if (!/^[A-Za-z0-9._-]+$/.test(tool.command)) throw new Error(`unsafe governed build-tool name: ${tool.command}`)
    const current = sealedRegularFile(tool.resolved.path, `native build tool ${tool.command}`, { executable: true })
    assertStableSealedArtifact(`native build tool ${tool.command}`, tool.resolved, current)
    const wrapper = join(realBin, process.platform === 'win32' ? `${tool.command}.cmd` : tool.command)
    if (existsSync(wrapper)) throw new Error(`ambiguous governed build-tool wrapper: ${tool.command}`)
    writeFileSync(wrapper, wrapperText(current.path, shell), { mode: 0o700 })
    chmodSync(wrapper, 0o700)
    tools[id] = sealedRegularFile(wrapper, `governed build-tool wrapper ${tool.command}`, { executable: true })
  }
  return { path: realBin, tools, manifest: sealedTree(realBin, 'governed build-tool bin') }
}

export function boundedEvidenceFiles(root) {
  const tree = sealedTree(root, 'qualification evidence')
  return tree.entries.filter((file) => file.path !== 'runtime-sealed-b1-b2-receipt.json')
}
