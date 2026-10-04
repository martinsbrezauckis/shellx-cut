import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import test from 'node:test'

import {
  defaultProbe,
  PUBLISHED_TEST_INVENTORY_GUARD,
  loadPublishedTestInventory,
  resolvePublishedPrerequisites,
  verifyPublishedTestResources,
  verifyPublishedUiLibraryMembership,
} from '../lib/published-test-inventory.mjs'

const ROOT = resolve(import.meta.dirname, '../..')
const RUNNER = resolve(ROOT, 'scripts/check-published-test-inventory.mjs')

function fixture(t, { tests = ['scripts/public-tests/fixture.test.mjs'], resources = [], inventory } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'cut-published-test-inventory-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const defaultInventory = {
    schema: 'shellx-cut/published-test-inventory@1',
    version: '0.0.1',
    contract: { node_test_count: 1, ui_library_test_count: 1 },
    prerequisites: [{
      id: 'python',
      environment: 'CUTD_ADAPTER_PYTHON',
      candidates: ['python3', 'python'],
      probe: ['-c', 'import sys'],
    }],
    tests: [{ path: tests[0], resources: [tests[0], ...resources], requires: [] }],
    ui_library_tests: {
      root: 'ui/public-tests',
      runner: 'ui/public-tests/lib/runLibraryTests.mjs',
      discovery: 'ui/public-tests/lib/libraryTestDiscovery.mjs',
      exclusions: 'ui/public-tests/library-test-exclusions.json',
      tests: ['fixture.test.ts'],
    },
  }
  const selectedInventory = inventory ?? defaultInventory
  for (const path of [...tests, ...resources]) {
    mkdirSync(join(root, path, '..'), { recursive: true })
    writeFileSync(join(root, path), 'fixture\n')
  }
  mkdirSync(join(root, 'scripts/public-tests'), { recursive: true })
  writeFileSync(join(root, 'scripts/public-tests/published-inventory.json'), JSON.stringify(selectedInventory, null, 2))
  mkdirSync(join(root, 'ui/public-tests/lib'), { recursive: true })
  writeFileSync(join(root, 'ui/package.json'), JSON.stringify({ version: selectedInventory.version }))
  writeFileSync(join(root, 'ui/public-tests/lib/runLibraryTests.mjs'), 'fixture\n')
  writeFileSync(join(root, 'ui/public-tests/lib/libraryTestDiscovery.mjs'), 'fixture\n')
  writeFileSync(join(root, 'ui/public-tests/library-test-exclusions.json'), JSON.stringify({ schema: 'shellx-cut/library-test-exclusions@1', exclusions: [] }))
  for (const path of selectedInventory.ui_library_tests.tests) {
    const target = join(root, 'ui/public-tests', path)
    mkdirSync(join(target, '..'), { recursive: true })
    writeFileSync(target, 'fixture\n')
  }
  return root
}

test('published test inventory names the exact public CI scope and every declared resource', () => {
  const inventory = loadPublishedTestInventory({ repoRoot: ROOT })
  assert.equal(inventory.version, JSON.parse(readFileSync(resolve(ROOT, 'ui/package.json'), 'utf8')).version)
  assert.deepEqual(inventory.contract, { node_test_count: 11, ui_library_test_count: 39 })
  assert.deepEqual(inventory.tests.map((entry) => entry.path), [
    'scripts/public-tests/cross-host-media.test.mjs',
    'scripts/public-tests/dependency-audit.test.mjs',
    'scripts/public-tests/fit-to-fill-fixture.test.mjs',
    'scripts/public-tests/grok-judge-tool-policy.test.mjs',
    'scripts/public-tests/judge-adapter.test.mjs',
    'scripts/public-tests/judge-provider-admission.test.mjs',
    'scripts/public-tests/manual-publication-browser-receipt.test.mjs',
    'scripts/public-tests/manual-publication-package.test.mjs',
    'scripts/public-tests/published-test-inventory.test.mjs',
    'scripts/public-tests/safe-data.test.mjs',
    'scripts/public-tests/third-party-notice.test.mjs',
  ])
  verifyPublishedTestResources(inventory)
})

test('published test inventory requires the POSIX media sampler instead of accepting its skip', () => {
  const inventory = loadPublishedTestInventory({ repoRoot: ROOT })
  const judgeAdapter = inventory.tests.find((entry) => entry.path === 'scripts/public-tests/judge-adapter.test.mjs')
  assert.deepEqual(judgeAdapter.requires, ['python', 'ffmpeg', 'ffprobe'])
  assert.equal(judgeAdapter.resources.includes('scripts/lib/judge-resource-map.mjs'), true)
  assert.throws(
    () => resolvePublishedPrerequisites(inventory, { environment: {}, probe: (command) => ({ ok: command !== 'ffprobe' }) }),
    new RegExp(`${PUBLISHED_TEST_INVENTORY_GUARD}: required prerequisite ffprobe is unavailable`),
  )
})

test('published Judge resources reject an absent Windows validation helper', () => {
  const inventory = loadPublishedTestInventory({ repoRoot: ROOT })
  const helper = resolve(ROOT, 'scripts/public-tests/restricted-claude-windows-validation.py')
  verifyPublishedTestResources(inventory)
  assert.throws(
    () => verifyPublishedTestResources(inventory, (path) => path !== helper && existsSync(path)),
    new RegExp(`${PUBLISHED_TEST_INVENTORY_GUARD}: declared resource is missing: scripts/public-tests/restricted-claude-windows-validation\\.py`),
  )
})

test('published UI library membership is compared with the maintained runner discovery', async (t) => {
  const root = fixture(t, {
    tests: ['scripts/public-tests/fixture.test.mjs'],
    resources: [
      'ui/package.json',
      'ui/public-tests/lib/runLibraryTests.mjs',
      'ui/public-tests/lib/libraryTestDiscovery.mjs',
      'ui/public-tests/library-test-exclusions.json',
      'ui/public-tests/alpha.test.ts',
      'ui/public-tests/beta.test.ts',
    ],
    inventory: {
      schema: 'shellx-cut/published-test-inventory@1',
      version: '0.0.1',
      contract: { node_test_count: 1, ui_library_test_count: 2 },
      prerequisites: [{
        id: 'python',
        environment: 'CUTD_ADAPTER_PYTHON',
        candidates: ['python3'],
        probe: ['-c', 'import sys'],
      }],
      tests: [{ path: 'scripts/public-tests/fixture.test.mjs', resources: ['scripts/public-tests/fixture.test.mjs'], requires: [] }],
      ui_library_tests: {
        root: 'ui/public-tests',
        runner: 'ui/public-tests/lib/runLibraryTests.mjs',
        discovery: 'ui/public-tests/lib/libraryTestDiscovery.mjs',
        exclusions: 'ui/public-tests/library-test-exclusions.json',
        tests: ['alpha.test.ts', 'beta.test.ts'],
      },
    },
  })
  writeFileSync(join(root, 'ui/package.json'), JSON.stringify({ version: '0.0.1' }))
  writeFileSync(join(root, 'ui/public-tests/library-test-exclusions.json'), JSON.stringify({ schema: 'shellx-cut/library-test-exclusions@1', exclusions: [] }))
  const inventory = loadPublishedTestInventory({ repoRoot: root })
  verifyPublishedTestResources(inventory)
  await verifyPublishedUiLibraryMembership(inventory)
  await verifyPublishedUiLibraryMembership(inventory, async () => ({ included: ['alpha.test.ts', 'beta.test.ts'] }))
  await verifyPublishedUiLibraryMembership(inventory, async () => ({ included: ['alpha.test.ts', 'beta.test.ts', 'canonical-only.test.ts'] }))
  await assert.rejects(
    () => verifyPublishedUiLibraryMembership(inventory, async () => ({ included: ['alpha.test.ts'] })),
    new RegExp(`${PUBLISHED_TEST_INVENTORY_GUARD}: declared UI library test membership differs from maintained discovery`),
  )
})

test('published runner executes only the declared tests and collects every declared failure', (t) => {
  const first = 'scripts/public-tests/first.test.mjs'
  const second = 'scripts/public-tests/second.test.mjs'
  const undeclared = 'scripts/public-tests/undeclared.test.mjs'
  const inventory = {
    schema: 'shellx-cut/published-test-inventory@1',
    version: '0.0.1',
    contract: { node_test_count: 2, ui_library_test_count: 1 },
    prerequisites: [{
      id: 'python',
      environment: 'CUTD_ADAPTER_PYTHON',
      candidates: ['python3'],
      probe: ['-c', 'import sys'],
    }],
    tests: [
      { path: first, resources: [first], requires: [] },
      { path: second, resources: [second], requires: [] },
    ],
    ui_library_tests: {
      root: 'ui/public-tests',
      runner: 'ui/public-tests/lib/runLibraryTests.mjs',
      discovery: 'ui/public-tests/lib/libraryTestDiscovery.mjs',
      exclusions: 'ui/public-tests/library-test-exclusions.json',
      tests: ['fixture.test.ts'],
    },
  }
  const root = fixture(t, { tests: [first, second, undeclared], inventory })
  writeFileSync(join(root, first), "import assert from 'node:assert/strict'\nimport test from 'node:test'\ntest('fixture failure', () => assert.fail('fixture failure'))\n")
  writeFileSync(join(root, second), "import assert from 'node:assert/strict'\nimport test from 'node:test'\ntest('fixture success', () => assert.equal(true, true))\n")
  writeFileSync(join(root, undeclared), "throw new Error('the runner must not discover this undeclared test')\n")

  const environment = { ...process.env }
  delete environment.NODE_TEST_CONTEXT
  const result = spawnSync(process.execPath, [RUNNER, '--root', root, '--run', '--no-bail'], {
    encoding: 'utf8',
    env: environment,
  })
  const output = `${result.stdout}\n${result.stderr}`
  assert.equal(result.status, 1, output)
  assert.match(output, new RegExp(`${PUBLISHED_TEST_INVENTORY_GUARD}: RUN ${first}`))
  assert.match(output, new RegExp(`${PUBLISHED_TEST_INVENTORY_GUARD}: RUN ${second}`))
  assert.doesNotMatch(output, /undeclared\.test\.mjs/)
  assert.match(output, new RegExp(`${PUBLISHED_TEST_INVENTORY_GUARD}: 1 of 2 published tests failed`))
})

test('published test inventory refuses a missing declared resource', (t) => {
  const root = fixture(t)
  const inventory = loadPublishedTestInventory({ repoRoot: root })
  rmSync(join(root, 'scripts/public-tests/fixture.test.mjs'))
  assert.throws(
    () => verifyPublishedTestResources(inventory),
    new RegExp(`${PUBLISHED_TEST_INVENTORY_GUARD}: declared test is missing: scripts/public-tests/fixture\\.test\\.mjs`),
  )
})

test('published test inventory rejects Windows drive-prefixed paths', () => {
  assert.throws(
    () => loadPublishedTestInventory({ repoRoot: ROOT, inventoryPath: 'C:/published-inventory.json' }),
    new RegExp(`${PUBLISHED_TEST_INVENTORY_GUARD}: inventory path must be a safe relative path`),
  )
})

test('published prerequisite probes fail closed on a bounded timeout', () => {
  const result = defaultProbe(process.execPath, ['-e', 'setTimeout(() => {}, 1000)'], 20)
  assert.equal(result.ok, false)
  assert.equal(result.error?.code, 'ETIMEDOUT')
})

test('published test inventory refuses to turn an absent Python prerequisite into a successful skip', (t) => {
  const root = fixture(t, {
    inventory: {
      schema: 'shellx-cut/published-test-inventory@1',
      version: '0.0.1',
      contract: { node_test_count: 1, ui_library_test_count: 1 },
      prerequisites: [{
        id: 'python',
        environment: 'CUTD_ADAPTER_PYTHON',
        candidates: ['python3', 'python'],
        probe: ['-c', 'import sys'],
      }],
      tests: [{
        path: 'scripts/public-tests/fixture.test.mjs',
        resources: ['scripts/public-tests/fixture.test.mjs'],
        requires: ['python'],
      }],
      ui_library_tests: {
        root: 'ui/public-tests',
        runner: 'ui/public-tests/lib/runLibraryTests.mjs',
        discovery: 'ui/public-tests/lib/libraryTestDiscovery.mjs',
        exclusions: 'ui/public-tests/library-test-exclusions.json',
        tests: ['fixture.test.ts'],
      },
    },
  })
  const inventory = loadPublishedTestInventory({ repoRoot: root })
  assert.throws(
    () => resolvePublishedPrerequisites(inventory, { probe: () => ({ ok: false }) }),
    new RegExp(`${PUBLISHED_TEST_INVENTORY_GUARD}: required prerequisite python is unavailable`),
  )
})

test('published test inventory forwards a verified Python command through CUTD_ADAPTER_PYTHON', (t) => {
  const root = fixture(t, {
    inventory: {
      schema: 'shellx-cut/published-test-inventory@1',
      version: '0.0.1',
      contract: { node_test_count: 1, ui_library_test_count: 1 },
      prerequisites: [{
        id: 'python',
        environment: 'CUTD_ADAPTER_PYTHON',
        candidates: ['python3', 'python'],
        probe: ['-c', 'import sys'],
      }],
      tests: [{
        path: 'scripts/public-tests/fixture.test.mjs',
        resources: ['scripts/public-tests/fixture.test.mjs'],
        requires: ['python'],
      }],
      ui_library_tests: {
        root: 'ui/public-tests',
        runner: 'ui/public-tests/lib/runLibraryTests.mjs',
        discovery: 'ui/public-tests/lib/libraryTestDiscovery.mjs',
        exclusions: 'ui/public-tests/library-test-exclusions.json',
        tests: ['fixture.test.ts'],
      },
    },
  })
  const inventory = loadPublishedTestInventory({ repoRoot: root })
  const resolved = resolvePublishedPrerequisites(inventory, {
    environment: {},
    probe: (command) => ({ ok: command === 'python' }),
  })
  assert.equal(resolved.get('python').command, 'python')
})
