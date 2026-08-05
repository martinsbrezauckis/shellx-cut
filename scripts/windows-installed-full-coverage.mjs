#!/usr/bin/env node
import { createHash } from 'node:crypto'
import { spawn, spawnSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { basename, dirname, join, resolve, win32 } from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchInstalledCutWithCdp, normalizeCdpPort } from './lib/windows-cdp-launch.mjs'
import { activateInstalledWindowForUnattendedRun, prepareWindowsQualificationEnvironment, proveWindowsInstalledReadiness, resolveInstalledHarnessFfmpeg, resolveWindowsBuildEnvironment, resolveWindowsNode } from './lib/windows-installed-qualification.mjs'
import { releaseWindowsWebviewProfile } from './lib/windows-webview-profile.mjs'
import { beginWindowsInstalledWalkthrough, finishWindowsInstalledWalkthrough } from './lib/windows-installed-walkthrough.mjs'
import { forwardEnvToWindows } from './lib/wsl-interop-env.mjs'
import { sourceContentManifest } from './lib/source-content-manifest.mjs'
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
function arg(name, fallback = '') {
  const index = process.argv.indexOf(name)
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback
}
const flag = (name) => process.argv.includes(name)
const stamp = new Date().toISOString().replace(/[:.]/g, '-')
const expandHome = (path) => path.startsWith('~/') ? join(homedir(), path.slice(2)) : path
function usage() {
  console.log(`Usage: node scripts/windows-installed-full-coverage.mjs --unattended-native-ui [--signed-final] [--diagnostic-section <comma-list>] [--skip-build] [--skip-install] [--clean-after] [--out <evidence-dir>] [--cdp-port 9223] [--windows-node <path>] [--scene <file>] [--speech <file>] [--face <file>] [--speakers <file>] [--second <file>]

Builds, installs, and drives the Windows shipping artifact through WebView2 CDP.
The unfiltered strict action matrix is mandatory. Default mode is an unsigned
installed candidate. --signed-final requires five explicit real-media roles,
requires valid Authenticode in the install smoke, and is reserved for final
native qualification. --diagnostic-section is a non-qualifying focused rerun
and is forbidden with --signed-final. --unattended-native-ui explicitly allows
the runner to activate the installed app once before native-dialog actions; use
it only on a dedicated machine where focus cannot interfere with a user.
--clean-after preserves evidence but removes the exact test profile/media
staging and rebuildable WSL build trees.`)
}
function run(command, args, { env = process.env, cwd = ROOT } = {}) {
  return new Promise((resolveRun, reject) => {
    const child = spawn(command, args, { cwd, env, stdio: 'inherit' })
    child.on('error', reject)
    child.on('exit', (code, signal) => {
      if (code === 0) resolveRun()
      else reject(new Error(`${command} ${args.join(' ')} failed: code=${code} signal=${signal || 'none'}`))
    })
  })
}
function captureSync(command, args) {
  const result = spawnSync(command, args, {
    cwd: ROOT,
    encoding: 'utf8',
  })
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed: ${result.stderr || result.stdout}`)
  }
  return result.stdout.trim()
}
function windowsPath(path) {
  return captureSync('wslpath', ['-w', resolve(path)])
}
function linuxPath(path) {
  return captureSync('wslpath', ['-u', path])
}
function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}
function stopInstalledProcesses() {
  captureSync('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    'Get-Process shellx-cut,cutd -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue',
  ])
}
function assertWindowsInteractiveSession() {
  const state = JSON.parse(captureSync('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    '$sessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId; ' +
      '$explorer = @(Get-Process explorer -ErrorAction SilentlyContinue | Where-Object SessionId -eq $sessionId).Count; ' +
      '$locked = @(Get-Process LogonUI -ErrorAction SilentlyContinue | Where-Object SessionId -eq $sessionId).Count; ' +
      '[pscustomobject]@{ sessionId = $sessionId; explorerWindows = $explorer; locked = ($locked -gt 0) } | ConvertTo-Json -Compress',
  ]))
  if (state.sessionId <= 0 || state.explorerWindows < 1 || state.locked) {
    throw new Error(
      `Windows installed qualification requires an unlocked interactive desktop session; ` +
      `session=${state.sessionId} explorer=${state.explorerWindows} locked=${state.locked}`,
    )
  }
  return state
}
function findInstaller() {
  const dir = join(ROOT, 'app/desktop/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis')
  const matches = existsSync(dir)
    ? readdirSync(dir).filter((name) => /^ShellX Cut_.+_x64-setup[.]exe$/.test(name))
    : []
  if (matches.length !== 1) throw new Error(`expected exactly one fresh NSIS installer under ${dir}; found ${matches.length}`)
  return join(dir, matches[0])
}

async function main() {
  if (flag('--help') || flag('-h')) return usage()
  if (process.env.FCV_SECTION || process.env.FCV_ONLY) {
    throw new Error('Windows installed qualification forbids FCV_SECTION and FCV_ONLY')
  }
  const signedFinal = flag('--signed-final')
  const diagnosticSection = arg('--diagnostic-section', '').trim()
  const unattendedNativeUi = flag('--unattended-native-ui')
  if (!unattendedNativeUi) {
    throw new Error(
      'Windows installed qualification requires --unattended-native-ui on a dedicated machine; refusing to steal focus from an unknown user session',
    )
  }
  if (signedFinal && diagnosticSection) {
    throw new Error('--signed-final forbids --diagnostic-section; final qualification is always unfiltered')
  }
  const cleanAfter = flag('--clean-after')
  const roleArgs = {
    scene: arg('--scene', join(ROOT, 'testdata/talking_head.mp4')),
    speech: arg('--speech', join(ROOT, 'testdata/talking_head.mp4')),
    face: arg('--face', join(ROOT, 'testdata/moving_face.mp4')),
    // This role needs speech/audio to prove media.diarize and audio.dub.
    speakers: arg('--speakers', join(ROOT, 'testdata/talking_head.mp4')),
    second: arg('--second', join(ROOT, 'testdata/silent_screen.mp4')),
  }
  if (signedFinal) {
    for (const role of Object.keys(roleArgs)) {
      if (!process.argv.includes(`--${role}`)) throw new Error(`--signed-final requires --${role} with real release media`)
    }
  }
  for (const [role, path] of Object.entries(roleArgs)) {
    roleArgs[role] = resolve(expandHome(path))
    if (!existsSync(roleArgs[role])) throw new Error(`${role} media not found: ${roleArgs[role]}`)
  }
  const interactiveSession = assertWindowsInteractiveSession()
  const out = resolve(expandHome(arg(
    '--out',
    `~/.shellx-scratch/shellx-cut/windows-installed-${diagnosticSection ? 'diagnostic' : signedFinal ? 'signed-final' : 'candidate'}-${stamp}`,
  )))
  const artifacts = join(out, 'artifacts')
  mkdirSync(artifacts, { recursive: true })
  const tempWin = captureSync('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command', '[IO.Path]::GetTempPath()',
  ]).replace(/[\\/]$/, '')
  const stageWin = win32.join(tempWin, `ShellXCutFinalAction-${stamp}`)
  const stage = linuxPath(stageWin); const verifierTempWin = win32.join(stageWin, 'verifier-temp')
  const mediaWin = win32.join(stageWin, 'media')
  const media = join(stage, 'media')
  const fixtureDir = join(stage, 'agent-fixtures')
  const fixtureWin = win32.join(stageWin, 'agent-fixtures')
  mkdirSync(media, { recursive: true })
  mkdirSync(fixtureDir, { recursive: true })
  const windowsBasePath = captureSync('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    "[Environment]::GetEnvironmentVariable('PATH','Machine') + ';' + [Environment]::GetEnvironmentVariable('PATH','User')",
  ])
  const localAppDataWin = captureSync('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command', '$env:LOCALAPPDATA',
  ])
  const webviewDataToken = `ShellXCutFinalAction-${stamp}`
  const webviewDataWin = win32.join(localAppDataWin, 'ShellX Cut Qualification', webviewDataToken)
  const webviewData = linuxPath(webviewDataWin)
  const adapterPythonWin = win32.join(
    localAppDataWin,
    'ShellX Cut',
    'perception',
    '.venv',
    'Scripts',
    'python.exe',
  )
  const staged = {}
  for (const [role, source] of Object.entries(roleArgs)) {
    const name = `${role}${source.slice(source.lastIndexOf('.')) || '.mp4'}`
    copyFileSync(source, join(media, name))
    staged[role] = win32.join(mediaWin, name)
  }
  let launched = false; let buildRuntime = null; let walkthrough = null
  try {
    if (!flag('--skip-build')) {
      const resolvedBuild = resolveWindowsBuildEnvironment({ cwd: ROOT })
      buildRuntime = resolvedBuild.receipt
      await run('bash', ['scripts/build-windows.sh', 'release'], {
        env: {
          ...resolvedBuild.env,
          TAURI_FEATURES: '',
          SHELLX_DISABLE_UPDATER_ARTIFACTS: signedFinal ? '0' : '1',
          SHELLX_WINDOWS_SIGNING_REQUIRED: signedFinal ? '1' : '0',
        },
      })
    }
    if (!existsSync(join(ROOT, 'ui/node_modules/playwright/package.json'))) await run('npm', ['--prefix', 'ui', 'ci', '--no-audit', '--no-fund'])
    const installer = findInstaller()
    const retainedInstaller = join(artifacts, basename(installer))
    copyFileSync(installer, retainedInstaller)
    const stagedInstaller = join(stage, 'installer', basename(installer))
    mkdirSync(dirname(stagedInstaller), { recursive: true })
    copyFileSync(retainedInstaller, stagedInstaller)
    const version = JSON.parse(readFileSync(
      join(ROOT, 'app/desktop/src-tauri/tauri.conf.json'),
      'utf8',
    )).version
    const head = captureSync('git', ['rev-parse', 'HEAD'])
    const status = captureSync('git', ['status', '--short'])
    const contentManifest = sourceContentManifest(ROOT)
    if (signedFinal && status) throw new Error('signed-final refuses a dirty tracked worktree')
    if (!flag('--skip-install')) {
      const installScript = windowsPath(join(ROOT, 'scripts/windows/install-cut-current.ps1'))
      const installArgs = [
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', installScript,
        '-SetupPath', windowsPath(stagedInstaller),
        '-ExpectedVersion', version,
      ]
      if (!signedFinal) installArgs.push('-AllowUnsignedSmoke')
      await run('powershell.exe', installArgs)
    }
    const windowsNode = resolveWindowsNode({ requested: arg('--windows-node', ''), cwd: ROOT })
    const harnessFfmpegWin = resolveInstalledHarnessFfmpeg({ cwd: ROOT })
    const qualificationEnv = prepareWindowsQualificationEnvironment({
      root: ROOT,
      fixtureDir,
      fixtureWin,
      harnessFfmpegWin,
      windowsBasePath,
      adapterPythonWin,
      stageWin,
      diarizeEndpoint: process.env.CUT_DIARIZE_ENDPOINT || '',
      dubEndpoint: process.env.CUT_DUB_ENDPOINT || '',
    })
    const cdpPort = normalizeCdpPort(arg('--cdp-port', '9223'))
    writeFileSync(join(out, 'source-receipt.json'), `${JSON.stringify({
      schema: 'shellx-cut/windows-installed-source@1',
      generatedAt: new Date().toISOString(),
      head,
      contentManifest: {
        files: contentManifest.files,
        bytes: contentManifest.bytes,
        sha256: contentManifest.sha256,
      },
      status: status ? status.split(/\r?\n/) : [],
      signedFinal,
      nativeUi: {
        unattendedFocusAllowed: unattendedNativeUi,
      },
      interactiveSession,
      verifierRuntime: {
        platform: windowsNode.platform,
        version: windowsNode.version,
        execPath: windowsNode.execPath,
      },
      buildRuntime,
      cdp: {
        port: cdpPort,
        optIn: 'SHELLX_CUT_WEBVIEW2_DEBUG_PORT',
        dataToken: webviewDataToken,
        dataDirectory: webviewDataWin,
      },
      installer: { name: basename(retainedInstaller), sha256: sha256(retainedInstaller) },
      media: Object.fromEntries(Object.entries(roleArgs).map(([role, path]) => [
        role, { source: path, sha256: sha256(path), enginePath: staged[role] },
      ])),
    }, null, 2)}\n`)
    const launch = launchInstalledCutWithCdp({
      cdpPort,
      stopExisting: true,
      env: {
        ...qualificationEnv,
        SHELLX_CUT_WEBVIEW2_DATA_TOKEN: webviewDataToken,
      },
    })
    if (launch.status !== 0) throw new Error(`installed launch failed: ${launch.stderr || launch.stdout}`)
    launched = true
    // Keep the parent WSL PATH intact while spawning native Windows Node.
    // Replacing it with a semicolon-delimited Windows PATH breaks WSL interop
    // from noninteractive SSH before the child can emit readiness diagnostics.
    const verifierEnv = { ...process.env }
    await proveWindowsInstalledReadiness({
      command: windowsNode.executableWsl,
      args: [windowsPath(join(ROOT, 'scripts/windows/launch-installed-cdp.mjs')), '--no-launch',
        '--cdp-port', String(cdpPort), '--engine', 'http://127.0.0.1:6161', '--wait-ms', '60000'],
      env: verifierEnv,
      cwd: ROOT,
      proofPath: join(out, 'installed-launch-proof.json'),
      proof: {
        source: { head, version },
        installer: { name: basename(retainedInstaller), sha256: sha256(retainedInstaller) },
        cdpPort,
      },
    })
    if (signedFinal) walkthrough = beginWindowsInstalledWalkthrough({
      root: ROOT, out, source: { gitCommit: head, version, contentManifestSha256: contentManifest.sha256 },
    })
    const nativeUiActivation = activateInstalledWindowForUnattendedRun({ root: ROOT })
    console.log(`NATIVE_UI_READY ${JSON.stringify(nativeUiActivation)}`)
    await run(windowsNode.executableWsl, [windowsPath(join(ROOT, 'ui/public-tests/full-coverage-verify.mjs'))], {
      env: forwardEnvToWindows(verifierEnv, {
        PATH: windowsBasePath,
        SWEEP_APP: 'http://127.0.0.1:6161',
        SWEEP_CUTD: 'http://127.0.0.1:6161',
        FCV_CDP_URL: `http://127.0.0.1:${cdpPort}`,
        FCV_UI_DRIVER: 'webview2-cdp',
        FCV_NATIVE_ACTION_CONTROLLER: windowsPath(join(ROOT, 'scripts/release/native-os-action-controller.mjs')),
        FCV_NATIVE_ACTION_PLATFORM: 'windows',
        // Windows TaskDialog acceptance can require one bounded PowerShell
        // timeout plus the exact-handle disappearance check and retry.
        FCV_NATIVE_ACTION_TIMEOUT_MS: '120000', FCV_IMPORT_DRAIN_TIMEOUT_MS: '600000',
        // Focused diagnostics own only their selected sections and must not be
        // blocked by unrelated heavy dependencies. Final qualification remains
        // strict and unfiltered.
        FCV_REQUIRE_FULL: diagnosticSection ? '0' : '1',
        FCV_FINAL_ALL_ACTIONS: diagnosticSection ? '0' : '1',
        FCV_REAL_SCREEN_RECORD: '1',
        FCV_INSTALLED_APP: '1',
        FCV_SOURCE_GIT_COMMIT: head,
        FCV_SOURCE_CONTENT_MANIFEST_SHA256: contentManifest.sha256,
        FCV_INSTALLED_RUNTIME_RECEIPT: signedFinal ? windowsPath(join(out, 'installed-runtime-receipt.json')) : '',
        FCV_AGENT_FIXTURES: '1',
        FCV_SECTION: diagnosticSection,
        FCV_TARGET_SURFACE: 'windows-installed',
        FCV_ACTION_MANIFEST: windowsPath(join(ROOT, 'ui/public-tests/full-ui-action-manifest.json')),
        FCV_SCREENS: windowsPath(join(out, 'screens')),
        FCV_RESULT_RECEIPT: windowsPath(join(out, 'full-coverage-receipt.json')), FCV_TMP_DIR: verifierTempWin, FCV_DEFER_TEMP_CLEANUP: '1',
        SHELLX_CUT_PROJECTS_DIR: qualificationEnv.SHELLX_CUT_PROJECTS_DIR,
        FCV_MEDIA_TIER: signedFinal ? 'real-release' : 'fixture-candidate', NODE_OPTIONS: '--max-old-space-size=8192',
        CUT_TEST_MEDIA_DIR: mediaWin,
        CUT_TEST_MEDIA_ENGINE_DIR: mediaWin,
        CUT_HARNESS_FFMPEG: harnessFfmpegWin,
        CUT_DIARIZE_ENDPOINT: qualificationEnv.CUT_DIARIZE_ENDPOINT, CUT_DUB_ENDPOINT: qualificationEnv.CUT_DUB_ENDPOINT,
        RELEASE_CLIP: staged.scene,
        RELEASE_CLIP_SPEECH: staged.speech,
        RELEASE_CLIP_FACE: staged.face,
        RELEASE_CLIP_SPEAKERS: staged.speakers,
        RELEASE_CLIP2: staged.second,
      }),
    })
    if (walkthrough) await finishWindowsInstalledWalkthrough({ root: ROOT, out, session: walkthrough })
  } finally {
    if (launched) stopInstalledProcesses()
    if (cleanAfter) {
      const removeOptions = { recursive: true, force: true, maxRetries: 20, retryDelay: 250 }
      // A bounded retry prevents a late Windows handle from replacing the verifier's real error.
      rmSync(stage, removeOptions)
      releaseWindowsWebviewProfile(webviewDataToken)
      rmSync(webviewData, removeOptions)
      for (const path of ['app/target', 'app/desktop/src-tauri/target', 'ui/node_modules', 'ui/dist']) {
        rmSync(join(ROOT, path), removeOptions)
      }
    }
  }
}

main().catch((error) => {
  console.error(error?.stack || error?.message || String(error))
  process.exit(1)
})
