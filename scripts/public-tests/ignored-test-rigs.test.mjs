import test from 'node:test'
import assert from 'node:assert/strict'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  cargoTestBinary,
  collectSourceIdentity,
  discoverIgnoredRustTests,
  loadIgnoredTestManifest,
  manifestPlatform,
  parseIgnoredRigArgs,
  rigExecutionEnv,
} from '../lib/ignored-test-rig.mjs'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const manifest = loadIgnoredTestManifest(repoRoot)

test('every ignored Rust test has exactly one rig classification', () => {
  const discovered = discoverIgnoredRustTests(repoRoot)
  const declared = manifest.tests
    .map(({ rustTest, source }) => ({ rustTest, source }))
    .sort((a, b) => a.rustTest.localeCompare(b.rustTest))
  assert.deepEqual(discovered, declared)
  assert.equal(discovered.length, 9)
  assert.equal(new Set(manifest.tests.map((entry) => entry.id)).size, manifest.tests.length)
})

test('rig definitions carry executable receipt contracts', () => {
  const classifications = new Set([
    'perception_runtime',
    'real_media',
    'hardware',
    'permission',
    'authenticated_service',
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

test('native platform ids match the manifest vocabulary', () => {
  assert.equal(manifestPlatform('linux'), 'linux')
  assert.equal(manifestPlatform('darwin'), 'macos')
  assert.equal(manifestPlatform('win32'), 'windows')
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

test('perception rigs default to the checked-out sidecar without overriding callers', () => {
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
  assert.equal(baseFallback.env.SHELLX_CUT_SIDECAR_DIR, resolve(repoRoot, 'app/perception/py'))
  assert.equal(baseFallback.env.SHELLX_CUT_PYTHON, undefined)
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
