import assert from 'node:assert/strict'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import { forwardEnvToWindows } from '../lib/wsl-interop-env.mjs'
import { prepareWindowsBuildEnvironment, prepareWindowsQualificationEnvironment, proveWindowsInstalledReadiness } from '../lib/windows-installed-qualification.mjs'

const runnerUrl = new URL('../windows-installed-full-coverage.mjs', import.meta.url)
const windowsBuildUrl = new URL('../build-windows.sh', import.meta.url)
const activatorUrl = new URL('../release/activate-installed-windows.ps1', import.meta.url)
const qualificationHelperUrl = new URL('../lib/windows-installed-qualification.mjs', import.meta.url)

test('Windows verifier explicitly forwards its WSL environment to native Node', () => {
  const env = forwardEnvToWindows(
    { WSLENV: 'EXISTING/u:PATH/l', EXISTING: 'keep' },
    { FCV_REQUIRE_FULL: 1, FCV_CDP_URL: 'http://127.0.0.1:9223', EMPTY: '' },
  )
  assert.equal(env.FCV_REQUIRE_FULL, '1')
  assert.equal(env.FCV_CDP_URL, 'http://127.0.0.1:9223')
  assert.equal(env.EMPTY, '')
  assert.equal(env.WSLENV, 'EXISTING/u:PATH/l:FCV_REQUIRE_FULL:FCV_CDP_URL:EMPTY')
  assert.equal(env.EXISTING, 'keep')
})

test('Windows installed environment stages deterministic adapters and native ffmpeg', async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-windows-env-'))
  const fixtureDir = join(root, 'stage')
  await mkdir(join(root, 'scripts/release/fixtures'), { recursive: true })
  await mkdir(join(root, 'ui/public-tests/fixtures'), { recursive: true })
  await mkdir(fixtureDir)
  await writeFile(join(root, 'scripts/release/fixtures', 'codex'), 'fixture')
  await writeFile(join(root, 'ui/public-tests/fixtures', 'generate-prompt-adapter.py'), 'prompt')
  await writeFile(join(root, 'ui/public-tests/fixtures', 'generate-storyboard-adapter.py'), 'storyboard')
  try {
    const env = prepareWindowsQualificationEnvironment({
      root,
      fixtureDir,
      fixtureWin: 'C:\\stage',
      harnessFfmpegWin: 'C:\\tools\\ffmpeg\\bin\\ffmpeg.exe',
      windowsBasePath: 'C:\\Windows\\System32',
      adapterPythonWin: 'C:\\perception\\python.exe',
      stageWin: 'C:\\candidate',
    })
    assert.equal(await readFile(join(fixtureDir, 'codex'), 'utf8'), 'fixture')
    assert.equal(await readFile(join(fixtureDir, 'generate-storyboard-adapter.py'), 'utf8'), 'storyboard')
    assert.equal(env.FFMPEG_BIN, 'C:\\tools\\ffmpeg\\bin\\ffmpeg.exe')
    assert.equal(env.CUTD_GENERATE_PROMPT_ADAPTER, 'C:\\stage\\generate-prompt-adapter.py')
    assert.match(env.PATH, /^C:\\stage;C:\\tools\\ffmpeg\\bin;/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test('Windows build environment pins every WSL and Windows toolchain path', () => {
  const commands = Object.fromEntries([
    'node', 'npm', 'cargo', 'cargo-xwin', 'clang-cl', 'lld-link',
    'llvm-config', 'llvm-lib', 'llvm-rc', 'makensis', 'powershell.exe', 'wslpath',
  ].map((name, index) => [name, `/explicit/tool-${index}/${name}`]))
  const env = prepareWindowsBuildEnvironment({
    baseEnv: { PATH: '/ambient/bin', LD_LIBRARY_PATH: '/ambient/lib' },
    commandPaths: commands,
    llvmLibDir: '/usr/lib/llvm-release/lib',
    windowsSystem32: '/mnt/c/Windows/System32',
  })
  for (const path of Object.values(commands)) assert.match(env.PATH, new RegExp(path.slice(0, path.lastIndexOf('/'))))
  assert.match(env.PATH, /[/]mnt[/]c[/]Windows[/]System32/)
  assert.match(env.LD_LIBRARY_PATH, /^[/]usr[/]lib[/]llvm-release[/]lib:/)
})

test('Windows build fails before compilation when portable NSIS cannot load its toolset', async () => {
  const source = await readFile(windowsBuildUrl, 'utf8')
  const preflight = source.indexOf('makensis -HDRINFO')
  const uiBuild = source.indexOf('[build-windows] building ui/dist')
  assert.ok(preflight >= 0 && preflight < uiBuild)
  assert.match(source, /readlink -f "[$][(]command -v makensis[)]"/)
  assert.match(source, /Portable NSIS detected/)
  assert.match(source, /sudo ln -s '[$]portable_nsis_root' [/ ]usr[/]share[/]nsis/)
})

test('Windows installed readiness writes a pre-row proof only after every marker', async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-windows-readiness-'))
  const proofPath = join(root, 'installed-launch-proof.json')
  try {
    await proveWindowsInstalledReadiness({
      command: process.execPath,
      args: ['-e', "console.log('CDP_READY\\nCUTD_READY\\nAGENT_DOCS_READY')"],
      env: process.env,
      cwd: root,
      proofPath,
      proof: { source: { head: 'abc', version: '0.6.105' }, cdpPort: 9223 },
    })
    const proof = JSON.parse(await readFile(proofPath, 'utf8'))
    assert.deepEqual(proof.markers, { cdp: true, engine: true, agentDocs: true })
    assert.equal(proof.uiRowsStarted, false)
    assert.equal(proof.source.head, 'abc')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test('Windows installed runner is one unfiltered exact-artifact action gate', async () => {
  const source = await readFile(runnerUrl, 'utf8')
  const qualificationHelper = await readFile(qualificationHelperUrl, 'utf8')
  assert.match(source, /FCV_SECTION and FCV_ONLY/)
  assert.match(source, /FCV_REQUIRE_FULL: diagnosticSection [?:] '0' [?:] '1'/)
  assert.match(source, /FCV_FINAL_ALL_ACTIONS: diagnosticSection [?:] '0' [?:] '1'/)
  assert.match(source, /FCV_INSTALLED_APP: '1'/)
  assert.match(source, /FCV_UI_DRIVER: 'webview2-cdp'/)
  assert.match(source, /FCV_NATIVE_ACTION_PLATFORM: 'windows'/)
  assert.match(source, /FCV_REAL_SCREEN_RECORD: '1'/)
  assert.match(source, /NODE_OPTIONS: '--max-old-space-size=8192'/)
  assert.match(source, /FCV_MEDIA_TIER: signedFinal [?:] 'real-release' [?:] 'fixture-candidate'/)
  assert.match(source, /assertWindowsInteractiveSession/)
  assert.match(source, /requires an unlocked interactive desktop session/)
  assert.match(source, /--unattended-native-ui/)
  assert.match(source, /refusing to steal focus from an unknown user session/)
  assert.match(source, /activateInstalledWindowForUnattendedRun/)
  assert.match(source, /windows-installed-qualification[.]mjs/)
  assert.match(qualificationHelper, /activate-installed-windows[.]ps1/)
  assert.match(source, /prepareWindowsQualificationEnvironment/)
  assert.match(qualificationHelper, /CUTD_GENERATE_PROMPT_ADAPTER/)
  assert.match(qualificationHelper, /CUTD_GENERATE_STORYBOARD_ADAPTER/)
  assert.match(qualificationHelper, /FFMPEG_BIN: harnessFfmpegWin/)
  assert.match(qualificationHelper, /SHELLX_CUT_FFMPEG: harnessFfmpegWin/)
  assert.match(qualificationHelper, /foregroundProcess/)
  assert.match(source, /NATIVE_UI_READY/)
  assert.match(source, /interactiveSession/)
  assert.match(source, /install-cut-current[.]ps1/)
  assert.match(source, /stagedInstaller = join[(]stage, 'installer'/)
  assert.match(source, /'-SetupPath', windowsPath[(]stagedInstaller[)]/)
  assert.match(source, /launchInstalledCutWithCdp/)
  assert.match(source, /resolveWindowsBuildEnvironment/)
  assert.match(qualificationHelper, /requires active WSL interop/)
  assert.match(qualificationHelper, /prepareWindowsBuildEnvironment/)
  assert.match(qualificationHelper, /explicitCommands/)
  assert.match(source, /installed-launch-proof[.]json/)
  assert.match(qualificationHelper, /uiRowsStarted: false/)
  assert.match(qualificationHelper, /CDP_READY/)
  assert.match(qualificationHelper, /CUTD_READY/)
  assert.match(qualificationHelper, /AGENT_DOCS_READY/)
  assert.match(
    source,
    /installed-launch-proof[.]json[\s\S]+activateInstalledWindowForUnattendedRun[\s\S]+full-coverage-verify[.]mjs/,
    'installed readiness proof must be written before any full UI action row starts',
  )
  assert.match(qualificationHelper, /requires native Windows Node/)
  assert.match(source, /windowsNode[.]executableWsl/)
  assert.match(source, /const verifierEnv = \{ [.][.][.]process[.]env \}/)
  assert.doesNotMatch(source, /const verifierEnv = \{ [.][.][.]process[.]env, PATH: windowsBasePath \}/)
  assert.match(source, /forwardEnvToWindows[(]verifierEnv/)
  assert.match(source, /forwardEnvToWindows[(]verifierEnv, \{[\s\S]+PATH: windowsBasePath,/)
  assert.match(source, /verifierRuntime/)
  assert.match(source, /SHELLX_CUT_WEBVIEW2_DEBUG_PORT/)
  assert.match(source, /SHELLX_CUT_WEBVIEW2_DATA_TOKEN/)
  assert.match(source, /dataDirectory: webviewDataWin/)
  assert.match(source, /CUT_HARNESS_FFMPEG: harnessFfmpegWin/)
  assert.match(source, /FCV_NATIVE_ACTION_CONTROLLER: windowsPath/)
  assert.match(source, /FCV_NATIVE_ACTION_TIMEOUT_MS: '120000'/)
  assert.match(source, /FCV_ACTION_MANIFEST: windowsPath/)
  assert.match(source, /FCV_RESULT_RECEIPT: windowsPath/)
  assert.match(source, /FCV_TMP_DIR: verifierTempWin/)
  assert.match(source, /SHELLX_CUT_PROJECTS_DIR: qualificationEnv[.]SHELLX_CUT_PROJECTS_DIR/)
  assert.match(source, /FCV_DEFER_TEMP_CLEANUP: '1'/)
  assert.match(source, /source-receipt[.]json/)
  assert.match(source, /sourceContentManifest/)
  assert.match(source, /FCV_SOURCE_CONTENT_MANIFEST_SHA256/)
  assert.match(source, /speakers: arg[(]'--speakers', join[(]ROOT, 'testdata[/]talking_head[.]mp4'[)]/)
})

test('unattended Windows focus activator targets one validated app window', async () => {
  const source = await readFile(activatorUrl, 'utf8')
  assert.match(source, /Get-Process [$]ExpectedProcessName/)
  assert.match(source, /[$]matches[.]Count -ne 1/)
  assert.match(source, /[$]targetPid -ne [$]process[.]Id/)
  assert.match(source, /AttachThreadInput/)
  assert.match(source, /SetForegroundWindow/)
  assert.match(source, /SwitchToThisWindow/)
  assert.match(source, /[$]landedPid -ne [$]process[.]Id/)
  assert.doesNotMatch(source, /Get-Process .*explorer/)
})

test('Windows focused diagnostics cannot masquerade as final qualification', async () => {
  const source = await readFile(runnerUrl, 'utf8')
  assert.match(source, /--diagnostic-section/)
  assert.match(source, /--signed-final forbids --diagnostic-section/)
  assert.match(source, /FCV_REQUIRE_FULL: diagnosticSection [?:] '0' [?:] '1'/)
  assert.match(source, /FCV_SECTION: diagnosticSection/)
  assert.match(
    source,
    /CUT_DIARIZE_ENDPOINT: qualificationEnv[.]CUT_DIARIZE_ENDPOINT[\s\S]+CUT_DUB_ENDPOINT: qualificationEnv[.]CUT_DUB_ENDPOINT/,
    'native Windows verifier receives the same tunneled service endpoints as the installed engine',
  )
})

test('Windows runner separates unsigned candidates from the signed final gate', async () => {
  const source = await readFile(runnerUrl, 'utf8')
  assert.match(source, /--signed-final/)
  assert.match(source, /SHELLX_DISABLE_UPDATER_ARTIFACTS: signedFinal [?:] '0' [?:] '1'/)
  assert.match(source, /SHELLX_WINDOWS_SIGNING_REQUIRED: signedFinal [?:] '1' [?:] '0'/)
  assert.match(source, /if [(]!signedFinal[)] installArgs[.]push[(]'-AllowUnsignedSmoke'[)]/)
  assert.match(source, /signed-final refuses a dirty tracked worktree/)
  assert.match(source, /requires --[$]\{role\} with real release media/)
  assert.match(source, /if [(]signedFinal[)] walkthrough = beginWindowsInstalledWalkthrough/)
  assert.match(source, /if [(]walkthrough[)] await finishWindowsInstalledWalkthrough/)
  assert.match(source, /FCV_INSTALLED_RUNTIME_RECEIPT: signedFinal/)
  assert.match(source, /FCV_TARGET_SURFACE: 'windows-installed'/)
})

test('Windows runner preserves evidence while cleaning exact rebuildable state', async () => {
  const source = await readFile(runnerUrl, 'utf8')
  assert.match(source, /--clean-after/)
  assert.match(source, /rmSync[(]stage/)
  assert.match(source, /maxRetries: 20/)
  assert.match(source, /bounded retry prevents[\s\S]{0,80}replacing the verifier's real error/)
  assert.match(
    source,
    /if [(]launched[)] stopInstalledProcesses[(][)][\s\S]{0,180}if [(]cleanAfter[)][\s\S]{0,520}rmSync[(]stage/,
    'runner stops installed processes before removing its exact verifier temp directory inside the stage',
  )
  assert.match(source, /releaseWindowsWebviewProfile[(]webviewDataToken[)]/)
  assert.match(source, /rmSync[(]webviewData/)
  assert.match(source, /'app[/]target'/)
  assert.match(source, /'app[/]desktop[/]src-tauri[/]target'/)
  assert.match(source, /'ui[/]node_modules'/)
  assert.doesNotMatch(source, /rmSync[(]out/)
})
