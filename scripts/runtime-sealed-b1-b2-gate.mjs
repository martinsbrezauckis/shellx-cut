#!/usr/bin/env node

import { spawn, spawnSync } from 'node:child_process'
import { appendFileSync, closeSync, mkdirSync, openSync, readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { sourceContentManifest } from './lib/source-content-manifest.mjs'
import {
  RUNTIME_SEALED_B1_B2_SCHEMA, assertCleanSourceIdentity, assertContextMenuLog,
  assertNoAmbientRuntimeOverrides, assertRuntimeAgent, assertSourceMonitorLog,
  assertVolumeAutomationReceipt, parseRuntimeSealedArgs, validateRuntimeSealedArgs,
} from './lib/runtime-sealed-b1-b2-contract.mjs'
import {
  assertStableSealedArtifact, createOwnedOutputRoot, removeOwnedTree,
  materializeBuildToolBin, resolveExecutable, resolveNativeBuildTools,
  assertStableNpmScriptShell, recheckNpmScriptShell, resolveNpmScriptShell,
  resolveRustupToolchain, sealedRegularFile, sealedTree,
} from './lib/runtime-sealed-b1-b2-files.mjs'
import { createOwnedSystemTemporaryRoot } from './lib/runtime-sealed-b1-b2-owned-temp.mjs'
import { startLoopbackCutd } from './lib/runtime-sealed-b1-b2-process.mjs'

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const SCRATCH = '.scratch/runtime-sealed-b1-b2'
const uiRequire = createRequire(join(REPO, 'ui/package.json'))
const USAGE = [
  'usage: node scripts/runtime-sealed-b1-b2-gate.mjs [--build | --cutd <path> --ui-dist <path>]',
  '  --base-clip <regular-media-file> --insert-clip <regular-media-file> --context-fixture <regular-media-file>',
  '  --volume-fixture <regular-media-file> [--out .scratch/runtime-sealed-b1-b2/<new-run>]',
].join('\n')

function toolText(tool, args, path) {
  const entry = tool.invocation || tool
  const result = spawnSync(entry.path, args, { cwd: REPO, encoding: 'utf8', timeout: 15_000, env: path })
  if (result.status !== 0) throw new Error(`${entry.path} ${args.join(' ')} failed: ${String(result.stderr || result.error || '').trim()}`)
  return String(result.stdout || result.stderr).split(/\r?\n/).find(Boolean) || entry.path
}

function nodeNpmCli(node) {
  const bin = dirname(node.path)
  const candidates = process.platform === 'win32'
    ? [join(bin, 'node_modules/npm/bin/npm-cli.js')]
    : [join(bin, '../lib/node_modules/npm/bin/npm-cli.js')]
  for (const path of candidates) {
    try { return sealedRegularFile(path, 'npm CLI script') } catch { /* try the next Node-owned location */ }
  }
  throw new Error(`sealed Node installation has no npm CLI script: ${node.path}`)
}

export function collectTools(build, playwrightChromiumPath = uiRequire('playwright').chromium.executablePath()) {
  const tools = {
    node: sealedRegularFile(process.execPath, 'Node executable', { executable: true }),
    git: resolveExecutable('git', process.env, 'git executable'),
    ffmpeg: resolveExecutable('ffmpeg', process.env, 'ffmpeg executable'),
    ffprobe: resolveExecutable('ffprobe', process.env, 'ffprobe executable'),
    playwrightChromium: sealedRegularFile(playwrightChromiumPath, 'Playwright Chromium executable', { executable: true }),
    harnesses: Object.fromEntries([
      ['verify-source-monitor.mjs', 'ui/private-tests/verify-source-monitor.mjs'],
      ['context-menu-surfaces-verify.mjs', 'ui/public-tests/context-menu-surfaces-verify.mjs'],
      ['full-coverage-verify.mjs', 'ui/private-tests/full-coverage-verify.mjs'],
    ].map(([name, path]) => [name, sealedRegularFile(join(REPO, path), `maintained harness ${name}`)])),
  }
  if (build) {
    tools.npmCli = nodeNpmCli(tools.node)
    tools.rustup = resolveRustupToolchain(process.env)
    tools.cargo = tools.rustup.cargo
    tools.rustc = tools.rustup.rustc
    tools.nativeBuild = resolveNativeBuildTools(REPO, process.env)
    tools.npmShell = resolveNpmScriptShell(process.env)
  }
  const env = sealedEnv({ artifacts: tools }, join(REPO, '.scratch', 'runtime-sealed-tool-home'))
  return {
    artifacts: tools,
    versions: {
      node: toolText(tools.node, ['--version'], env), git: toolText(tools.git, ['--version'], env),
      ffmpeg: toolText(tools.ffmpeg, ['-version'], env), ffprobe: toolText(tools.ffprobe, ['-version'], env),
      playwrightChromium: toolText(tools.playwrightChromium, ['--version'], env),
      ...(build ? { npm: toolText(tools.node, [tools.npmCli.path, '--version'], env), cargo: toolText(tools.cargo, ['--version'], env), rustc: toolText(tools.rustc, ['--version'], env) } : {}),
    },
  }
}

export function sealedEnv(tools, home, buildToolBin = null) {
  const names = Object.entries(tools.artifacts).flatMap(([id, tool]) => id === 'npmShell' ? [] : (tool?.invocation || tool)?.path ? [dirname((tool.invocation || tool).path)] : [])
  const path = [...new Set(buildToolBin ? [buildToolBin.path] : names)].join(process.platform === 'win32' ? ';' : ':')
  const rustup = tools.artifacts.rustup
  const native = buildToolBin?.tools
  return { PATH: path, HOME: home, SHELLX_CUT_HOME: home, SHELLX_CUT_WORKTREE: REPO, ...(rustup ? { CARGO_HOME: rustup.cargoHome, RUSTUP_HOME: rustup.rustupHome, RUSTUP_TOOLCHAIN: rustup.toolchain.name } : {}), ...(native ? { CC: native.cc.path, AR: native.ar.path, LD: native.ld.path, ...(native.pkgConfig ? { PKG_CONFIG: native.pkgConfig.path } : {}) } : {}), ...(process.platform === 'win32' ? { SYSTEMROOT: process.env.SYSTEMROOT || '', COMSPEC: process.env.COMSPEC || '' } : {}) }
}

function git(tools, args) {
  const result = spawnSync(tools.artifacts.git.path, args, { cwd: REPO, encoding: 'utf8', env: sealedEnv(tools, join(REPO, '.scratch', 'runtime-sealed-git-home')) })
  if (result.status !== 0) throw new Error(`git ${args.join(' ')} failed: ${String(result.stderr).trim()}`)
  return result.stdout.trim()
}

function sourceIdentity(tools, ownedOutput = '') {
  const tauri = JSON.parse(readFileSync(join(REPO, 'app/desktop/src-tauri/tauri.conf.json'), 'utf8'))
  const manifest = sourceContentManifest(REPO)
  const action = sealedRegularFile(join(REPO, 'ui/private-tests/full-ui-action-manifest.json'), 'UI action manifest')
  const statusArgs = ['status', '--porcelain', '--untracked-files=all', '--', '.']
  if (ownedOutput) statusArgs.push(`:(exclude)${relative(REPO, ownedOutput).replaceAll('\\', '/')}`)
  return {
    gitCommit: git(tools, ['rev-parse', 'HEAD']), gitTree: git(tools, ['rev-parse', 'HEAD^{tree}']),
    gitDirty: git(tools, statusArgs).length > 0, version: tauri.version,
    contentManifest: { sha256: manifest.sha256, files: manifest.files, bytes: manifest.bytes },
    actionManifest: { path: 'ui/private-tests/full-ui-action-manifest.json', sha256: action.sha256, bytes: action.bytes },
  }
}

function commandLog(path, command, args, env, cwd = REPO) {
  return new Promise((resolveCommand, reject) => {
    const log = openSync(path, 'a')
    let settled = false
    const settle = (error) => { if (!settled) { settled = true; closeSync(log); error ? reject(error) : resolveCommand() } }
    appendFileSync(path, `$ ${command} ${args.join(' ')}\n`)
    let child
    try { child = spawn(command, args, { cwd, env, stdio: ['ignore', log, log] }) } catch (error) { settle(error); return }
    child.once('error', (error) => settle(error))
    child.once('exit', (code, signal) => settle(code === 0 ? null : new Error(`${command} exited ${code ?? 'null'}${signal ? ` (${signal})` : ''}; see ${path}`)))
  })
}

function candidateFor(source, fixture) {
  return {
    id: `runtime-sealed-b1-b2-${source.gitCommit.slice(0, 8)}`, sourceCommit: source.gitCommit, sourceTree: source.gitTree,
    worktree: REPO, contentManifestSha256: source.contentManifest.sha256, testControlManifestSha256: source.actionManifest.sha256,
    fixtureId: 'runtime-sealed-b2-volume-fixture', fixtureSeedSha256: fixture.sha256, runnerId: 'runtime-sealed-b1-b2-browser',
  }
}

async function cleanupFlow(runtime, roots) {
  const failures = []
  let daemon = { status: 'not-started' }
  if (runtime) {
    try { daemon = await runtime.stop() } catch (error) { daemon = { status: 'cleanup-failed', error: String(error.message || error) } }
    if (!['graceful', 'forced'].includes(daemon.status)) failures.push(`cutd=${daemon.status}`)
  }
  const owned = []
  for (const root of roots) {
    const binding = { label: root.label, parent: root.parent, path: root.path, allocation: root.allocation || null }
    try { owned.push({ ...binding, cleanup: removeOwnedTree(root.parent, root.path, root.label, { allowOwnedLinks: root.allowOwnedLinks === true }) }) } catch (error) { owned.push({ ...binding, status: 'cleanup-failed', error: String(error.message || error) }); failures.push(`state=${error.message || error}`) }
  }
  return { status: failures.length ? 'cleanup-failed' : 'pass', daemon, ownedState: { status: failures.length ? 'cleanup-failed' : 'removed', roots: owned }, failures }
}

function recordBrowserTemporaryRoot(flowRoot, temporary) {
  const path = join(flowRoot, 'browser-temporary-root.json')
  writeFileSync(path, `${JSON.stringify({
    schema: 'shellx-cut/runtime-sealed-browser-temporary-root@1',
    parent: temporary.parent,
    path: temporary.path,
    allocation: temporary.allocation,
    initialTree: temporary.initialTree,
  }, null, 2)}\n`)
  return sealedRegularFile(path, 'browser temporary allocation record')
}

async function executeFlow({ id, cutd, uiDist, root, source, tools, run }) {
  const flowRoot = join(root, id)
  mkdirSync(flowRoot, { mode: 0o700 })
  const runtimeHome = join(flowRoot, 'home')
  const projects = join(flowRoot, 'projects')
  let runtime
  let allocationRecord
  const cleanupRoots = []
  let flow = { id, status: 'fail', browserTemporaryRoot: null, cleanup: { status: 'not-run' } }
  try {
    const temporary = createOwnedSystemTemporaryRoot()
    cleanupRoots.push({ parent: temporary.parent, path: temporary.path, label: 'flow-owned browser temporary state', allocation: temporary.allocation, allowOwnedLinks: true })
    allocationRecord = recordBrowserTemporaryRoot(flowRoot, temporary)
    flow.browserTemporaryRoot = {
      parent: temporary.parent,
      path: temporary.path,
      allocation: temporary.allocation,
      initialTree: temporary.initialTree,
      allocationRecord: { before: allocationRecord, after: null },
      cleanup: null,
    }
    mkdirSync(runtimeHome, { mode: 0o700 })
    cleanupRoots.unshift({ parent: flowRoot, path: runtimeHome, label: 'flow-owned home state', allowOwnedLinks: true })
    mkdirSync(projects, { mode: 0o700 })
    cleanupRoots.splice(1, 0, { parent: flowRoot, path: projects, label: 'flow-owned project state' })
    const env = {
      ...sealedEnv(tools, runtimeHome),
      SHELLX_CUT_PROJECTS_DIR: projects,
      TMPDIR: temporary.path,
      TEMP: temporary.path,
      TMP: temporary.path,
      SHELLX_CUT_REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM: '1',
      SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_PATH: tools.artifacts.playwrightChromium.path,
      SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_SHA256: tools.artifacts.playwrightChromium.sha256,
    }
    runtime = await startLoopbackCutd({ cutd: cutd.path, uiDist: uiDist.path, env, logPath: join(flowRoot, 'cutd.log') })
    const runtimeIdentity = assertRuntimeAgent(runtime.agent, { cutd: cutd.path, version: source.version })
    if (!readFileSync(join(flowRoot, 'cutd.log'), 'utf8').includes(`serving UI from ${uiDist.path}`)) throw new Error(`cutd did not report the sealed UI tree: ${uiDist.path}`)
    writeFileSync(join(flowRoot, 'api-agent.json'), `${JSON.stringify(runtime.agent, null, 2)}\n`)
    const result = await run({ ...env, SWEEP_CUTD: runtime.url, SWEEP_APP: runtime.url }, flowRoot)
    const cleanup = await cleanupFlow(runtime, cleanupRoots)
    const allocationRecordAfter = sealedRegularFile(allocationRecord.path, 'browser temporary allocation record')
    assertStableSealedArtifact('browser temporary allocation record', allocationRecord, allocationRecordAfter)
    const browserCleanup = cleanup.ownedState.roots.find((root) => root.label === 'flow-owned browser temporary state') || null
    flow = {
      id,
      status: cleanup.status === 'pass' ? 'pass' : 'fail',
      runtime: runtimeIdentity,
      browserTemporaryRoot: { ...flow.browserTemporaryRoot, allocationRecord: { before: allocationRecord, after: allocationRecordAfter }, cleanup: browserCleanup },
      cleanup,
      ...result,
    }
    if (flow.status !== 'pass') throw new Error(`${id} cleanup failed`)
    writeFileSync(join(flowRoot, 'result.json'), `${JSON.stringify(flow, null, 2)}\n`)
    return flow
  } catch (error) {
    if (flow.cleanup.status === 'not-run') flow.cleanup = await cleanupFlow(runtime, cleanupRoots)
    if (flow.browserTemporaryRoot) {
      let allocationRecordAfter = null
      if (allocationRecord) {
        try {
          allocationRecordAfter = sealedRegularFile(allocationRecord.path, 'browser temporary allocation record')
          assertStableSealedArtifact('browser temporary allocation record', allocationRecord, allocationRecordAfter)
        } catch (recordError) { flow.browserTemporaryRoot.allocationRecordError = String(recordError.message || recordError) }
      }
      flow.browserTemporaryRoot = {
        ...flow.browserTemporaryRoot,
        allocationRecord: allocationRecord ? { before: allocationRecord, after: allocationRecordAfter } : null,
        cleanup: flow.cleanup.ownedState?.roots?.find((root) => root.label === 'flow-owned browser temporary state') || null,
      }
    }
    flow.error = String(error.message || error)
    try { writeFileSync(join(flowRoot, 'result.json'), `${JSON.stringify(flow, null, 2)}\n`) } catch (writeError) { flow.receiptError = String(writeError.message || writeError) }
    error.flow = flow
    throw error
  }
}

function writeReceipt(path, value) { writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`) }

function evidence(root) {
  const tree = sealedTree(root, 'qualification evidence')
  return { sha256: tree.sha256, bytes: tree.bytes, files: tree.entries.filter((file) => file.path !== 'runtime-sealed-b1-b2-receipt.json') }
}

function failureEvidence(root) {
  try { return evidence(root) } catch (error) { return { status: 'unavailable', error: String(error.message || error) } }
}

async function qualify(args) {
  let root = ''
  let mode
  const progress = { flows: [], cleanup: { status: 'not-started' } }
  try {
    mode = validateRuntimeSealedArgs(args)
    assertNoAmbientRuntimeOverrides(process.env)
    root = createOwnedOutputRoot(REPO, SCRATCH, args.out || '')
    const receiptPath = join(root, 'runtime-sealed-b1-b2-receipt.json')
    const tools = collectTools(mode.mode === 'build')
    progress.tools = tools
    const before = sourceIdentity(tools, root)
    assertCleanSourceIdentity(before)
    const fixtures = Object.fromEntries(Object.entries({ sourceMonitorBase: args.baseClip, sourceMonitorInsert: args.insertClip, contextMenus: args.contextFixture, volumeAutomation: args.volumeFixture })
      .map(([id, path]) => [id, sealedRegularFile(resolve(REPO, path), `fixture ${id}`)]))
    progress.fixtures = fixtures
    let uiDist
    let cutd
    let buildToolBin
    if (mode.mode === 'build') {
      const build = join(root, 'build'); mkdirSync(build, { mode: 0o700 })
      const npmShell = recheckNpmScriptShell(tools.artifacts.npmShell, process.env)
      buildToolBin = materializeBuildToolBin(tools.artifacts.nativeBuild, tools.artifacts.node, join(build, 'native-tools'), process.platform === 'win32' ? null : npmShell)
      progress.buildToolBin = { before: buildToolBin }
      const buildEnv = sealedEnv(tools, join(build, 'home'), buildToolBin); mkdirSync(buildEnv.HOME, { mode: 0o700 })
      await commandLog(join(build, 'ui-build.log'), tools.artifacts.node.path, [tools.artifacts.npmCli.path, '--prefix', 'ui', 'run', 'build', '--script-shell', npmShell.resolved.path], buildEnv)
      await commandLog(join(build, 'cargo-build.log'), tools.artifacts.cargo.invocation.path, ['build', '--manifest-path', 'app/Cargo.toml', '-p', 'server', '--bin', 'cutd', '--release', '--target-dir', join(build, 'cargo-target')], buildEnv)
      uiDist = sealedTree(join(REPO, 'ui/dist'), 'built ui/dist')
      cutd = sealedRegularFile(join(build, 'cargo-target/release', process.platform === 'win32' ? 'cutd.exe' : 'cutd'), 'built cutd', { executable: true })
    } else { uiDist = sealedTree(resolve(REPO, args.uiDist), '--ui-dist'); cutd = sealedRegularFile(resolve(REPO, args.cutd), '--cutd', { executable: true }) }
    const candidate = candidateFor(before, fixtures.volumeAutomation)
    const flows = [
      ['b1-source-monitor', async (env, flowRoot) => {
        const screenDir = join(flowRoot, 'screens'); mkdirSync(screenDir, { mode: 0o700 }); const screenshot = join(screenDir, 'source-monitor.png'); const log = join(flowRoot, 'runner.log')
        await commandLog(log, tools.artifacts.node.path, ['ui/private-tests/verify-source-monitor.mjs'], { ...env, CUT_SOURCE_MONITOR_BASE_CLIP: fixtures.sourceMonitorBase.path, CUT_SOURCE_MONITOR_SOURCE_CLIP: fixtures.sourceMonitorInsert.path, CUT_SOURCE_MONITOR_SCREENSHOT: screenshot })
        return { ...assertSourceMonitorLog(readFileSync(log, 'utf8')), screenshots: [sealedRegularFile(screenshot, 'B1 Source Monitor screenshot')] }
      }],
      ['b1-context-surfaces', async (env, flowRoot) => {
        const log = join(flowRoot, 'runner.log')
        await commandLog(log, tools.artifacts.node.path, ['ui/public-tests/context-menu-surfaces-verify.mjs'], { ...env, CONTEXT_MENU_MUXED_CLIP: fixtures.contextMenus.path, CONTEXT_MENU_PROJECT_ROOT: join(flowRoot, 'projects') })
        return assertContextMenuLog(readFileSync(log, 'utf8'))
      }],
      ['b2-volume-automation', async (env, flowRoot) => {
        const log = join(flowRoot, 'runner.log'); const resultPath = join(flowRoot, 'full-coverage-result.json')
        await commandLog(log, tools.artifacts.node.path, ['ui/private-tests/full-coverage-verify.mjs'], { ...env,
          FCV_CANDIDATE_ID: candidate.id, FCV_SOURCE_GIT_COMMIT: candidate.sourceCommit, FCV_SOURCE_GIT_TREE: candidate.sourceTree, FCV_SOURCE_WORKTREE: candidate.worktree, FCV_SOURCE_CONTENT_MANIFEST_SHA256: candidate.contentManifestSha256, FCV_TEST_CONTROL_MANIFEST_SHA256: candidate.testControlManifestSha256, FCV_FIXTURE_ID: candidate.fixtureId, FCV_FIXTURE_SEED_SHA256: candidate.fixtureSeedSha256, FCV_RUNNER_ID: candidate.runnerId, FCV_ACTION_MANIFEST: join(REPO, before.actionManifest.path), FCV_SECTION: 'audio', FCV_ONLY: 'volume-automation', FCV_REQUIRE_FULL: '0', FCV_FINAL_ALL_ACTIONS: '0', FCV_INSTALLED_APP: '0', FCV_NO_AGENT: '1', FCV_UI_DRIVER: 'playwright-chromium', FCV_RESULT_RECEIPT: resultPath, FCV_SCREENS: join(flowRoot, 'screens'), FCV_MEDIA_TIER: 'runtime-sealed-fixture', RELEASE_CLIP: fixtures.volumeAutomation.path, RELEASE_CLIP_SPEECH: fixtures.volumeAutomation.path, RELEASE_CLIP_FACE: fixtures.volumeAutomation.path, RELEASE_CLIP_SPEAKERS: fixtures.volumeAutomation.path, RELEASE_CLIP2: fixtures.volumeAutomation.path,
        })
        return assertVolumeAutomationReceipt(JSON.parse(readFileSync(resultPath, 'utf8')), { source: before, candidate })
      }],
    ]
    for (const [id, run] of flows) {
      try { progress.flows.push(await executeFlow({ id, cutd, uiDist, root, source: before, tools, run })) } catch (error) { progress.flows.push(error.flow || { id, status: 'fail', error: String(error.message || error) }); throw error }
    }
    const runtimeAfter = { uiDist: sealedTree(uiDist.path, 'UI distribution tree'), cutd: sealedRegularFile(cutd.path, 'cutd binary', { executable: true }) }
    assertStableSealedArtifact('UI distribution tree', uiDist, runtimeAfter.uiDist); assertStableSealedArtifact('cutd binary', cutd, runtimeAfter.cutd)
    const fixturesAfter = Object.fromEntries(Object.entries(fixtures).map(([id, value]) => [id, sealedRegularFile(value.path, `fixture ${id}`)]))
    for (const id of Object.keys(fixtures)) assertStableSealedArtifact(`fixture ${id}`, fixtures[id], fixturesAfter[id])
    const toolsAfter = collectTools(mode.mode === 'build'); assertStableSealedArtifact('sealed tool bundle', { kind: 'tools', sha256: JSON.stringify(tools.artifacts) }, { kind: 'tools', sha256: JSON.stringify(toolsAfter.artifacts) })
    if (mode.mode === 'build') assertStableNpmScriptShell(tools.artifacts.npmShell, toolsAfter.artifacts.npmShell)
    if (buildToolBin) {
      const afterBuildToolBin = sealedTree(buildToolBin.path, 'governed build-tool bin')
      assertStableSealedArtifact('governed build-tool bin', buildToolBin.manifest, afterBuildToolBin)
      progress.buildToolBin.after = afterBuildToolBin
    }
    const after = sourceIdentity(toolsAfter, root); assertCleanSourceIdentity(before, after)
    progress.cleanup = { status: 'pass', daemons: progress.flows.map((flow) => ({ id: flow.id, ...flow.cleanup })) }
    const receipt = { schema: RUNTIME_SEALED_B1_B2_SCHEMA, status: 'pass', generatedAt: new Date().toISOString(), scope: 'source-browser-only', claims: { installedApp: false, nativeHost: false, signed: false, releaseFinal: false }, source: { before, after }, runtime: { mode: mode.mode, artifacts: { uiDist: { before: uiDist, after: runtimeAfter.uiDist }, cutd: { before: cutd, after: runtimeAfter.cutd } }, buildToolBin: progress.buildToolBin || null }, fixtures: { before: fixtures, after: fixturesAfter }, tools: { before: tools, after: toolsAfter }, flows: progress.flows, cleanup: progress.cleanup, evidence: evidence(root) }
    writeReceipt(receiptPath, receipt)
    return { receiptPath }
  } catch (error) {
    if (!root) { try { root = createOwnedOutputRoot(REPO, SCRATCH, '') } catch { /* a hostile scratch root cannot safely receive a receipt */ } }
    if (root) {
      try { writeReceipt(join(root, 'runtime-sealed-b1-b2-receipt.json'), { schema: RUNTIME_SEALED_B1_B2_SCHEMA, status: 'fail', generatedAt: new Date().toISOString(), scope: 'source-browser-only', claims: { installedApp: false, nativeHost: false, signed: false, releaseFinal: false }, error: String(error.message || error), progress, evidence: failureEvidence(root) }) } catch (receiptError) { error.message = `${error.message}; failed to write final receipt: ${receiptError.message || receiptError}` }
    }
    throw error
  }
}

async function main(argv) {
  let args
  try { args = parseRuntimeSealedArgs(argv) } catch (error) {
    let root = ''
    try { root = createOwnedOutputRoot(REPO, SCRATCH, '') } catch { /* no safe writable receipt location remains */ }
    if (root) writeReceipt(join(root, 'runtime-sealed-b1-b2-receipt.json'), { schema: RUNTIME_SEALED_B1_B2_SCHEMA, status: 'fail', generatedAt: new Date().toISOString(), scope: 'source-browser-only', claims: { installedApp: false, nativeHost: false, signed: false, releaseFinal: false }, error: String(error.message || error), progress: { flows: [], cleanup: { status: 'not-started' } } })
    throw error
  }
  if (args.help) { console.log(USAGE); return }
  const { receiptPath } = await qualify(args)
  console.log(`PASS runtime-sealed B1/B2 browser qualification: ${receiptPath}`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch((error) => { console.error(`FAIL runtime-sealed B1/B2 browser qualification: ${error.message}`); process.exitCode = 2 })
}
