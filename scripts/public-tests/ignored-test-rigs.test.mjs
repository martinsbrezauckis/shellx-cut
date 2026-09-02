import test from 'node:test'
import assert from 'node:assert/strict'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import {
  cargoTestBinary,
  collectSourceIdentity,
  commandExists,
  discoverIgnoredRustTests,
  envValue,
  loadIgnoredTestManifest,
  manifestPlatform,
  parseIgnoredRigArgs,
  rigExecutionEnv,
  resolveIgnoredTestRigReceiptDir,
  runIgnoredTestRig,
} from '../lib/ignored-test-rig.mjs'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const manifest = loadIgnoredTestManifest(repoRoot)

test('every ignored Rust test has exactly one rig classification', () => {
  const discovered = discoverIgnoredRustTests(repoRoot)
  const declared = manifest.tests
    .map(({ rustTest, source }) => ({ rustTest, source }))
    .sort((a, b) => a.rustTest.localeCompare(b.rustTest))
  assert.deepEqual(discovered, declared)
  assert.equal(discovered.length, 10)
  assert.equal(new Set(manifest.tests.map((entry) => entry.id)).size, manifest.tests.length)
})

test('rig definitions carry executable receipt contracts', () => {
  const classifications = new Set([
    'perception_runtime',
    'real_media',
    'hardware',
    'permission',
    'authenticated_service',
    'deterministic_host',
  ])
  for (const rig of manifest.tests) {
    assert.ok(classifications.has(rig.classification), `${rig.id}: classification`)
    assert.ok(rig.platforms.length > 0, `${rig.id}: platforms`)
    assert.ok(rig.requirements.length > 0, `${rig.id}: requirements`)
    assert.ok(rig.command.includes(rig.rustTest), `${rig.id}: exact test filter`)
    assert.ok(rig.command.includes('--ignored'), `${rig.id}: ignored runner flag`)
    assert.ok(rig.testBinaryPrefix, `${rig.id}: test binary binding`)
  }
})

test('requiredEnv is the exact runtime-adapter contract for every ignored rig', () => {
  const expected = {
    'perception-full-battery': ['SHELLX_CUT_SIDECAR_DIR', 'SHELLX_CUT_PYTHON'],
    'perception-audio-battery': ['SHELLX_CUT_SIDECAR_DIR', 'SHELLX_CUT_PYTHON'],
    'perception-base-fallback': [],
    'silent-footage-profile': ['SHELLX_CUT_SIDECAR_DIR', 'SHELLX_CUT_PYTHON'],
    'real-4k-software': ['SHELLX_CUT_TEST_4K', 'SHELLX_CUT_TEST_OUT_DIR'],
    'real-4k-gpu': ['SHELLX_CUT_FFMPEG', 'SHELLX_CUT_TEST_4K', 'SHELLX_CUT_TEST_OUT_DIR'],
    'real-4k-vram-fallback': ['SHELLX_CUT_TEST_4K', 'SHELLX_CUT_TEST_OUT_DIR'],
    'gstreamer-sparse-checkpoint': [],
    'hw-encoder-size-gate': ['SHELLX_CUT_FFMPEG'],
    'stabilize-quality': [],
  }
  assert.deepEqual(
    Object.fromEntries(manifest.tests.map((rig) => [rig.id, rig.requiredEnv])),
    expected,
  )
})

test('runtime-adapter manifest entries stay tied to the source contracts they execute', () => {
  const sidecar = readFileSync(resolve(repoRoot, 'app/perception/src/sidecar.rs'), 'utf8')
  const media = readFileSync(resolve(repoRoot, 'app/media/tests/media_engine.rs'), 'utf8')
  assert.match(sidecar, /ENV_SIDECAR_DIR: &str = "SHELLX_CUT_SIDECAR_DIR"/)
  assert.match(sidecar, /ENV_PYTHON: &str = "SHELLX_CUT_PYTHON"/)
  assert.match(media, /var\("SHELLX_CUT_TEST_4K"\)/)
  assert.match(media, /var\("SHELLX_CUT_TEST_OUT_DIR"\)/)
})

test('native platform ids match the manifest vocabulary', () => {
  assert.equal(manifestPlatform('linux'), 'linux')
  assert.equal(manifestPlatform('darwin'), 'macos')
  assert.equal(manifestPlatform('win32'), 'windows')
})

test('manifest never advertises removed live-host source tests', () => {
  for (const id of ['desktop-system-audio', 'screen-capture-doctor', 'live-claude-judge']) {
    assert.equal(manifest.tests.some((entry) => entry.id === id), false, `${id} must live in private native qualification, not this source manifest`)
  }
})

test('receipt source identity binds version, commit, and Cargo.lock', () => {
  const identity = collectSourceIdentity(repoRoot)
  assert.match(identity.version, /^\d+\.\d+\.\d+$/)
  assert.match(identity.gitCommit, /^[0-9a-f]{40}$/)
  assert.equal(identity.cargoLock.exists, true)
  assert.match(identity.cargoLock.sha256, /^[0-9a-f]{64}$/)
})

test('receipt binds the exact test binary reported by Cargo', () => {
  const logs = 'Running unittests src/main.rs (app/target/debug/deps/cutd-0123abcd)\n'
  assert.equal(
    cargoTestBinary(repoRoot, logs, 'cutd-'),
    resolve(repoRoot, 'app/target/debug/deps/cutd-0123abcd'),
  )
  assert.equal(cargoTestBinary(repoRoot, logs, 'media_engine-'), null)
})

test('perception runtime adapters default to the checked-out sidecar without overriding callers', () => {
  const full = manifest.tests.find((rig) => rig.id === 'perception-full-battery')
  const fallback = manifest.tests.find((rig) => rig.id === 'perception-base-fallback')
  const linux = rigExecutionEnv(repoRoot, full, { PATH: '/bin' }, 'linux')
  assert.equal(linux.env.SHELLX_CUT_SIDECAR_DIR, resolve(repoRoot, 'app/perception/py'))
  assert.equal(linux.env.SHELLX_CUT_PYTHON, resolve(repoRoot, 'app/perception/py/.venv/bin/python'))
  assert.deepEqual(linux.defaults.map(({ name }) => name), [
    'SHELLX_CUT_SIDECAR_DIR',
    'SHELLX_CUT_PYTHON',
  ])

  const windows = rigExecutionEnv(repoRoot, full, {}, 'win32')
  assert.equal(windows.env.SHELLX_CUT_PYTHON, resolve(repoRoot, 'app/perception/py/.venv/Scripts/python.exe'))

  const explicit = rigExecutionEnv(repoRoot, full, {
    SHELLX_CUT_SIDECAR_DIR: '/opt/cut-sidecar',
    SHELLX_CUT_PYTHON: '/opt/cut-python',
  })
  assert.equal(explicit.env.SHELLX_CUT_SIDECAR_DIR, '/opt/cut-sidecar')
  assert.equal(explicit.env.SHELLX_CUT_PYTHON, '/opt/cut-python')
  assert.deepEqual(explicit.defaults, [])

  const baseFallback = rigExecutionEnv(repoRoot, fallback, {}, 'linux')
  assert.equal(baseFallback.env.SHELLX_CUT_SIDECAR_DIR, undefined)
  assert.equal(baseFallback.env.SHELLX_CUT_PYTHON, undefined)
  assert.deepEqual(baseFallback.defaults, [])
})

// Both cases below are environment-shaped: they cannot be reproduced from this
// repo's own shell, so the platform and env are injected rather than trusted.
// Regressions here are silent and expensive — they made every Windows rig
// unrunnable (case) or let a rig fail after preflight passed (Store alias).

test('environment lookup ignores case, because a spread of process.env drops its proxy', () => {
  // Windows spells it `Path`. `process.env` resolves that case-insensitively,
  // but `{ ...process.env }` is a plain object and does not.
  assert.equal(envValue({ Path: 'C:\\bin' }, 'PATH'), 'C:\\bin')
  assert.equal(envValue({ PATH: '/usr/bin' }, 'PATH'), '/usr/bin')
  assert.equal(envValue({ path: '/usr/bin' }, 'PATH'), '/usr/bin')
  // An exact match still wins, and a genuinely absent name stays undefined.
  assert.equal(envValue({ PATH: '/a', Path: '/b' }, 'PATH'), '/a')
  assert.equal(envValue({}, 'PATH'), undefined)
})

test('command lookup finds real executables and rejects zero-byte Store aliases', () => {
  const dir = mkdtempSync(resolve(tmpdir(), 'cut-rig-cmd-'))
  const bin = resolve(dir, 'bin')
  mkdirSync(bin)
  // Extensions are spelled to MATCH the PATHEXT entries below. Real Windows
  // resolves these case-insensitively at the filesystem layer, but this test
  // runs on a case-sensitive filesystem, so the fixture must be self-consistent
  // to exercise the lookup logic rather than the host's path semantics.
  writeFileSync(resolve(bin, 'cargo.EXE'), 'MZ real executable content')
  // A Microsoft-Store app execution alias: exists, satisfies existsSync, is a
  // zero-byte reparse point, and cannot run. Preflight must not accept it.
  writeFileSync(resolve(bin, 'python3.EXE'), '')

  // Windows semantics, with the lowercase `Path` spelling that broke every rig.
  const winEnv = { Path: bin, PATHEXT: '.EXE;.CMD' }
  assert.equal(commandExists('cargo', winEnv, 'win32'), true)
  assert.equal(commandExists('python3', winEnv, 'win32'), false)
  assert.equal(commandExists('absent', winEnv, 'win32'), false)

  // POSIX: no extension list, and the same zero-byte rejection applies.
  writeFileSync(resolve(bin, 'ffprobe'), '#!/bin/sh\n')
  chmodSync(resolve(bin, 'ffprobe'), 0o755)
  writeFileSync(resolve(bin, 'stub'), '')
  const posixEnv = { PATH: bin }
  assert.equal(commandExists('ffprobe', posixEnv, 'linux'), true)
  assert.equal(commandExists('stub', posixEnv, 'linux'), false)
  writeFileSync(resolve(bin, 'not-runnable'), 'regular file')
  assert.equal(commandExists('not-runnable', posixEnv, 'linux'), false)
})

test('preflight rejects empty executable adapters and output paths that are not writable directories', () => {
  const dir = mkdtempSync(resolve(tmpdir(), 'cut-rig-preflight-'))
  const emptyAdapter = resolve(dir, 'ffmpeg')
  const clip = resolve(dir, 'source.mp4')
  const outputFile = resolve(dir, 'not-a-directory')
  writeFileSync(emptyAdapter, '')
  writeFileSync(clip, 'non-empty media fixture')
  writeFileSync(outputFile, 'not a directory')

  const adapter = runIgnoredTestRig({
    repoRoot,
    id: 'hw-encoder-size-gate',
    outDir: resolve(dir, 'adapter-receipt'),
    allowDirty: true,
    env: { ...process.env, SHELLX_CUT_FFMPEG: emptyAdapter },
  })
  assert.equal(adapter.exitCode, 2)
  assert.ok(adapter.receipt.preflight.missing.includes(`artifact:not-executable:${emptyAdapter}`))

  const output = runIgnoredTestRig({
    repoRoot,
    id: 'real-4k-software',
    outDir: resolve(dir, 'output-receipt'),
    allowDirty: true,
    env: {
      ...process.env,
      SHELLX_CUT_TEST_4K: clip,
      SHELLX_CUT_TEST_OUT_DIR: outputFile,
    },
  })
  assert.equal(output.exitCode, 2)
  assert.ok(output.receipt.preflight.missing.includes(`output-dir:not-directory:${outputFile}`))
})

test('CLI argument parser keeps dirty override explicit', () => {
  assert.deepEqual(parseIgnoredRigArgs(['--id', 'real-4k-gpu', '--out', '/tmp/rig']), {
    id: 'real-4k-gpu',
    outDir: '/tmp/rig',
    allowDirty: false,
    list: false,
  })
  assert.equal(parseIgnoredRigArgs(['--id', 'real-4k-gpu', '--allow-dirty']).allowDirty, true)
  assert.equal(parseIgnoredRigArgs(['--list']).list, true)
})

test('implicit rig receipts stay in the checked-out scratch tree', () => {
  assert.equal(
    resolveIgnoredTestRigReceiptDir({
      repoRoot: '/work/shellx-cut',
      id: 'real-4k-gpu',
      date: new Date('2026-07-08T10:11:12Z'),
    }),
    '/work/shellx-cut/.scratch/ignored-test-rigs/real-4k-gpu-2026-07-08T10-11-12-000Z',
  )
})
