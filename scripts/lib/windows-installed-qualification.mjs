import { spawn, spawnSync } from 'node:child_process'
import { copyFileSync, existsSync, readdirSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { delimiter, dirname, join, resolve, win32 } from 'node:path'

function capture(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
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
  harnessFfmpegWin,
  windowsBasePath,
  adapterPythonWin,
  stageWin,
  diarizeEndpoint = '',
  dubEndpoint = '',
}) {
  const releaseFixtures = join(root, 'scripts/release/fixtures')
  for (const name of readdirSync(releaseFixtures)) {
    const source = join(releaseFixtures, name)
    if (existsSync(source)) copyFileSync(source, join(fixtureDir, name))
  }
  for (const name of ['generate-prompt-adapter.py', 'generate-storyboard-adapter.py']) {
    copyFileSync(join(root, 'ui/public-tests/fixtures', name), join(fixtureDir, name))
  }
  return {
    SHELLX_CUT_HOME: win32.join(stageWin, 'app-home'),
    SHELLX_CUT_PROJECTS_DIR: win32.join(stageWin, 'projects'),
    CUT_DIARIZE_ENDPOINT: diarizeEndpoint,
    CUT_DUB_ENDPOINT: dubEndpoint,
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
