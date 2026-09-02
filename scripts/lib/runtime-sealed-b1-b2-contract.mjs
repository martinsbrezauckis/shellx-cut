import { createHash } from 'node:crypto'
import { isAbsolute, relative, resolve } from 'node:path'

export const RUNTIME_SEALED_B1_B2_SCHEMA = 'shellx-cut/runtime-sealed-b1-b2-browser@1'
export const EXPECTED_ROWS = Object.freeze({
  sourceMonitor: 9,
  contextMenus: 34,
  volumeAutomation: 13,
})

export const B1_SOURCE_MONITOR_ASSERTIONS = Object.freeze([
  'unused timed asset exposes Source monitor',
  'monitor streams original source with transport',
  'mark In and Out preserve the selected source range',
  'range insert creates aligned linked video and audio',
  'Source Monitor target controls expose keyboard-operable Off state',
  'Source Monitor lands one linked V/A overwrite without shifting the downstream insert',
  'composed pixels inside the overwrite match the selected source frame',
  'Source monitor fits the supported minimum window',
  'Escape closes Source monitor',
])

export const B1_CONTEXT_ASSERTIONS = Object.freeze([
  'Preview monitor center routes its native context gesture past passive media',
  'Preview menu exposes exact-base source and marker routes', 'Preview source route preserves exact asset identity',
  'Assets menu targets the clicked asset', 'Assets menu dismisses with Escape',
  'Clip context exposes enabled registered Source file reveal', 'Clip context browser Source file reveal refuses without native invoke',
  'Clip context exposes enabled exact Project reveal', 'Clip context Project reveal selects the exact registered asset',
  'Reversed footage clip exposes Match Frame at its first timeline instant',
  'Reverse Match Frame opens the final included source millisecond',
  'Reverse Match Frame fixture restores the normal clip before later rows', 'Footage clip exposes exact Match Frame',
  'Clip Match Frame opens the exact source frame', 'All uses transport rejection clears loading and exposes recovery',
  'All uses can hide after a transport rejection', 'All uses switches and seeks the selected crossfade occurrence in laid time',
  'Track Match Frame refuses an ambiguous crossfade', 'Projects current card refuses reopen/delete',
  'Custom speed accepts engine-valid 0.25×', 'Custom speed accepts engine-valid 4×',
  'Custom speed accepts engine-valid 0.251×', 'Custom speed refuses invalid 0.249',
  'Custom speed refuses invalid 4.001', 'Custom speed refuses invalid empty value', 'fixture creates a lift gap',
  'Gap menu exposes only gap-valid routes', 'Gap fit refuses absent clipboard identity',
  'Empty timeline menu is non-clip operational context',
  'Track header keyboard menu owns Match Frame plus video track controls', 'Track Match Frame keyboard action opens Source Monitor',
  'fixture locks base video track', 'Locked track keeps read-only Match Frame but blocks edit mutations',
  'Locked menu unlock dispatches to the exact locked track',
])

export const B2_RESULTS = Object.freeze([
  ['catalog-guard::CATALOG-DRIFT:effects', 'support', 'catalog-guard', 'CATALOG-DRIFT:effects', 'pass', 'na', 'na', 'pass'],
  ['audio-clip::GATE:audio-eq-shown', 'support', 'audio-clip', 'GATE:audio-eq-shown', 'pass', 'na', 'na', 'pass'],
  ['audio-clip::GATE:transform-hidden', 'support', 'audio-clip', 'GATE:transform-hidden', 'pass', 'na', 'na', 'pass'],
  ['audio-clip::GATE:volume-automation-shown', 'support', 'audio-clip', 'GATE:volume-automation-shown', 'pass', 'na', 'na', 'pass'],
  ['volume-automation-time', 'ui_action', 'audio-clip', 'volume-automation-time', 'pass', 'pass', 'pass', 'pass'],
  ['volume-automation-level', 'ui_action', 'audio-clip', 'volume-automation-level', 'pass', 'pass', 'pass', 'pass'],
  ['volume-automation-add', 'ui_action', 'audio-clip', 'volume-automation-add-first', 'pass', 'pass', 'pass', 'pass'],
  ['volume-automation-add', 'ui_action', 'audio-clip', 'volume-automation-add-second', 'pass', 'pass', 'pass', 'pass'],
  ['volume-automation-point', 'ui_action', 'audio-clip', 'volume-automation-point', 'pass', 'pass', 'pass', 'pass'],
  ['volume-automation-interpolation', 'ui_action', 'audio-clip', 'volume-automation-interpolation', 'pass', 'pass', 'pass', 'pass'],
  ['volume-automation-remove', 'ui_action', 'audio-clip', 'volume-automation-remove', 'pass', 'pass', 'pass', 'pass'],
  ['volume-automation-clear', 'ui_action', 'audio-clip', 'volume-automation-clear', 'pass', 'pass', 'pass', 'pass'],
  ['global::console-clean', 'support', 'global', 'console-clean', 'na', 'na', 'na', 'pass'],
])

export function parseRuntimeSealedArgs(argv) {
  const args = {
    build: false,
    cutd: '',
    uiDist: '',
    baseClip: '',
    insertClip: '',
    contextFixture: '',
    volumeFixture: '',
    out: '',
    help: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--build') args.build = true
    else if (arg === '--cutd') args.cutd = argv[++index] || ''
    else if (arg === '--ui-dist') args.uiDist = argv[++index] || ''
    else if (arg === '--base-clip') args.baseClip = argv[++index] || ''
    else if (arg === '--insert-clip') args.insertClip = argv[++index] || ''
    else if (arg === '--context-fixture') args.contextFixture = argv[++index] || ''
    else if (arg === '--volume-fixture') args.volumeFixture = argv[++index] || ''
    else if (arg === '--out') args.out = argv[++index] || ''
    else if (arg === '--help' || arg === '-h') args.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return args
}

export function validateRuntimeSealedArgs(args) {
  const consuming = Boolean(args.cutd || args.uiDist)
  if (args.build && consuming) throw new Error('--build cannot be combined with --cutd or --ui-dist')
  if (consuming && (!args.cutd || !args.uiDist)) throw new Error('consume mode requires both --cutd and --ui-dist')
  for (const [flag, value] of Object.entries({
    '--base-clip': args.baseClip,
    '--insert-clip': args.insertClip,
    '--context-fixture': args.contextFixture,
    '--volume-fixture': args.volumeFixture,
  })) {
    if (!value) throw new Error(`${flag} is required`)
  }
  return { mode: consuming ? 'consume' : 'build' }
}

export function assertRuntimeSealedOutputPath(repoRoot, path, scratchRoot) {
  const resolved = resolve(repoRoot, path)
  const expectedRoot = resolve(repoRoot, scratchRoot)
  const inside = relative(expectedRoot, resolved)
  if (!inside || inside === '..' || inside.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`) || isAbsolute(inside)) {
    throw new Error(`--out must be a new directory below ${scratchRoot}`)
  }
  return resolved
}

export function assertCleanSourceIdentity(before, after = before) {
  for (const source of [before, after]) {
    if (!source?.gitCommit || !source?.gitTree || source.gitDirty !== false) {
      throw new Error('runtime-sealed gate requires a clean committed source tree')
    }
    if (!source.contentManifest?.sha256 || !source.actionManifest?.sha256) {
      throw new Error('runtime-sealed gate requires content and action manifest identities')
    }
  }
  for (const key of ['gitCommit', 'gitTree']) {
    if (before[key] !== after[key]) throw new Error(`source drift: ${key} changed during qualification`)
  }
  for (const key of ['sha256', 'files', 'bytes']) {
    if (before.contentManifest[key] !== after.contentManifest[key]) throw new Error(`source drift: content manifest ${key} changed during qualification`)
  }
  for (const key of ['sha256', 'bytes']) {
    if (before.actionManifest[key] !== after.actionManifest[key]) throw new Error(`source drift: action manifest ${key} changed during qualification`)
  }
  return true
}

function assertionNames(log, prefix, terminal = '') {
  return String(log).split(/\r?\n/).flatMap((line) => {
    if (!line.startsWith(prefix) || line === terminal) return []
    const body = line.slice(prefix.length)
    const separator = body.search(/\s{2}|\s—\s/)
    return [separator < 0 ? body : body.slice(0, separator)]
  })
}

function assertExact(label, actual, expected) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${label} assertion inventory drifted: expected ${expected.length} exact named rows, got ${actual.length}`)
  }
}

export function assertSourceMonitorLog(log) {
  if (/^FAIL\s/m.test(log)) throw new Error('B1 Source Monitor log contains a failure')
  const names = assertionNames(log, 'PASS  ')
  assertExact('B1 Source Monitor', names, B1_SOURCE_MONITOR_ASSERTIONS)
  return { expectedRows: EXPECTED_ROWS.sourceMonitor, observedRows: names.length, assertions: names, failures: 0 }
}

export function assertContextMenuLog(log) {
  const summary = new RegExp(`^PASS context-menu surfaces — ${EXPECTED_ROWS.contextMenus} pass / 0 fail$`, 'm')
  if (/^FAIL\s/m.test(log) || !summary.test(log)) throw new Error('B1 context log contains a failure or lacks its terminal summary')
  const names = assertionNames(log, 'PASS ', `PASS context-menu surfaces — ${EXPECTED_ROWS.contextMenus} pass / 0 fail`)
  assertExact('B1 context', names, B1_CONTEXT_ASSERTIONS)
  return { expectedRows: EXPECTED_ROWS.contextMenus, observedRows: names.length, assertions: names, failures: 0 }
}

export function assertVolumeAutomationReceipt(receipt, expected) {
  const controls = receipt?.summary?.controls
  const candidate = receipt?.candidate
  const runtime = receipt?.runtime
  const dimensions = receipt?.summary?.dimensions
  if (receipt?.schema !== 'shellx-cut/full-coverage-results@1' || receipt.ok !== true
    || receipt.full !== false || receipt.strictAllActions !== false) {
    throw new Error('B2 FCV receipt is not the expected passing source-browser receipt')
  }
  const exactControls = { total: 13, uiActions: 8, supportRows: 5, fullyVerified: 13, delegated: 0, dependencySkips: 0, optionalAgentSkips: 0, guards: 0, couldNotVerify: 0, strictUnverified: 0, focusedUnverified: 0, failures: 0 }
  const exactDimensions = { present: { pass: 12, fail: 0, na: 1 }, render: { pass: 8, fail: 0, na: 5 }, click: { pass: 8, fail: 0, na: 5 }, result: { pass: 13, fail: 0, na: 0 } }
  if (!controls || JSON.stringify(controls) !== JSON.stringify(exactControls) || JSON.stringify(dimensions) !== JSON.stringify(exactDimensions)) {
    throw new Error('B2 FCV receipt is partial, unverified, failed, or has the wrong row count')
  }
  const rows = receipt?.results
  const normalized = Array.isArray(rows) ? rows.map((row) => [row.actionId, row.rowKind, row.surface, row.name, row.present, row.render, row.click, row.result]) : null
  if (!normalized || JSON.stringify(normalized) !== JSON.stringify(B2_RESULTS) || rows.some((row) => row.ok !== true || row.classification !== 'fully_verified')) {
    throw new Error('B2 FCV result inventory has missing, extra, or semantically different rows')
  }
  const action = receipt?.actionManifest
  const actionIds = ['volume-automation-add', 'volume-automation-clear', 'volume-automation-interpolation', 'volume-automation-level', 'volume-automation-point', 'volume-automation-remove', 'volume-automation-time']
  const expectedActionHash = createHash('sha256').update(JSON.stringify(actionIds)).digest('hex')
  if (!action || action.algorithm !== 'sha256' || action.sha256 !== expectedActionHash || action.total !== 7 || action.occurrences !== 8
    || JSON.stringify(action.observed) !== JSON.stringify(actionIds) || JSON.stringify(action.repeated) !== JSON.stringify([{ id: 'volume-automation-add', count: 2 }])) {
    throw new Error('B2 FCV action inventory or repeated add semantics drifted')
  }
  if (runtime?.installedApp !== false || runtime?.nativeAttached !== false || runtime?.driver !== 'playwright-chromium') {
    throw new Error('B2 FCV receipt is not browser-source evidence')
  }
  const requiredCandidate = ['id', 'sourceCommit', 'sourceTree', 'worktree', 'contentManifestSha256', 'testControlManifestSha256', 'fixtureId', 'fixtureSeedSha256', 'runnerId']
  if (!candidate || requiredCandidate.some((key) => candidate[key] !== expected.candidate[key])) {
    throw new Error('B2 FCV candidate identity does not match the sealed source/runtime inputs')
  }
  if (runtime.sourceGitCommit !== expected.source.gitCommit
    || runtime.sourceContentManifestSha256 !== expected.source.contentManifest.sha256) {
    throw new Error('B2 FCV runtime source identity is not sealed to the candidate')
  }
  return { expectedRows: EXPECTED_ROWS.volumeAutomation, observedRows: controls.total, failures: 0, unverified: 0 }
}

export function assertRuntimeAgent(agent, expected) {
  const executable = agent?.runtime?.executable
  if (agent?.schema !== 'shellx-cut/agent-docs/2' || agent?.version !== expected.version
    || !executable || resolve(executable) !== resolve(expected.cutd)) {
    throw new Error('running cutd does not report the exact candidate executable/version')
  }
  return { executable: resolve(executable), version: agent.version }
}

export function assertNoAmbientRuntimeOverrides(env) {
  const forbidden = [
    'SWEEP_CUTD', 'SWEEP_APP', 'FCV_SECTION', 'FCV_ONLY', 'FCV_NO_AGENT',
    'FCV_REQUIRE_FULL', 'FCV_FINAL_ALL_ACTIONS', 'FCV_INSTALLED_APP', 'FCV_UI_DRIVER',
    'NODE_OPTIONS', 'NODE_PATH', 'PLAYWRIGHT_BROWSERS_PATH',
    'SHELLX_CUT_REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM', 'SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_PATH',
    'SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_SHA256', 'FFMPEG', 'FFPROBE',
    'FFMPEG_PATH', 'FFPROBE_PATH', 'SHELLX_CUT_FFMPEG', 'SHELLX_CUT_FFPROBE',
    'RUSTC', 'CARGO_TARGET_DIR', 'CARGO_HOME', 'RUSTUP_HOME', 'RUSTUP_TOOLCHAIN', 'NPM_CONFIG_PREFIX',
    'NPM_CONFIG_SCRIPT_SHELL', 'npm_config_script_shell',
    'CC', 'CXX', 'AR', 'LD', 'PKG_CONFIG', 'PKG_CONFIG_PATH', 'PKG_CONFIG_LIBDIR', 'CFLAGS', 'CXXFLAGS', 'LDFLAGS',
  ].filter((name) => env[name] !== undefined && env[name] !== '')
  if (forbidden.length) throw new Error(`ambient qualification override(s) are forbidden: ${forbidden.join(', ')}`)
  return true
}
