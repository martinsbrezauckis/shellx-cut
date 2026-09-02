import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { EventEmitter } from 'node:events'
import { chmodSync, existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { delimiter, dirname, join, resolve } from 'node:path'
import test from 'node:test'

import {
  B1_CONTEXT_ASSERTIONS, B1_SOURCE_MONITOR_ASSERTIONS, B2_RESULTS, EXPECTED_ROWS,
  assertCleanSourceIdentity, assertContextMenuLog, assertNoAmbientRuntimeOverrides,
  assertRuntimeAgent, assertRuntimeSealedOutputPath, assertSourceMonitorLog,
  assertVolumeAutomationReceipt, parseRuntimeSealedArgs, validateRuntimeSealedArgs,
} from '../lib/runtime-sealed-b1-b2-contract.mjs'
import {
  assertPosixShebangPath, assertStableNpmScriptShell, assertStableSealedArtifact,
  boundedEvidenceFiles, createOwnedOutputRoot, materializeBuildToolBin, npmScriptShellSemantics,
  removeOwnedTree, resolveNativeBuildTools, resolveNpmScriptShell, resolveRustupToolchain, sealedRegularFile, sealedTree,
} from '../lib/runtime-sealed-b1-b2-files.mjs'
import { createOwnedSystemTemporaryRoot, sealedSystemTemporaryParent } from '../lib/runtime-sealed-b1-b2-owned-temp.mjs'
import { createBoundedChildController, startLoopbackCutd } from '../lib/runtime-sealed-b1-b2-process.mjs'
import { collectTools, sealedEnv } from '../runtime-sealed-b1-b2-gate.mjs'
import {
  REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM, SEALED_PLAYWRIGHT_CHROMIUM_PATH,
  SEALED_PLAYWRIGHT_CHROMIUM_SHA256, sealedPlaywrightChromiumLaunchOptions,
} from '../../ui/public-tests/lib/sealedPlaywrightChromium.mjs'

const candidate = {
  id: 'runtime-sealed-b1-b2-deadbeef', sourceCommit: 'a'.repeat(40), sourceTree: 'b'.repeat(40),
  worktree: '/work/shellx-cut', contentManifestSha256: 'c'.repeat(64), testControlManifestSha256: 'd'.repeat(64),
  fixtureId: 'runtime-sealed-b2-volume-fixture', fixtureSeedSha256: 'e'.repeat(64), runnerId: 'runtime-sealed-b1-b2-browser',
}
const source = {
  gitCommit: candidate.sourceCommit, gitTree: candidate.sourceTree, gitDirty: false,
  contentManifest: { sha256: candidate.contentManifestSha256, files: 12, bytes: 34 },
  actionManifest: { sha256: candidate.testControlManifestSha256, bytes: 56 },
}

function volumeReceipt() {
  const actionIds = ['volume-automation-add', 'volume-automation-clear', 'volume-automation-interpolation', 'volume-automation-level', 'volume-automation-point', 'volume-automation-remove', 'volume-automation-time']
  return {
    schema: 'shellx-cut/full-coverage-results@1', ok: true, full: false, strictAllActions: false,
    runtime: { installedApp: false, nativeAttached: false, driver: 'playwright-chromium', sourceGitCommit: source.gitCommit, sourceContentManifestSha256: source.contentManifest.sha256 },
    candidate: structuredClone(candidate),
    summary: { dimensions: { present: { pass: 12, fail: 0, na: 1 }, render: { pass: 8, fail: 0, na: 5 }, click: { pass: 8, fail: 0, na: 5 }, result: { pass: 13, fail: 0, na: 0 } }, controls: { total: 13, uiActions: 8, supportRows: 5, fullyVerified: 13, delegated: 0, dependencySkips: 0, optionalAgentSkips: 0, guards: 0, couldNotVerify: 0, strictUnverified: 0, focusedUnverified: 0, failures: 0 } },
    results: B2_RESULTS.map(([actionId, rowKind, surface, name, present, render, click, result]) => ({ actionId, rowKind, surface, name, present, render, click, result, ok: true, classification: 'fully_verified', evidence: 'sealed', shot: null })),
    actionManifest: { algorithm: 'sha256', sha256: createHash('sha256').update(JSON.stringify(actionIds)).digest('hex'), total: 7, occurrences: 8, observed: actionIds, repeated: [{ id: 'volume-automation-add', count: 2 }] },
  }
}

function sourceLog() { return B1_SOURCE_MONITOR_ASSERTIONS.map((name) => `PASS  ${name}  receipt`).join('\n') }
function contextLog() { return [...B1_CONTEXT_ASSERTIONS.map((name) => `PASS ${name}  receipt`), 'PASS context-menu surfaces — 34 pass / 0 fail'].join('\n') }
function temp() { return mkdtempSync(join(tmpdir(), 'runtime-sealed-b1-b2-')) }
class FakeChild extends EventEmitter { kill() { return true } }

test('runtime-sealed CLI makes fixture identities mandatory and keeps build/consume exclusive', () => {
  const args = parseRuntimeSealedArgs(['--cutd', '/candidate/cutd', '--ui-dist', '/candidate/ui', '--base-clip', '/media/base.mp4', '--insert-clip', '/media/insert.mp4', '--context-fixture', '/media/context.mp4', '--volume-fixture', '/media/volume.mp4'])
  assert.equal(validateRuntimeSealedArgs(args).mode, 'consume')
  assert.throws(() => validateRuntimeSealedArgs({ ...args, build: true }), /cannot be combined/)
  assert.throws(() => validateRuntimeSealedArgs({ ...args, contextFixture: '' }), /context-fixture is required/)
})

test('runtime-sealed outputs cannot be reused or leave the checkout scratch root', () => {
  assert.equal(assertRuntimeSealedOutputPath('/work/shellx-cut', '.scratch/runtime-sealed-b1-b2/run-a', '.scratch/runtime-sealed-b1-b2'), '/work/shellx-cut/.scratch/runtime-sealed-b1-b2/run-a')
  assert.throws(() => assertRuntimeSealedOutputPath('/work/shellx-cut', '.scratch/other/run-a', '.scratch/runtime-sealed-b1-b2'), /below/)
  assert.throws(() => assertRuntimeSealedOutputPath('/work/shellx-cut', '.scratch/runtime-sealed-b1-b2', '.scratch/runtime-sealed-b1-b2'), /below/)
})

test('source identity rejects dirty, tree, content, and action-manifest drift', () => {
  assert.equal(assertCleanSourceIdentity(source, structuredClone(source)), true)
  assert.throws(() => assertCleanSourceIdentity({ ...source, gitDirty: true }), /clean committed/)
  assert.throws(() => assertCleanSourceIdentity(source, { ...source, gitTree: 'z'.repeat(40) }), /gitTree changed/)
  assert.throws(() => assertCleanSourceIdentity(source, { ...source, contentManifest: { ...source.contentManifest, sha256: 'f'.repeat(64) } }), /content manifest sha256 changed/)
  assert.throws(() => assertCleanSourceIdentity(source, { ...source, actionManifest: { ...source.actionManifest, bytes: 57 } }), /action manifest bytes changed/)
})

test('B1 assertion inventory is exact and cannot be satisfied by generic pass counts', () => {
  assert.equal(assertSourceMonitorLog(sourceLog()).observedRows, EXPECTED_ROWS.sourceMonitor)
  assert.equal(assertContextMenuLog(contextLog()).observedRows, EXPECTED_ROWS.contextMenus)
  assert.throws(() => assertSourceMonitorLog(sourceLog().replace(B1_SOURCE_MONITOR_ASSERTIONS[0], 'semantic bypass')), /inventory drifted/)
  assert.throws(() => assertContextMenuLog(contextLog().replace(B1_CONTEXT_ASSERTIONS[4], 'semantic bypass')), /inventory drifted/)
})

test('B2 receipt seals every exact result/action row and repeated add semantics', () => {
  assert.equal(assertVolumeAutomationReceipt(volumeReceipt(), { source, candidate }).observedRows, EXPECTED_ROWS.volumeAutomation)
  const extra = volumeReceipt(); extra.results.push(structuredClone(extra.results[0]))
  assert.throws(() => assertVolumeAutomationReceipt(extra, { source, candidate }), /result inventory/)
  const collapsed = volumeReceipt(); collapsed.actionManifest.repeated = []
  assert.throws(() => assertVolumeAutomationReceipt(collapsed, { source, candidate }), /repeated add semantics/)
  const failed = volumeReceipt(); failed.summary.dimensions.result.fail = 1
  assert.throws(() => assertVolumeAutomationReceipt(failed, { source, candidate }), /partial, unverified, failed/)
})

test('artifact sealing rejects nested redirects and detects tool-content drift', () => {
  const root = temp()
  try {
    const served = join(root, 'ui'); mkdirSync(served); writeFileSync(join(served, 'index.html'), 'safe')
    const elsewhere = join(root, 'elsewhere'); mkdirSync(elsewhere); writeFileSync(join(elsewhere, 'redirected.js'), 'bad')
    symlinkSync(join(elsewhere, 'redirected.js'), join(served, 'redirected.js'))
    assert.throws(() => sealedTree(served, 'served UI'), /symbolic link/)
    const evidence = join(root, 'evidence'); mkdirSync(evidence); symlinkSync(join(elsewhere, 'redirected.js'), join(evidence, 'result.json'))
    assert.throws(() => boundedEvidenceFiles(evidence), /symbolic link/)
    const repo = join(root, 'repo'); mkdirSync(repo); symlinkSync(elsewhere, join(repo, '.scratch'))
    assert.throws(() => createOwnedOutputRoot(repo, '.scratch/runtime-sealed-b1-b2', ''), /symbolic link/)
    const tool = join(root, 'tool'); writeFileSync(tool, 'first'); const before = sealedRegularFile(tool, 'tool')
    writeFileSync(tool, 'second'); assert.throws(() => assertStableSealedArtifact('tool', before, sealedRegularFile(tool, 'tool')), /drifted/)
    const owned = join(root, 'owned'); mkdirSync(owned); writeFileSync(join(elsewhere, 'preserved'), 'outside')
    symlinkSync(join(elsewhere, 'preserved'), join(owned, 'owned-link'))
    assert.throws(() => removeOwnedTree(root, owned, 'owned link tree'), /symbolic link/)
    assert.equal(readFileSync(join(elsewhere, 'preserved'), 'utf8'), 'outside')
    assert.equal(removeOwnedTree(root, owned, 'owned link tree', { allowOwnedLinks: true }).status, 'removed')
    assert.equal(readFileSync(join(elsewhere, 'preserved'), 'utf8'), 'outside')
    const firstBin = join(root, 'first-bin'); const secondBin = join(root, 'second-bin'); mkdirSync(firstBin); mkdirSync(secondBin)
    for (const path of [join(firstBin, 'cc'), join(secondBin, 'cc')]) { writeFileSync(path, '#!/bin/sh\nexit 0\n'); chmodSync(path, 0o700) }
    assert.throws(() => resolveNativeBuildTools(root, { PATH: `${firstBin}${delimiter}${secondBin}` }), /exactly one regular target/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('sealed Playwright Chromium launch uses one exact executable and refuses cache, missing, and drift paths', () => {
  const root = temp()
  try {
    const withSpaces = join(root, 'browser with spaces'); mkdirSync(withSpaces)
    const chromium = join(withSpaces, 'chromium fixture')
    writeFileSync(chromium, '#!/bin/sh\nexit 0\n'); chmodSync(chromium, 0o700)
    const hash = createHash('sha256').update(readFileSync(chromium)).digest('hex')
    const sealedEnv = {
      [REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM]: '1',
      [SEALED_PLAYWRIGHT_CHROMIUM_PATH]: chromium,
      [SEALED_PLAYWRIGHT_CHROMIUM_SHA256]: hash,
    }
    assert.deepEqual(sealedPlaywrightChromiumLaunchOptions(sealedEnv), { executablePath: chromium })
    assert.throws(() => sealedPlaywrightChromiumLaunchOptions({ [REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM]: '1' }), /requires SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_PATH/)
    assert.throws(() => sealedPlaywrightChromiumLaunchOptions({ ...sealedEnv, PLAYWRIGHT_BROWSERS_PATH: join(root, 'ambient-cache') }), /forbids PLAYWRIGHT_BROWSERS_PATH/)
    writeFileSync(chromium, '#!/bin/sh\necho drift\n'); chmodSync(chromium, 0o700)
    assert.throws(() => sealedPlaywrightChromiumLaunchOptions(sealedEnv), /SHA-256 drifted/)
    assert.throws(() => sealedPlaywrightChromiumLaunchOptions({ ...sealedEnv, [SEALED_PLAYWRIGHT_CHROMIUM_PATH]: join(withSpaces, 'missing chromium') }), /does not exist/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('runtime-sealed browser profiles use one short owned system-temp child and clean it recursively', () => {
  const temporary = createOwnedSystemTemporaryRoot()
  try {
    assert.equal(dirname(temporary.path), temporary.parent)
    assert.match(temporary.path, /shellx-cut-runtime-sealed-/)
    assert.deepEqual(temporary.allocation, { mechanism: 'mkdtemp', prefix: 'shellx-cut-runtime-sealed-', leaf: temporary.allocation.leaf })
    assert.match(temporary.allocation.leaf, /^shellx-cut-runtime-sealed-/)
    assert.equal(temporary.initialTree.path, temporary.path)
    assert.equal(temporary.initialTree.files, 0)
    if (process.platform !== 'win32') assert.ok(Buffer.byteLength(temporary.path) <= 60)
    writeFileSync(join(temporary.path, 'profile-state'), 'owned')
    assert.equal(removeOwnedTree(temporary.parent, temporary.path, 'flow-owned browser temporary state').status, 'removed')
    assert.equal(existsSync(temporary.path), false)
  } finally {
    if (existsSync(temporary.path)) rmSync(temporary.path, { recursive: true, force: true })
  }
})

test('runtime-sealed browser temporary root refuses reparse parents and cleans up an overlong owned child', () => {
  const root = temp()
  try {
    const target = join(root, 'real'); mkdirSync(target)
    const redirected = join(root, 'redirected')
    symlinkSync(target, redirected)
    assert.throws(() => createOwnedSystemTemporaryRoot(redirected), /symbolic link/)
    const overlong = join(root, 'x'.repeat(80)); mkdirSync(overlong)
    if (process.platform !== 'win32') {
      assert.throws(() => createOwnedSystemTemporaryRoot(overlong), /too long for Chromium/)
      assert.deepEqual(readdirSync(overlong), [])
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('sealed browser temp parent is canonical on POSIX and derives only from Windows SystemRoot', () => {
  if (process.platform !== 'win32') assert.equal(sealedSystemTemporaryParent(), resolve(realpathSync.native('/tmp')))
  assert.equal(sealedSystemTemporaryParent({ platform: 'win32', systemRoot: 'C:\\Windows' }), 'C:\\Windows\\Temp')
  assert.throws(() => sealedSystemTemporaryParent({ platform: 'win32', systemRoot: '' }), /SystemRoot/)
  assert.throws(() => sealedSystemTemporaryParent({ platform: 'win32', systemRoot: 'Windows' }), /SystemRoot/)
})

test('cutd startup errors, early exits, and cleanup timeouts fail closed', async () => {
  const root = temp()
  try {
    for (const event of ['error', 'exit']) {
      const child = new FakeChild(); queueMicrotask(() => event === 'error' ? child.emit('error', new Error('no spawn')) : child.emit('exit', 1, null))
      await assert.rejects(() => startLoopbackCutd({ cutd: '/candidate/cutd', uiDist: '/candidate/ui', env: { SHELLX_CUT_WORKTREE: root }, logPath: join(root, `${event}.log`), spawnFn: () => child, readyMs: 50 }), event === 'error' ? /startup error/ : /exited before readiness/)
    }
    let closes = 0; const controller = createBoundedChildController({ child: new FakeChild(), closeLog: () => { closes += 1 }, waitMs: 1 })
    const first = controller.stop(); assert.strictEqual(first, controller.stop())
    assert.equal((await first).status, 'cleanup-failed'); assert.equal(closes, 1)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('build preflight seals and invokes the canonical Rustup dispatcher without ambient state', () => {
  const rustup = resolveRustupToolchain({})
  const env = { PATH: dirname(rustup.cargo.invocation.path), CARGO_HOME: rustup.cargoHome, RUSTUP_HOME: rustup.rustupHome, RUSTUP_TOOLCHAIN: rustup.toolchain.name }
  for (const command of ['cargo', 'rustc']) {
    const tool = rustup[command]
    assert.equal(tool.invocation.kind, 'rustup-dispatcher-link')
    assert.equal(tool.invocation.dispatcher.sha256, rustup.dispatcher.sha256)
    const result = spawnSync(tool.invocation.path, ['--version'], { encoding: 'utf8', env, timeout: 15_000 })
    assert.equal(result.status, 0, String(result.stderr || result.error || ''))
    assert.match(String(result.stdout), new RegExp(`^${command} `))
  }
  const preflight = collectTools(true, process.execPath)
  assert.equal(preflight.artifacts.cargo.resolved.sha256, rustup.cargo.resolved.sha256)
  assert.equal(preflight.artifacts.rustc.resolved.sha256, rustup.rustc.resolved.sha256)
  const npmShell = resolveNpmScriptShell({ PATH: process.env.PATH })
  assert.equal(preflight.artifacts.npmShell.resolved.sha256, npmShell.resolved.sha256)
  assert.equal(preflight.artifacts.npmShell.command, process.platform === 'win32' ? 'cmd.exe' : 'sh')
  assert.match(preflight.artifacts.npmShell.resolved.sha256, /^[a-f0-9]{64}$/)
  const root = temp()
  try {
    if (process.platform !== 'win32') assert.throws(() => materializeBuildToolBin(preflight.artifacts.nativeBuild, preflight.artifacts.node, join(root, 'missing-shell-tools')), /sealed shell/)
    const buildTools = materializeBuildToolBin(preflight.artifacts.nativeBuild, preflight.artifacts.node, join(root, 'native-tools'), process.platform === 'win32' ? null : preflight.artifacts.npmShell)
    const buildEnv = sealedEnv(preflight, join(root, 'home'), buildTools)
    mkdirSync(buildEnv.HOME)
    assert.equal(buildEnv.PATH, buildTools.path)
    assert.ok(!buildEnv.PATH.split(delimiter).includes(dirname(preflight.artifacts.npmShell.invocation.path)))
    assert.equal(buildTools.tools.npmShell, undefined)
    assert.equal(buildEnv.CC, buildTools.tools.cc.path)
    assert.equal(buildEnv.PKG_CONFIG, buildTools.tools.pkgConfig.path)
    const c = join(root, 'answer.c'); const object = join(root, 'answer.o'); const archive = join(root, 'libanswer.a'); const rust = join(root, 'main.rs'); const binary = join(root, 'sealed-link-proof')
    writeFileSync(c, 'int sealed_answer(void) { return 42; }\n')
    writeFileSync(rust, 'unsafe extern "C" { fn sealed_answer() -> i32; }\nfn main() { assert_eq!(unsafe { sealed_answer() }, 42); }\n')
    for (const [tool, args] of [[buildTools.tools.cc.path, ['-c', c, '-o', object]], [buildTools.tools.ar.path, ['rcs', archive, object]], [buildTools.tools.ld.path, process.platform === 'darwin' ? ['-v'] : ['--version']], [buildTools.tools.pkgConfig.path, ['--version']], [preflight.artifacts.rustc.invocation.path, [rust, '-L', `native=${root}`, '-l', 'static=answer', '-o', binary]]]) {
      const result = spawnSync(tool, args, { cwd: root, encoding: 'utf8', env: buildEnv, timeout: 30_000 })
      assert.equal(result.status, 0, String(result.stderr || result.error || ''))
    }
    const linked = spawnSync(binary, [], { cwd: root, encoding: 'utf8', env: buildEnv, timeout: 15_000 })
    assert.equal(linked.status, 0, String(linked.stderr || linked.error || ''))
    const cargoProject = join(root, 'gate-preflight'); const cargoSource = join(cargoProject, 'src'); mkdirSync(cargoProject); mkdirSync(cargoSource)
    writeFileSync(join(cargoProject, 'Cargo.toml'), '[package]\nname = "sealed_build_preflight"\nversion = "0.1.0"\nedition = "2021"\n')
    writeFileSync(join(cargoSource, 'main.rs'), 'fn main() { println!("sealed build preflight"); }\n')
    const cargo = spawnSync(preflight.artifacts.cargo.invocation.path, ['build', '--offline', '--manifest-path', join(cargoProject, 'Cargo.toml'), '--target-dir', join(root, 'cargo-target')], { cwd: root, encoding: 'utf8', env: buildEnv, timeout: 60_000 })
    assert.equal(cargo.status, 0, String(cargo.stderr || cargo.error || ''))
    const npmProject = join(root, 'npm-script'); mkdirSync(npmProject)
    writeFileSync(join(npmProject, 'package.json'), '{"scripts":{"proof":"node -e \\\"console.log(\'sealed-npm-shell-proof\')\\\""}}\n')
    const npmArgs = [preflight.artifacts.npmCli.path, '--prefix', npmProject, 'run', 'proof', '--script-shell', preflight.artifacts.npmShell.resolved.path]
    const npm = spawnSync(preflight.artifacts.node.path, npmArgs, { cwd: root, encoding: 'utf8', env: buildEnv, timeout: 30_000 })
    assert.equal(npm.status, 0, String(npm.stderr || npm.error || ''))
    assert.match(String(npm.stdout), /sealed-npm-shell-proof/)
    const missingShell = spawnSync(preflight.artifacts.node.path, [...npmArgs.slice(0, -1), join(root, 'missing-shell')], { cwd: root, encoding: 'utf8', env: buildEnv, timeout: 30_000 })
    assert.notEqual(missingShell.status, 0)
    assert.throws(() => assertStableNpmScriptShell(preflight.artifacts.npmShell, { ...preflight.artifacts.npmShell, resolved: { ...preflight.artifacts.npmShell.resolved, sha256: '0'.repeat(64) } }), /drifted/)
    writeFileSync(buildTools.tools.cc.path, 'drift\n')
    assert.throws(() => assertStableSealedArtifact('governed build-tool bin', buildTools.manifest, sealedTree(buildTools.path, 'governed build-tool bin')), /drifted/)
  } finally { rmSync(root, { recursive: true, force: true }) }
  assert.throws(() => assertStableSealedArtifact('resolved cargo tool', rustup.cargo.resolved, { ...rustup.cargo.resolved, sha256: '0'.repeat(64) }), /drifted/)
})

test('npm script shell honors Windows cmd recognition and POSIX shebang limits', () => {
  const windows = npmScriptShellSemantics('C:\\Windows\\System32\\cmd.exe', 'win32')
  assert.deepEqual(windows, { path: 'C:\\Windows\\System32\\cmd.exe', npmArgs: ['/d', '/s', '/c'] })
  assert.notDeepEqual(windows.npmArgs, ['-c'])
  assert.throws(() => npmScriptShellSemantics('C:\\Windows\\System32\\cmd.exe.cmd', 'win32'), /cmd\.exe/)
  assert.throws(() => npmScriptShellSemantics('C:\\Windows\\System32\\sh.exe', 'win32'), /cmd\.exe/)
  assert.throws(() => npmScriptShellSemantics('C:\\Windows\\System32\\shell.cmd', 'win32'), /cmd\.exe/)
  assert.equal(assertPosixShebangPath('/usr/bin/dash'), '/usr/bin/dash')
  assert.throws(() => assertPosixShebangPath('dash'), /representable/)
  assert.throws(() => assertPosixShebangPath('/tmp/shell path'), /representable/)
  assert.throws(() => assertPosixShebangPath('/tmp/shell\npath'), /representable/)
})

test('runtime identity and ambient tool-selection overrides fail closed', () => {
  assert.deepEqual(assertRuntimeAgent({ schema: 'shellx-cut/agent-docs/2', version: '0.6.109', runtime: { executable: '/candidate/cutd' } }, { cutd: '/candidate/cutd', version: '0.6.109' }), { executable: '/candidate/cutd', version: '0.6.109' })
  assert.throws(() => assertRuntimeAgent({ schema: 'shellx-cut/agent-docs/2', version: '0.6.109', runtime: { executable: '/other/cutd' } }, { cutd: '/candidate/cutd', version: '0.6.109' }), /exact candidate/)
  assert.equal(assertNoAmbientRuntimeOverrides({}), true)
  assert.throws(() => assertNoAmbientRuntimeOverrides({ FCV_INSTALLED_APP: '1' }), /forbidden/)
  assert.throws(() => assertNoAmbientRuntimeOverrides({ PLAYWRIGHT_BROWSERS_PATH: '/other' }), /forbidden/)
  assert.throws(() => assertNoAmbientRuntimeOverrides({ SHELLX_CUT_REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM: '1' }), /forbidden/)
  assert.throws(() => assertNoAmbientRuntimeOverrides({ SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_PATH: '/other/chromium' }), /forbidden/)
  assert.throws(() => assertNoAmbientRuntimeOverrides({ SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_SHA256: 'a'.repeat(64) }), /forbidden/)
  assert.throws(() => assertNoAmbientRuntimeOverrides({ RUSTUP_TOOLCHAIN: 'other' }), /forbidden/)
  assert.throws(() => assertNoAmbientRuntimeOverrides({ CC: '/other/cc' }), /forbidden/)
  assert.throws(() => assertNoAmbientRuntimeOverrides({ NPM_CONFIG_SCRIPT_SHELL: '/other/sh' }), /forbidden/)
  assert.throws(() => assertNoAmbientRuntimeOverrides({ npm_config_script_shell: '/other/sh' }), /forbidden/)
})

test('runner stays source-browser only and delegates behavior to maintained flows', () => {
  const runner = readFileSync(resolve('scripts/runtime-sealed-b1-b2-gate.mjs'), 'utf8')
  for (const [path, owner] of [
    ['verify-source-monitor.mjs', 'private-tests'],
    ['context-menu-surfaces-verify.mjs', 'public-tests'],
    ['full-coverage-verify.mjs', 'private-tests'],
  ]) {
    assert.match(runner, new RegExp(path.replace('.', '\\.')))
    const harness = readFileSync(resolve('ui', owner, path), 'utf8')
    assert.match(harness, /sealedPlaywrightChromiumLaunchOptions/)
  }
  for (const name of ['SHELLX_CUT_REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM', 'SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_PATH', 'SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_SHA256']) assert.match(runner, new RegExp(name))
  for (const marker of ['browser-temporary-root.json', 'browserTemporaryRoot', 'allocationRecord', 'allowOwnedLinks: true', 'progress.fixtures = fixtures']) assert.match(runner, new RegExp(marker))
  assert.match(runner, /--script-shell/)
  assert.match(runner, /npmShell/)
  for (const claim of ['installedApp: false', 'nativeHost: false', 'signed: false']) assert.match(runner, new RegExp(claim))
})
