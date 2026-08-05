import test, { afterEach } from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

import {
  createExactSourceReceipt,
  evidencePassed,
  parseExactSourceArgs,
} from '../lib/exact-source-receipt.mjs'
import { buildFullCoverageReceipt } from '../lib/full-coverage-receipt.mjs'
import { collectSourceIdentity } from '../lib/ignored-test-rig.mjs'
import { sourceContentManifest } from '../lib/source-content-manifest.mjs'

const fixtureRoots = []

afterEach(() => {
  for (const root of fixtureRoots.splice(0)) rmSync(root, { recursive: true, force: true })
})

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'cut-exact-source-producer-'))
  fixtureRoots.push(root)
  const repo = join(root, 'repo')
  mkdirSync(join(repo, 'app', 'desktop', 'src-tauri'), { recursive: true })
  mkdirSync(join(repo, 'scripts', 'release'), { recursive: true })
  for (const dir of ['docs', 'schema', 'skill', 'testdata', 'ui']) mkdirSync(join(repo, dir))
  for (const name of ['.gitignore', 'AGENTS.md', 'LICENSE', 'NOTICE', 'START_HERE_FOR_AGENT.txt']) {
    writeFileSync(join(repo, name), `${name}\n`)
  }
  for (const dir of ['docs', 'schema', 'skill', 'ui']) {
    writeFileSync(join(repo, dir, 'fixture.txt'), `${dir}\n`)
  }
  writeFileSync(join(repo, 'testdata', 'test_lut_invert.cube'), 'tracked LUT fixture\n')
  writeFileSync(join(repo, 'app', 'Cargo.lock'), '# fixture lock\n')
  writeFileSync(join(repo, 'app', 'desktop', 'src-tauri', 'tauri.conf.json'), JSON.stringify({ version: '9.8.7' }))
  writeFileSync(join(repo, 'scripts', 'release', 'ignored-test-rigs.json'), JSON.stringify({
    receiptSchema: 'shellx-cut/ignored-test-rig-receipt/1',
    tests: [{
      id: 'real-4k-gpu',
      rustTest: 'real_4k_render_gpu_only',
      classification: 'hardware',
      platforms: ['linux'],
      command: ['cargo', 'test', 'real_4k_render_gpu_only', '--', '--ignored'],
      inputArtifacts: [{ env: 'SHELLX_CUT_TEST_4K' }],
      outputArtifacts: [{ env: 'SHELLX_CUT_TEST_OUT_DIR', join: 'real4k_gpu.mp4' }],
    }],
  }))
  writeFileSync(join(repo, 'README.md'), 'fixture\n')
  execFileSync('git', ['init'], { cwd: repo, stdio: 'ignore' })
  execFileSync('git', ['config', 'user.email', 'fixture@example.invalid'], { cwd: repo })
  execFileSync('git', ['config', 'user.name', 'Fixture'], { cwd: repo })
  execFileSync('git', ['add', '.'], { cwd: repo })
  execFileSync('git', ['commit', '-m', 'fixture'], { cwd: repo, stdio: 'ignore' })
  const artifact = join(root, 'cutd.bin')
  const evidence = join(root, 'results.json')
  writeFileSync(artifact, 'artifact bytes')
  writeFileSync(evidence, JSON.stringify({ schema: 'fixture/results@1', status: 'pass' }))
  return { root, repo, artifact, evidence, out: join(root, 'private', 'receipt.json') }
}

function receiptArtifact(seed) {
  return { exists: true, bytes: 10, sha256: seed.repeat(64) }
}

function ignoredRigReceipt(repo) {
  const source = collectSourceIdentity(repo)
  return {
    schema: 'shellx-cut/ignored-test-rig-receipt/1',
    id: 'real-4k-gpu',
    rustTest: 'real_4k_render_gpu_only',
    classification: 'hardware',
    source,
    host: { platform: 'linux', arch: 'x64' },
    command: ['cargo', 'test', 'real_4k_render_gpu_only', '--', '--ignored'],
    preflight: { platformAllowed: true, missing: [] },
    result: { status: 0, signal: null, error: null },
    artifacts: {
      inputs: [receiptArtifact('a')],
      outputs: [receiptArtifact('b')],
      testBinary: receiptArtifact('c'),
      stdout: receiptArtifact('d'),
      stderr: receiptArtifact('e'),
    },
    pass: true,
  }
}

function realDropProject(kind) {
  const name = `real-drop-${kind}`
  return {
    kind,
    name,
    native: { input: 'real' },
    state: {
      name,
      settings: { width: 1920, height: 1080, fps: 30 },
      assets: {
        a1: { probe: kind === 'video' ? { width: 1920, height: 1080, fps: 30 } : {} },
      },
      tracks: [{
        kind: 'video',
        clips: [{ src_in_ms: 0, src_out_ms: kind === 'image' ? 5_000 : 10_000, speed: 1 }],
      }],
    },
  }
}

const REAL_DROP_CASES = [
  {
    schema: 'shellx-cut/windows-installed-real-file-drop@1',
    surface: 'windows-installed',
    platform: 'win32',
    gesture: 'real-explorer-ole-file-drag',
    manager: 'explorer',
  },
  {
    schema: 'shellx-cut/macos-installed-real-file-drop@1',
    surface: 'macos-installed',
    platform: 'darwin',
    gesture: 'real-finder-file-window-drag',
    manager: 'finder',
  },
  {
    schema: 'shellx-cut/linux-installed-real-file-drop@1',
    surface: 'linux-control',
    platform: 'linux',
    gesture: 'real-nautilus-x11-file-drag',
    manager: 'nautilus',
  },
]

function realDropReceipt(repo, shellSha256, dropCase = REAL_DROP_CASES[0]) {
  const source = collectSourceIdentity(repo)
  return {
    schema: dropCase.schema,
    ok: true,
    installedApp: true,
    platform: dropCase.platform,
    gesture: dropCase.gesture,
    source: { head: source.gitCommit },
    runtime: {
      shell: { sha256: shellSha256 },
      cutd: receiptArtifact('b'),
    },
    media: {
      video: receiptArtifact('c'),
      image: receiptArtifact('d'),
    },
    checks: [
      { id: 'projects-first', pass: true },
      { id: `video-real-${dropCase.manager}-drop-create`, pass: true },
      { id: `image-real-${dropCase.manager}-drop-create`, pass: true },
    ],
    projects: [realDropProject('video'), realDropProject('image')],
  }
}

function installedWalkthroughReceipt(repo, artifactSha256, surface = 'windows-installed') {
  const source = collectSourceIdentity(repo)
  return {
    schema: 'shellx-cut/installed-surface-walkthrough@1',
    status: 'pass',
    installedApp: true,
    surface,
    source: {
      gitCommit: source.gitCommit,
      version: source.version,
      contentManifestSha256: sourceContentManifest(repo).sha256,
    },
    artifact: {
      sha256: artifactSha256,
      version: source.version,
      integrityVerified: true,
      webdriverTestFeatureAbsent: true,
      signed: surface === 'windows-installed',
      notarized: surface === 'macos-installed',
    },
    rows: ['installed-agent-docs', 'settings', 'library', 'about', 'debug-api', 'mcp-self-test']
      .map((id) => ({ id, status: 'pass' })),
  }
}

function fileSha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

function updaterReceipt(repo, files) {
  const source = collectSourceIdentity(repo)
  const platforms = ['darwin-aarch64', 'windows-x86_64']
  return {
    schema: 'shellx-cut/updater-manifest-verify@1',
    status: 'pass',
    source: {
      gitCommit: source.gitCommit,
      gitDirty: false,
      version: source.version,
      cargoLockSha256: source.cargoLock.sha256,
      contentManifestSha256: sourceContentManifest(repo).sha256,
    },
    release: {
      repository: 'martinsbrezauckis/shellx-cut',
      tag: `v${source.version}`,
      version: source.version,
    },
    manifest: {
      name: 'latest.json',
      bytes: readFileSync(files.manifest).length,
      sha256: fileSha256(files.manifest),
      platforms,
    },
    artifacts: [
      {
        platform: 'windows-x86_64',
        name: 'ShellX Cut_9.8.7_x64-setup.exe',
        bytes: readFileSync(files.windows).length,
        sha256: fileSha256(files.windows),
        signatureName: 'ShellX Cut_9.8.7_x64-setup.exe.sig',
        signatureBytes: readFileSync(files.windowsSig).length,
        signatureSha256: fileSha256(files.windowsSig),
        signatureVerified: true,
        url: `https://github.com/martinsbrezauckis/shellx-cut/releases/download/v${source.version}/ShellX%20Cut_9.8.7_x64-setup.exe`,
      },
      {
        platform: 'darwin-aarch64',
        name: 'ShellX Cut.app.tar.gz',
        bytes: readFileSync(files.macos).length,
        sha256: fileSha256(files.macos),
        signatureName: 'ShellX Cut.app.tar.gz.sig',
        signatureBytes: readFileSync(files.macosSig).length,
        signatureSha256: fileSha256(files.macosSig),
        signatureVerified: true,
        url: `https://github.com/martinsbrezauckis/shellx-cut/releases/download/v${source.version}/ShellX%20Cut.app.tar.gz`,
      },
    ],
    checks: [
      'artifact-minisign-verified-against-embedded-pubkey',
      'all-required-platforms-present',
      'release-url-version-bound',
    ],
  }
}

test('CLI parser keeps repeated capabilities and named paths explicit', () => {
  const args = parseExactSourceArgs([
    '--surface', 'linux-control',
    '--capability', 'full-coverage',
    '--capability', 'real-4k-gpu',
    '--artifact', 'cutd=/tmp/cutd',
    '--artifact-tree', 'ui=/tmp/dist',
    '--evidence', 'results=/tmp/results.json',
    '--out', '/tmp/receipt.json',
  ])
  assert.equal(args.surface, 'linux-control')
  assert.deepEqual(args.capabilities, ['full-coverage', 'real-4k-gpu'])
  assert.deepEqual(args.artifacts.map(({ name, tree }) => ({ name, tree })), [
    { name: 'cutd', tree: false },
    { name: 'ui', tree: true },
  ])
  assert.equal(args.evidence[0].name, 'results')
})

test('pass detection handles the established receipt shapes', () => {
  assert.equal(evidencePassed({ status: 'pass' }), true)
  assert.equal(evidencePassed({ ok: true }), true)
  assert.equal(evidencePassed({ pass: true }), true)
  assert.equal(evidencePassed({ summary: { total: 4, fail: 0 } }), true)
  assert.equal(evidencePassed({ status: 'fail', summary: { total: 4, fail: 1 } }), false)
  assert.equal(evidencePassed({ status: 'fail', ok: true }), false)
})

test('producer binds a clean source to independently hashed artifact and evidence', () => {
  const f = fixture()
  const result = createExactSourceReceipt({
    repoRoot: f.repo,
    surface: 'linux-control',
    capabilities: ['full-coverage'],
    artifacts: [{ name: 'cutd', path: f.artifact, tree: false }],
    evidence: [{ name: 'results', path: f.evidence }],
    outPath: f.out,
    generatedAt: '2026-07-11T08:00:00.000Z',
  })
  assert.equal(result.receipt.schema, 'shellx-cut/exact-source-rig@1')
  assert.equal(result.receipt.source.gitDirty, false)
  assert.match(result.receipt.source.gitCommit, /^[a-f0-9]{40}$/)
  assert.match(result.receipt.source.cargoLockSha256, /^[a-f0-9]{64}$/)
  assert.match(result.receipt.source.contentManifestSha256, /^[a-f0-9]{64}$/)
  assert.ok(result.receipt.source.contentManifestFiles > 0)
  assert.match(result.receipt.artifacts[0].sha256, /^[a-f0-9]{64}$/)
  assert.match(result.receipt.evidence[0].sha256, /^[a-f0-9]{64}$/)
  assert.deepEqual(JSON.parse(readFileSync(f.out, 'utf8')), result.receipt)
})

test('producer preserves strict full-UI matrix claims while allowing repeated probe rows', () => {
  const f = fixture()
  const contentManifest = sourceContentManifest(f.repo)
  const actionIds = ['settings::setup-btn', 'library::library-search']
  const fullReceipt = buildFullCoverageReceipt([...actionIds, actionIds[0]].map((actionId, index) => ({
    actionId,
    rowKind: 'ui_action',
    surface: actionId.split('::')[0],
    name: `${actionId.split('::')[1]}-${index}`,
    present: 'pass',
    render: 'pass',
    click: 'pass',
    result: 'pass',
    evidence: 'real builder regression fixture',
  })), {
    full: true,
    strictAllActions: true,
    surface: 'linux-control',
    runtime: {
      installedApp: true,
      driver: 'linux-native-webview',
      nativeAttached: true,
      nativeProvider: 'external',
      sourceContentManifestSha256: contentManifest.sha256,
    },
    sourceActionIds: actionIds,
    expectedSourceActionIds: actionIds,
    runtimeSourceActionIds: actionIds,
    expectedRuntimeSourceActionIds: actionIds,
  })
  assert.equal(fullReceipt.actionManifest.repeated.length, 1)
  writeFileSync(f.evidence, JSON.stringify(fullReceipt))
  const result = createExactSourceReceipt({
    repoRoot: f.repo,
    surface: 'linux-control',
    capabilities: ['full-ui-action-matrix'],
    artifacts: [{ name: 'cutd', path: f.artifact, tree: false }],
    evidence: [{ name: 'full-ui-actions', path: f.evidence }],
    outPath: f.out,
  })
  assert.deepEqual(result.receipt.evidence[0].fullUiActionMatrix, {
    strictAllActions: true,
    surface: 'linux-control',
    installedApp: true,
    driver: 'linux-native-webview',
    nativeAttached: true,
    nativeProvider: 'external',
    sourceContentManifestSha256: contentManifest.sha256,
    actionManifestSha256: fullReceipt.runtimeSourceActionManifest.sha256,
    expectedActionManifestSha256: fullReceipt.sourceActionManifest.expectedSha256,
    actionManifestMatchesExpected: true,
    total: actionIds.length,
    duplicateCount: 0,
    fullyVerified: actionIds.length,
    strictUnverified: 0,
    failures: 0,
  })
})

test('producer rejects a strict UI receipt from different synchronized content', () => {
  const f = fixture()
  writeFileSync(f.evidence, JSON.stringify({
    schema: 'shellx-cut/full-coverage-results@1',
    ok: true,
    strictAllActions: true,
    surface: 'linux-control',
    runtime: {
      installedApp: true,
      driver: 'linux-native-webview',
      nativeAttached: true,
      sourceContentManifestSha256: 'f'.repeat(64),
    },
    sourceActionManifest: {
      sha256: 'e'.repeat(64), expectedSha256: 'e'.repeat(64), matchesExpected: true,
      total: 412,
    },
    runtimeSourceActionManifest: {
      sha256: 'e'.repeat(64), expectedSha256: 'e'.repeat(64), matchesExpected: true,
      total: 412,
    },
    summary: { controls: { fullyVerified: 412, strictUnverified: 0, failures: 0 } },
  }))
  assert.throws(() => createExactSourceReceipt({
    repoRoot: f.repo,
    surface: 'linux-control',
    capabilities: ['full-ui-action-matrix'],
    artifacts: [{ name: 'cutd', path: f.artifact, tree: false }],
    evidence: [{ name: 'full-ui-actions', path: f.evidence }],
    outPath: f.out,
  }), /source-content-matched receipt/)
})

test('producer binds ignored-test capabilities to their exact classified receipt', () => {
  const f = fixture()
  writeFileSync(f.evidence, JSON.stringify(ignoredRigReceipt(f.repo)))
  const result = createExactSourceReceipt({
    repoRoot: f.repo,
    surface: 'linux-control',
    capabilities: ['real-4k-gpu'],
    artifacts: [{ name: 'cutd', path: f.artifact, tree: false }],
    evidence: [{ name: 'real-4k-gpu', path: f.evidence }],
    outPath: f.out,
  })
  assert.equal(result.receipt.evidence[0].ignoredTestRig.id, 'real-4k-gpu')
  assert.equal(result.receipt.evidence[0].ignoredTestRig.gitCommit, result.receipt.source.gitCommit)
  assert.match(result.receipt.evidence[0].ignoredTestRig.testBinarySha256, /^[a-f0-9]{64}$/)
})

test('producer rejects generic or source-mismatched ignored-test claims', () => {
  const generic = fixture()
  assert.throws(() => createExactSourceReceipt({
    repoRoot: generic.repo,
    surface: 'linux-control',
    capabilities: ['real-4k-gpu'],
    artifacts: [{ name: 'cutd', path: generic.artifact, tree: false }],
    evidence: [{ name: 'results', path: generic.evidence }],
    outPath: generic.out,
  }), /requires exactly one matching rig receipt/)

  const mismatch = fixture()
  const receipt = ignoredRigReceipt(mismatch.repo)
  receipt.source.gitCommit = 'f'.repeat(40)
  writeFileSync(mismatch.evidence, JSON.stringify(receipt))
  assert.throws(() => createExactSourceReceipt({
    repoRoot: mismatch.repo,
    surface: 'linux-control',
    capabilities: ['real-4k-gpu'],
    artifacts: [{ name: 'cutd', path: mismatch.artifact, tree: false }],
    evidence: [{ name: 'real-4k-gpu', path: mismatch.evidence }],
    outPath: mismatch.out,
  }), /source commit does not match/)
})

test('producer binds updater capability to both signed platforms and latest.json', () => {
  const f = fixture()
  const files = {
    windows: join(f.root, 'ShellX Cut_9.8.7_x64-setup.exe'),
    windowsSig: join(f.root, 'ShellX Cut_9.8.7_x64-setup.exe.sig'),
    macos: join(f.root, 'ShellX Cut.app.tar.gz'),
    macosSig: join(f.root, 'ShellX Cut.app.tar.gz.sig'),
    manifest: join(f.root, 'latest.json'),
  }
  for (const [name, path] of Object.entries(files)) writeFileSync(path, `${name} bytes`)
  writeFileSync(f.evidence, JSON.stringify(updaterReceipt(f.repo, files)))
  const result = createExactSourceReceipt({
    repoRoot: f.repo,
    surface: 'windows-installed',
    capabilities: ['updater'],
    artifacts: Object.entries(files).map(([name, path]) => ({ name: name.toLowerCase(), path, tree: false })),
    evidence: [{ name: 'updater-verification', path: f.evidence }],
    outPath: f.out,
  })
  assert.deepEqual(result.receipt.evidence[0].updaterManifest.platforms, [
    'darwin-aarch64',
    'windows-x86_64',
  ])
  assert.equal(
    result.receipt.evidence[0].updaterManifest.artifactSha256['windows-x86_64'],
    fileSha256(files.windows),
  )
})

test('producer rejects generic, partial, or unbound updater claims', () => {
  const generic = fixture()
  assert.throws(() => createExactSourceReceipt({
    repoRoot: generic.repo,
    surface: 'windows-installed',
    capabilities: ['updater'],
    artifacts: [{ name: 'cutd', path: generic.artifact, tree: false }],
    evidence: [{ name: 'results', path: generic.evidence }],
    outPath: generic.out,
  }), /matching updater verification receipt/)

  const partial = fixture()
  const files = {
    windows: join(partial.root, 'windows.exe'),
    windowsSig: join(partial.root, 'windows.exe.sig'),
    macos: join(partial.root, 'macos.tar.gz'),
    macosSig: join(partial.root, 'macos.tar.gz.sig'),
    manifest: join(partial.root, 'latest.json'),
  }
  for (const [name, path] of Object.entries(files)) writeFileSync(path, `${name} bytes`)
  const receipt = updaterReceipt(partial.repo, files)
  receipt.artifacts = receipt.artifacts.filter((item) => item.platform !== 'darwin-aarch64')
  writeFileSync(partial.evidence, JSON.stringify(receipt))
  assert.throws(() => createExactSourceReceipt({
    repoRoot: partial.repo,
    surface: 'windows-installed',
    capabilities: ['updater'],
    artifacts: Object.entries(files).map(([name, path]) => ({ name: name.toLowerCase(), path, tree: false })),
    evidence: [{ name: 'updater-verification', path: partial.evidence }],
    outPath: partial.out,
  }), /updater evidence.*artifact count.*darwin-aarch64/)
})

test('producer binds every real file-manager drop schema to source, installed state, and shell artifact', () => {
  for (const dropCase of REAL_DROP_CASES) {
    const f = fixture()
    const shellSha256 = receiptArtifact('a').sha256
    writeFileSync(f.artifact, 'installed shell bytes')
    const actualShell = createHash('sha256').update(readFileSync(f.artifact)).digest('hex')
    const receipt = realDropReceipt(f.repo, actualShell, dropCase)
    writeFileSync(f.evidence, JSON.stringify(receipt))
    const result = createExactSourceReceipt({
      repoRoot: f.repo,
      surface: dropCase.surface,
      capabilities: ['file-manager-drop-video', 'file-manager-drop-image'],
      artifacts: [{ name: 'installed-shell', path: f.artifact, tree: false }],
      evidence: [{ name: `real-${dropCase.manager}-drop`, path: f.evidence }],
      outPath: f.out,
    })
    assert.deepEqual(result.receipt.evidence[0].realFileDrop.cases, ['video', 'image'])
    assert.equal(result.receipt.evidence[0].realFileDrop.shellSha256, actualShell)
    assert.notEqual(result.receipt.evidence[0].realFileDrop.shellSha256, shellSha256)
  }
})

test('producer rejects generic, synthetic, stale-source, and wrong-artifact drop claims', () => {
  const generic = fixture()
  assert.throws(() => createExactSourceReceipt({
    repoRoot: generic.repo,
    surface: 'windows-installed',
    capabilities: ['file-manager-drop-video'],
    artifacts: [{ name: 'installed-shell', path: generic.artifact, tree: false }],
    evidence: [{ name: 'generic', path: generic.evidence }],
    outPath: generic.out,
  }), /requires exactly one matching real file-drop receipt/)

  for (const mutate of [
    (receipt) => { receipt.gesture = 'webdriver-test-only-tauri-event-bridge' },
    (receipt) => { receipt.source.head = 'f'.repeat(40) },
    (receipt) => { receipt.runtime.shell.sha256 = 'e'.repeat(64) },
    (receipt) => { receipt.projects[0].native = {} },
    (receipt) => { receipt.checks.pop() },
    (receipt) => { receipt.checks[1].pass = false },
    (receipt) => { receipt.projects[0].state.settings.width = 1280 },
    (receipt) => { receipt.projects[1].state.tracks[0].clips[0].src_out_ms = 4_000 },
  ]) {
    const f = fixture()
    const actualShell = createHash('sha256').update(readFileSync(f.artifact)).digest('hex')
    const receipt = realDropReceipt(f.repo, actualShell)
    mutate(receipt)
    writeFileSync(f.evidence, JSON.stringify(receipt))
    assert.throws(() => createExactSourceReceipt({
      repoRoot: f.repo,
      surface: 'windows-installed',
      capabilities: ['file-manager-drop-video', 'file-manager-drop-image'],
      artifacts: [{ name: 'installed-shell', path: f.artifact, tree: false }],
      evidence: [{ name: 'real-explorer-drop', path: f.evidence }],
      outPath: f.out,
    }), /real file-drop evidence/)
  }
})

test('producer binds installed docs and human control walkthrough to the shipping artifact', () => {
  for (const surface of ['windows-installed', 'macos-installed', 'linux-control']) {
    const f = fixture()
    const artifactSha256 = createHash('sha256').update(readFileSync(f.artifact)).digest('hex')
    writeFileSync(f.evidence, JSON.stringify(installedWalkthroughReceipt(f.repo, artifactSha256, surface)))
    const result = createExactSourceReceipt({
      repoRoot: f.repo,
      surface,
      capabilities: ['installed-agent-docs', 'settings-library-debug-mcp-walkthrough'],
      artifacts: [{ name: 'shipping-app', path: f.artifact, tree: false }],
      evidence: [{ name: 'installed-walkthrough', path: f.evidence }],
      outPath: f.out,
    })
    assert.deepEqual(result.receipt.evidence[0].installedWalkthrough.rows, [
      'installed-agent-docs', 'settings', 'library', 'about', 'debug-api', 'mcp-self-test',
    ])
  }
})

test('producer rejects generic, incomplete, unsealed, and instrumented walkthrough claims', () => {
  const generic = fixture()
  assert.throws(() => createExactSourceReceipt({
    repoRoot: generic.repo,
    surface: 'windows-installed',
    capabilities: ['installed-agent-docs'],
    artifacts: [{ name: 'shipping-app', path: generic.artifact, tree: false }],
    evidence: [{ name: 'generic', path: generic.evidence }],
    outPath: generic.out,
  }), /matching installed walkthrough receipt/)

  for (const mutate of [
    (receipt) => { receipt.rows.pop() },
    (receipt) => { receipt.rows[0].status = 'na' },
    (receipt) => { receipt.artifact.integrityVerified = false },
    (receipt) => { receipt.artifact.webdriverTestFeatureAbsent = false },
    (receipt) => { receipt.artifact.sha256 = 'f'.repeat(64) },
  ]) {
    const f = fixture()
    const artifactSha256 = createHash('sha256').update(readFileSync(f.artifact)).digest('hex')
    const receipt = installedWalkthroughReceipt(f.repo, artifactSha256)
    mutate(receipt)
    writeFileSync(f.evidence, JSON.stringify(receipt))
    assert.throws(() => createExactSourceReceipt({
      repoRoot: f.repo,
      surface: 'windows-installed',
      capabilities: ['installed-agent-docs', 'settings-library-debug-mcp-walkthrough'],
      artifacts: [{ name: 'shipping-app', path: f.artifact, tree: false }],
      evidence: [{ name: 'installed-walkthrough', path: f.evidence }],
      outPath: f.out,
    }), /installed walkthrough evidence/)
  }
})

test('producer refuses dirty source and non-passing child evidence', () => {
  const dirty = fixture()
  writeFileSync(join(dirty.repo, 'README.md'), 'changed\n')
  assert.throws(() => createExactSourceReceipt({
    repoRoot: dirty.repo,
    surface: 'linux-control',
    capabilities: ['full-coverage'],
    artifacts: [{ name: 'cutd', path: dirty.artifact, tree: false }],
    evidence: [{ name: 'results', path: dirty.evidence }],
    outPath: dirty.out,
  }), /clean product worktree/)

  const failed = fixture()
  writeFileSync(failed.evidence, JSON.stringify({ status: 'fail' }))
  assert.throws(() => createExactSourceReceipt({
    repoRoot: failed.repo,
    surface: 'linux-control',
    capabilities: ['full-coverage'],
    artifacts: [{ name: 'cutd', path: failed.artifact, tree: false }],
    evidence: [{ name: 'results', path: failed.evidence }],
    outPath: failed.out,
  }), /not a passing receipt/)
})

test('producer refuses evidence output inside the public product tree', () => {
  const f = fixture()
  assert.throws(() => createExactSourceReceipt({
    repoRoot: f.repo,
    surface: 'linux-control',
    capabilities: ['full-coverage'],
    artifacts: [{ name: 'cutd', path: f.artifact, tree: false }],
    evidence: [{ name: 'results', path: f.evidence }],
    outPath: join(f.repo, 'release-evidence.json'),
  }), /outside the product repo/)
})
