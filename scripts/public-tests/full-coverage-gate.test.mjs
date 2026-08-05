import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

import {
  assessCompleteEnv,
  assessExternalFixtureContract,
  assessFinalAllActionsConfig,
  prependEnvPath,
  surfaceLabel,
} from '../release/full-coverage-gate.mjs'
import {
  resolveTestProjectsIsolation,
  withIsolatedProjectCreate,
} from '../lib/test-project-isolation.mjs'

function completeChecks() {
  return {
    claude: { ok: true },
    perception: { ok: true },
    matte: { ok: true },
    diarize: { ok: true },
    dub: { ok: true },
    zscale: { ok: true },
    clips: [
      { role: 'SCENE', ok: true },
      { role: 'SPEECH', ok: true },
      { role: 'FACE', ok: true },
      { role: 'SPEAKERS', ok: true },
    ],
  }
}

test('complete environment requires the matte runtime used by the inner UI gate', () => {
  const checks = completeChecks()
  checks.matte = { ok: false, detail: 'RVM model missing' }
  const verdict = assessCompleteEnv(checks, { requireFull: true })

  assert.equal(verdict.ok, false)
  assert.match(verdict.missing.join('\n'), /matte runtime is incomplete/)
  assert.match(verdict.missing.join('\n'), /RVM model missing/)
})

test('complete environment passes when matte and every other release dependency are ready', () => {
  const verdict = assessCompleteEnv(completeChecks(), { requireFull: true })
  assert.equal(verdict.ok, true)
  assert.deepEqual(verdict.missing, [])
})

test('external fixture mode requires confirmation that cutd inherited fixture env', () => {
  const verdict = assessExternalFixtureContract({
    external: true,
    fixtureActive: true,
    acknowledged: false,
  })
  assert.equal(verdict.ok, false)
  assert.match(verdict.missing[0], /FCV_EXTERNAL_FIXTURES_READY=1/)
  assert.match(verdict.missing[0], /CUTD_DRAFT_ADAPTER/)
})

test('cold-start and explicitly acknowledged external stacks satisfy the contract', () => {
  assert.equal(assessExternalFixtureContract({ external: false, fixtureActive: true, acknowledged: false }).ok, true)
  assert.equal(assessExternalFixtureContract({ external: true, fixtureActive: false, acknowledged: false }).ok, true)
  assert.equal(assessExternalFixtureContract({ external: true, fixtureActive: true, acknowledged: true }).ok, true)
})

test('surface labels follow the local platform unless an external driver names its target', () => {
  assert.equal(surfaceLabel({ platform: 'darwin' }), 'macos-installed')
  assert.equal(surfaceLabel({ platform: 'win32' }), 'windows-installed')
  assert.equal(surfaceLabel({ platform: 'linux' }), 'linux-control')
  assert.equal(surfaceLabel({ platform: 'linux', override: 'windows-installed', external: true }), 'windows-installed')
})

test('surface overrides cannot mislabel cold starts or use unknown targets', () => {
  assert.throws(
    () => surfaceLabel({ platform: 'linux', override: 'windows-installed', external: false }),
    /only valid when SWEEP_CUTD/,
  )
  assert.throws(
    () => surfaceLabel({ platform: 'linux', override: 'windows', external: true }),
    /must be one of/,
  )
})

test('cold-start UI tests use a run-owned repository scratch project root', () => {
  const isolation = resolveTestProjectsIsolation({
    external: false,
    configuredDir: '',
    repoDir: '/repo',
    receiptStem: 'linux-control-run',
    homeDir: '/home/example',
  })
  assert.deepEqual(isolation, {
    ok: true,
    dir: '/repo/.shellx-scratch/full-coverage/projects/linux-control-run',
    ownedByRun: true,
  })
})

test('external UI tests fail closed without a native isolated project root', () => {
  const missing = resolveTestProjectsIsolation({
    external: true,
    configuredDir: '',
    repoDir: '/repo',
    receiptStem: 'run',
    homeDir: '/home/example',
  })
  assert.equal(missing.ok, false)
  assert.match(missing.error, /require SHELLX_CUT_PROJECTS_DIR/)

  const configured = resolveTestProjectsIsolation({
    external: true,
    configuredDir: '/private-run/projects',
    repoDir: '/repo',
    receiptStem: 'run',
    homeDir: '/home/example',
  })
  assert.deepEqual(configured, {
    ok: true,
    dir: '/private-run/projects',
    ownedByRun: false,
  })
})

test('UI tests reject the normal user project library even when explicitly configured', () => {
  for (const configuredDir of [
    '/home/example/ShellX Cut Projects',
    '/home/example/Documents/ShellX Cut Projects/',
    'C:\\Users\\Example\\Documents\\ShellX Cut Projects',
    'C:\\Users\\SomeoneElse\\Documents\\ShellX Cut Projects',
  ]) {
    const homeDir = configuredDir.startsWith('C:') ? 'C:\\Users\\Example' : '/home/example'
    const isolation = resolveTestProjectsIsolation({
      external: true,
      configuredDir,
      repoDir: '/repo',
      receiptStem: 'run',
      homeDir,
    })
    assert.equal(isolation.ok, false, configuredDir)
    assert.match(isolation.error, /default user project library/)
  }
})

test('test project creation is forced under the isolated engine path', () => {
  assert.deepEqual(
    withIsolatedProjectCreate('project.create', { name: 'fcv_project' }, '/tmp/cut-run/projects'),
    { name: 'fcv_project', dir: '/tmp/cut-run/projects/fcv_project.cutproj' },
  )
  assert.deepEqual(
    withIsolatedProjectCreate('project.create', { name: 'iv_project' }, 'C:\\CutRun\\projects'),
    { name: 'iv_project', dir: 'C:\\CutRun\\projects\\iv_project.cutproj' },
  )
  const explicit = { name: 'release', dir: '/tmp/cut-run/projects/release.cutproj' }
  assert.equal(withIsolatedProjectCreate('project.create', explicit, '/tmp/cut-run/projects'), explicit)
  assert.throws(
    () => withIsolatedProjectCreate(
      'project.create',
      { name: 'stray', dir: '/home/example/ShellX Cut Projects/stray.cutproj' },
      '/tmp/cut-run/projects',
    ),
    /must stay inside SHELLX_CUT_PROJECTS_DIR/,
  )
})

test('every project-creating UI gate fails closed onto the shared isolation helper', () => {
  for (const relative of [
    '../../ui/public-tests/full-coverage-verify.mjs',
    '../../ui/public-tests/interaction-verify.mjs',
    '../../ui/public-tests/release-verify.mjs',
  ]) {
    const source = readFileSync(new URL(relative, import.meta.url), 'utf8')
    assert.match(source, /requireIsolatedTestProjectsDir/)
    assert.match(source, /withIsolatedProjectCreate/)
  }
  const wrapper = readFileSync(new URL('../release/full-coverage-gate.mjs', import.meta.url), 'utf8')
  assert.match(wrapper, /projectsIsolation[.]ownedByRun/)
  assert.match(wrapper, /rmSync\(projectsIsolation[.]dir/)
})

test('final all-actions mode accepts installed WebView2 CDP', () => {
  const base = {
    finalAllActions: true,
    requireFull: true,
    filtered: false,
    installedApp: true,
    uiDriver: 'webview2-cdp',
    cdpUrl: 'http://127.0.0.1:9223',
  }
  assert.equal(assessFinalAllActionsConfig(base).ok, true)
  const browserOnly = assessFinalAllActionsConfig({
    ...base,
    uiDriver: 'tauri-webdriver',
    cdpUrl: '',
  })
  assert.equal(browserOnly.ok, false)
  assert.match(browserOnly.missing.join('\n'), /installed WebView2 CDP/)
})

test('final all-actions mode accepts official external Tauri WebDriver on native Linux', () => {
  const verdict = assessFinalAllActionsConfig({
    finalAllActions: true,
    requireFull: true,
    filtered: false,
    installedApp: true,
    uiDriver: 'tauri-wdio',
    nativeProvider: 'external',
    platform: 'linux',
  })
  assert.equal(verdict.ok, true)
})

test('embedded or macOS Tauri WebDriver cannot claim installed final proof', () => {
  const embedded = assessFinalAllActionsConfig({
    finalAllActions: true,
    requireFull: true,
    filtered: false,
    installedApp: true,
    uiDriver: 'tauri-wdio',
    nativeProvider: 'embedded',
    platform: 'linux',
  })
  const macExternal = assessFinalAllActionsConfig({
    finalAllActions: true,
    requireFull: true,
    filtered: false,
    installedApp: true,
    uiDriver: 'tauri-wdio',
    nativeProvider: 'external',
    platform: 'darwin',
  })
  assert.equal(embedded.ok, false)
  assert.equal(macExternal.ok, false)
  assert.match(macExternal.missing.join('\n'), /native Linux\/Windows/)
})

test('fixture PATH prepend preserves Windows Path casing and existing entries', () => {
  const env = { Path: 'C:\\Windows\\System32;C:\\Program Files\\nodejs' }
  const key = prependEnvPath(env, 'C:\\release-fixtures', { platform: 'win32' })

  assert.equal(key, 'Path')
  assert.equal(env.Path, 'C:\\release-fixtures;C:\\Windows\\System32;C:\\Program Files\\nodejs')
  assert.equal(Object.hasOwn(env, 'PATH'), false)
})
