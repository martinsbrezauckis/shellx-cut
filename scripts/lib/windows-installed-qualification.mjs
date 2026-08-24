import { spawn, spawnSync } from 'node:child_process'
import { copyFileSync, existsSync, readdirSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { delimiter, dirname, join, resolve, win32 } from 'node:path'

import { compileWindowsAntigravityFixture as compileNativeAntigravityFixture } from './windows-antigravity-fixture-compile.mjs'
// Keep provider executables and their executable-side support files together.
// The fixture directory is intentionally first on PATH, so copying an unselected
// provider here would silently shadow that provider's canonical Windows CLI.
const WINDOWS_PROVIDER_FIXTURE_ARTIFACTS = Object.freeze({
  claude: ['claude', 'claude.cmd', 'agent-edit-fixture.mjs'],
  codex: ['codex', 'codex.cmd', 'agent-chat-provider-fixture.mjs', 'agent-edit-fixture.mjs'],
  grok: ['grok', 'grok.cmd', 'agent-chat-provider-fixture.mjs', 'agent-edit-fixture.mjs'],
  // agy.cmd is deliberately never staged: the deterministic fixture must retain
  // the same native agy.exe boundary as the installed Antigravity CLI.
  antigravity: ['agy', 'agy.exe', 'agy-windows-launcher.rs', 'agent-chat-provider-fixture.mjs', 'agent-edit-fixture.mjs'],
})

function capture(command, args, cwd, env = process.env) {
  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: 'utf8',
  })
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed: ${result.stderr || result.stdout}`)
  }
  return result.stdout.trim()
}

function windowsPath(path, cwd) {
  return capture('wslpath', ['-w', resolve(path)], cwd)
}

export function compileWindowsAntigravityFixture({
  fixtureDir,
  fixtureWin,
  nativeCompileParentWin,
  nativeCompileWin,
  captureCommand = capture,
}) {
  return compileNativeAntigravityFixture({
    fixtureDir, fixtureWin, nativeCompileParentWin, nativeCompileWin, captureCommand,
  })
}

function runCaptured(command, args, { env = process.env, cwd }) {
  return new Promise((resolveRun, reject) => {
    const child = spawn(command, args, { cwd, env, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''; let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk; process.stdout.write(chunk) })
    child.stderr.on('data', (chunk) => { stderr += chunk; process.stderr.write(chunk) })
    child.on('error', reject)
    child.on('exit', (code, signal) => {
      if (code === 0) resolveRun({ stdout, stderr })
      else reject(new Error(`${command} ${args.join(' ')} failed: code=${code} signal=${signal || 'none'}${stderr ? `; ${stderr.trim()}` : ''}`))
    })
  })
}

export function resolveInstalledHarnessFfmpeg({ cwd }) {
  return capture('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    "$candidates = @(" +
      "(Join-Path $env:LOCALAPPDATA 'ShellX Cut\\tools\\ffmpeg\\bin\\ffmpeg.exe')," +
      "(Join-Path $env:LOCALAPPDATA 'ShellX Cut\\ffmpeg\\bin\\ffmpeg.exe')," +
      "((Get-Command ffmpeg.exe -ErrorAction SilentlyContinue).Source)" +
      "); $found = $candidates | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } | Select-Object -First 1; " +
      "if (-not $found) { throw 'Installed qualification requires a Windows ffmpeg.exe' }; $found",
  ], cwd)
}

export function stopWindowsInstalledProcesses({ cwd }) {
  capture('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    '$targets=@(Get-Process shellx-cut,cutd -ErrorAction SilentlyContinue); if($targets.Count){$targets|Stop-Process -Force -ErrorAction SilentlyContinue}; exit 0',
  ], cwd)
}

export function assertWindowsInteractiveSession({ cwd }) {
  const state = JSON.parse(capture('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    '$sessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId; ' +
      '$explorer = @(Get-Process explorer -ErrorAction SilentlyContinue | Where-Object SessionId -eq $sessionId).Count; ' +
      '$locked = @(Get-Process LogonUI -ErrorAction SilentlyContinue | Where-Object SessionId -eq $sessionId).Count; ' +
      '[pscustomobject]@{ hostId = $env:COMPUTERNAME; sessionId = $sessionId; explorerWindows = $explorer; locked = ($locked -gt 0) } | ConvertTo-Json -Compress',
  ], cwd))
  if (state.sessionId <= 0 || state.explorerWindows < 1 || state.locked) {
    throw new Error(
      `Windows installed qualification requires an unlocked interactive desktop session; ` +
      `session=${state.sessionId} explorer=${state.explorerWindows} locked=${state.locked}`,
    )
  }
  return state
}

export function findWindowsNsisInstaller({ root }) {
  const dir = join(root, 'app/desktop/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis')
  const matches = existsSync(dir)
    ? readdirSync(dir).filter((name) => /^ShellX Cut_.+_x64-setup[.]exe$/.test(name))
    : []
  if (matches.length !== 1) throw new Error(`expected exactly one fresh NSIS installer under ${dir}; found ${matches.length}`)
  return join(dir, matches[0])
}

export function activateInstalledWindowForUnattendedRun({ root, cwd = root }) {
  const script = windowsPath(join(root, 'scripts/release/activate-installed-windows.ps1'), cwd)
  const state = JSON.parse(capture('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-STA', '-ExecutionPolicy', 'Bypass',
    '-File', script, '-ExpectedProcessName', 'shellx-cut',
  ], cwd))
  if (String(state.foregroundProcess || '').toLowerCase() !== 'shellx-cut' ||
      Number(state.foregroundPid) !== Number(state.requestedPid)) {
    throw new Error(
      `unattended native UI activation did not land on the exact ShellX Cut process: ${JSON.stringify(state)}`,
    )
  }
  return state
}

export function prepareWindowsBuildEnvironment({
  baseEnv = process.env,
  commandPaths,
  llvmLibDir,
  windowsSystem32,
}) {
  const required = [
    'node', 'npm', 'cargo', 'cargo-xwin', 'clang-cl', 'lld-link',
    'llvm-config', 'llvm-lib', 'llvm-rc', 'makensis', 'powershell.exe', 'wslpath',
  ]
  for (const name of required) {
    if (!commandPaths?.[name]) throw new Error(`Windows build environment is missing explicit ${name}`)
  }
  if (!llvmLibDir) throw new Error('Windows build environment is missing the LLVM runtime directory')
  if (!windowsSystem32) throw new Error('Windows build environment is missing the WSL System32 path')

  const unique = (values) => [...new Set(values.filter(Boolean))]
  const pathEntries = unique([
    ...required.map((name) => dirname(commandPaths[name])),
    windowsSystem32,
    ...String(baseEnv.PATH || '').split(delimiter),
  ])
  const libraryEntries = unique([
    llvmLibDir,
    ...String(baseEnv.LD_LIBRARY_PATH || '').split(delimiter),
  ])
  return {
    ...baseEnv,
    PATH: pathEntries.join(delimiter),
    LD_LIBRARY_PATH: libraryEntries.join(delimiter),
  }
}

export function resolveWindowsBuildEnvironment({ baseEnv = process.env, cwd }) {
  const wslInteropPresent = Boolean(baseEnv.WSL_INTEROP) || existsSync('/run/WSL')
  if (!wslInteropPresent) throw new Error('Windows build environment requires active WSL interop')
  const names = [
    'node', 'npm', 'cargo', 'cargo-xwin', 'clang-cl', 'lld-link',
    'llvm-config', 'llvm-lib', 'llvm-rc', 'makensis', 'powershell.exe', 'wslpath',
  ]
  const commandPaths = Object.fromEntries(names.map((name) => [name, capture('which', [name], cwd)]))
  const llvmLibDir = capture(commandPaths['llvm-config'], ['--libdir'], cwd)
  const system32Win = capture('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command', '[Environment]::SystemDirectory',
  ], cwd)
  const windowsSystem32 = capture('wslpath', ['-u', system32Win], cwd)
  return {
    env: prepareWindowsBuildEnvironment({ baseEnv, commandPaths, llvmLibDir, windowsSystem32 }),
    receipt: {
      explicitCommands: names,
      explicitLlvmRuntime: true,
      explicitWindowsSystem32: true,
      wslInteropPresent,
    },
  }
}

export function resolveWindowsNode({ requested = '', cwd }) {
  let executableWin = requested.trim()
  if (executableWin) {
    const expanded = executableWin.startsWith('~/') ? join(homedir(), executableWin.slice(2)) : executableWin
    const local = resolve(expanded)
    if (existsSync(local)) executableWin = windowsPath(local, cwd)
  } else {
    executableWin = capture('powershell.exe', [
      '-NoProfile', '-NonInteractive', '-Command',
      '(Get-Command node.exe -ErrorAction Stop).Source',
    ], cwd)
  }
  const executableWsl = capture('wslpath', ['-u', executableWin], cwd)
  const probe = spawnSync(executableWsl, [
    '-p',
    'JSON.stringify({ platform: process.platform, version: process.version, execPath: process.execPath })',
  ], { cwd, encoding: 'utf8' })
  if (probe.status !== 0) throw new Error(`Windows Node probe failed: ${probe.stderr || probe.stdout}`)
  const runtime = JSON.parse(probe.stdout.trim())
  if (runtime.platform !== 'win32') {
    throw new Error(`installed Windows qualification requires native Windows Node; got ${runtime.platform}`)
  }
  return { ...runtime, executableWin, executableWsl }
}

export async function proveWindowsInstalledReadiness({ command, args, env, cwd, proofPath, proof }) {
  const readiness = await runCaptured(command, args, { env, cwd })
  const markers = {
    cdp: /(?:^|\n)CDP_READY\b/.test(readiness.stdout),
    engine: /(?:^|\n)CUTD_READY\b/.test(readiness.stdout),
    agentDocs: /(?:^|\n)AGENT_DOCS_READY\b/.test(readiness.stdout),
  }
  if (!Object.values(markers).every(Boolean)) {
    throw new Error(`installed app readiness proof is incomplete: ${JSON.stringify(markers)}`)
  }
  writeFileSync(proofPath, `${JSON.stringify({
    schema: 'shellx-cut/windows-installed-launch@1',
    generatedAt: new Date().toISOString(),
    ...proof,
    markers,
    uiRowsStarted: false,
  }, null, 2)}\n`)
  return markers
}

export function prepareWindowsQualificationEnvironment({
  root,
  fixtureDir,
  fixtureWin,
  fixtureProviders = ['claude', 'codex', 'grok', 'antigravity'],
  harnessFfmpegWin,
  windowsBasePath,
  adapterPythonWin,
  stageWin,
  diarizeEndpoint = '',
  dubEndpoint = '',
  matteModel = '',
  nativeCompileWin = '',
  copyFixtureFile = copyFileSync,
}) {
  const stagedProviders = new Set(fixtureProviders)
  const releaseFixtures = join(root, 'scripts/release/fixtures')
  const stagedArtifacts = new Set()
  for (const provider of stagedProviders) {
    const artifacts = WINDOWS_PROVIDER_FIXTURE_ARTIFACTS[provider]
    if (!artifacts) {
      throw new Error(`Windows qualification does not recognize Agent Chat fixture provider ${provider}`)
    }
    for (const name of artifacts) stagedArtifacts.add(name)
  }
  for (const name of stagedArtifacts) {
    const source = join(releaseFixtures, name)
    if (existsSync(source)) copyFixtureFile(source, join(fixtureDir, name))
  }
  if (stagedProviders.has('antigravity') && !existsSync(join(fixtureDir, 'agy.exe'))) {
    compileWindowsAntigravityFixture({
      fixtureDir,
      fixtureWin,
      nativeCompileParentWin: win32.join(stageWin, 'app-home'),
      nativeCompileWin,
    })
  }
  const fixtureExecutables = {
    claude: 'claude.cmd',
    codex: 'codex.cmd',
    grok: 'grok.cmd',
    antigravity: 'agy.exe',
  }
  for (const provider of stagedProviders) {
    const name = fixtureExecutables[provider]
    if (!name) throw new Error(`Windows qualification does not recognize Agent Chat fixture provider ${provider}`)
    if (!existsSync(join(fixtureDir, name))) {
      throw new Error(`Windows qualification requires the staged Agent Chat fixture ${name}`)
    }
  }
  // These adapters are provider-independent runtime dependencies. They must be
  // staged even when live generation leaves Codex, Grok, and Antigravity on
  // their canonical PATH; merely pointing CUTD_*_ADAPTER at an absent file
  // degrades every Doctor judge card and falsely reports the CLIs unavailable.
  for (const name of ['comment-draft-adapter.py', 'judge-adapter.py']) {
    copyFixtureFile(join(releaseFixtures, name), join(fixtureDir, name))
  }
  for (const name of ['generate-prompt-adapter.py', 'generate-storyboard-adapter.py']) {
    copyFixtureFile(join(root, 'ui/public-tests/fixtures', name), join(fixtureDir, name))
  }
  return {
    SHELLX_CUT_HOME: win32.join(stageWin, 'app-home'),
    SHELLX_CUT_PROJECTS_DIR: win32.join(stageWin, 'projects'),
    CUT_DIARIZE_ENDPOINT: diarizeEndpoint,
    CUT_DUB_ENDPOINT: dubEndpoint,
    MATTE_MODEL: matteModel,
    PATH: `${fixtureWin};${win32.dirname(harnessFfmpegWin)};${windowsBasePath}`,
    FFMPEG_BIN: harnessFfmpegWin,
    SHELLX_CUT_FFMPEG: harnessFfmpegWin,
    SHELLX_CUT_PYTHON: adapterPythonWin,
    CUTD_DRAFT_ADAPTER: win32.join(fixtureWin, 'comment-draft-adapter.py'),
    CUTD_JUDGE_ADAPTER: win32.join(fixtureWin, 'judge-adapter.py'),
    CUTD_GENERATE_PROMPT_ADAPTER: win32.join(fixtureWin, 'generate-prompt-adapter.py'),
    CUTD_GENERATE_STORYBOARD_ADAPTER: win32.join(fixtureWin, 'generate-storyboard-adapter.py'),
    CUTD_GENERATE_FIXTURE_DELAY_MS: '1200',
  }
}
