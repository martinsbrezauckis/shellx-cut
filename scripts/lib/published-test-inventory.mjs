import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

import { discoverLibraryTests } from '../../ui/public-tests/lib/libraryTestDiscovery.mjs'

export const PUBLISHED_TEST_INVENTORY_GUARD = 'TEST-PUBLISHED-INVENTORY-01'
export const PUBLISHED_TEST_INVENTORY_SCHEMA = 'shellx-cut/published-test-inventory@1'
export const DEFAULT_PUBLISHED_INVENTORY = 'scripts/public-tests/published-inventory.json'
export const PUBLISHED_PREREQUISITE_PROBE_TIMEOUT_MS = 5_000

function fail(message) {
  throw new Error(`${PUBLISHED_TEST_INVENTORY_GUARD}: ${message}`)
}

function safeRelativePath(value, label) {
  if (typeof value !== 'string' || !value || value.startsWith('/') || /^[A-Za-z]:/.test(value)
    || value.includes('\\') || value.split('/').includes('..')) {
    fail(`${label} must be a safe relative path`)
  }
  return value
}

function readInventory(path) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'))
  } catch (error) {
    fail(`cannot read inventory ${path}: ${error.message}`)
  }
}

function validPrerequisite(value) {
  return value
    && typeof value === 'object'
    && !Array.isArray(value)
    && typeof value.id === 'string'
    && /^[a-z][a-z0-9_]*$/.test(value.id)
    && (value.environment === undefined || (typeof value.environment === 'string' && /^[A-Z][A-Z0-9_]*$/.test(value.environment)))
    && Array.isArray(value.candidates)
    && value.candidates.length > 0
    && value.candidates.every((candidate) => typeof candidate === 'string' && candidate.trim())
    && Array.isArray(value.probe)
    && value.probe.length > 0
    && value.probe.every((argument) => typeof argument === 'string')
}

function validCount(value) {
  return Number.isInteger(value) && value > 0
}

function readPackageVersion(root) {
  try {
    return JSON.parse(readFileSync(resolve(root, 'ui/package.json'), 'utf8')).version
  } catch (error) {
    fail(`cannot read ui/package.json: ${error.message}`)
  }
}

export function loadPublishedTestInventory({ repoRoot = '.', inventoryPath = DEFAULT_PUBLISHED_INVENTORY } = {}) {
  const root = resolve(repoRoot)
  const path = resolve(root, safeRelativePath(inventoryPath, 'inventory path'))
  const inventory = readInventory(path)
  if (inventory?.schema !== PUBLISHED_TEST_INVENTORY_SCHEMA) {
    fail(`inventory schema must be ${PUBLISHED_TEST_INVENTORY_SCHEMA}`)
  }
  if (typeof inventory.version !== 'string' || !/^\d+\.\d+\.\d+$/.test(inventory.version)) {
    fail('inventory version must be a release version')
  }
  if (!inventory.contract || typeof inventory.contract !== 'object' || Array.isArray(inventory.contract)
    || !validCount(inventory.contract.node_test_count) || !validCount(inventory.contract.ui_library_test_count)) {
    fail('inventory contract must declare positive node_test_count and ui_library_test_count')
  }
  if (!Array.isArray(inventory.prerequisites) || inventory.prerequisites.length === 0) {
    fail('inventory prerequisites must be a non-empty array')
  }
  if (!Array.isArray(inventory.tests) || inventory.tests.length === 0) {
    fail('inventory tests must be a non-empty array')
  }

  const prerequisites = new Map()
  for (const prerequisite of inventory.prerequisites) {
    if (!validPrerequisite(prerequisite)) fail('each prerequisite must declare id, environment, candidates, and probe')
    if (prerequisites.has(prerequisite.id)) fail(`duplicate prerequisite ${prerequisite.id}`)
    prerequisites.set(prerequisite.id, Object.freeze({
      id: prerequisite.id,
      environment: prerequisite.environment ?? null,
      candidates: [...prerequisite.candidates],
      probe: [...prerequisite.probe],
    }))
  }

  const tests = []
  const seenTests = new Set()
  const resources = new Set()
  for (const entry of inventory.tests) {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) fail('each test must be an object')
    const testPath = safeRelativePath(entry.path, 'test path')
    if (!testPath.startsWith('scripts/public-tests/') || !testPath.endsWith('.test.mjs')) {
      fail(`test path must be a public Node test: ${JSON.stringify(testPath)}`)
    }
    if (seenTests.has(testPath)) fail(`duplicate test ${testPath}`)
    seenTests.add(testPath)

    if (!Array.isArray(entry.resources) || entry.resources.length === 0) {
      fail(`${testPath} must declare a non-empty resources array`)
    }
    const testResources = []
    const seenResources = new Set()
    for (const resource of entry.resources) {
      const resourcePath = safeRelativePath(resource, `${testPath} resource`)
      if (seenResources.has(resourcePath)) fail(`${testPath} duplicates resource ${resourcePath}`)
      seenResources.add(resourcePath)
      resources.add(resourcePath)
      testResources.push(resourcePath)
    }

    if (!Array.isArray(entry.requires)) fail(`${testPath} must declare a requires array`)
    const requires = []
    const seenRequires = new Set()
    for (const id of entry.requires) {
      if (typeof id !== 'string' || !prerequisites.has(id)) fail(`${testPath} references unknown prerequisite ${JSON.stringify(id)}`)
      if (seenRequires.has(id)) fail(`${testPath} duplicates prerequisite ${id}`)
      seenRequires.add(id)
      requires.push(id)
    }
    tests.push(Object.freeze({ path: testPath, resources: testResources, requires }))
  }
  if (tests.length !== inventory.contract.node_test_count) {
    fail(`node_test_count ${inventory.contract.node_test_count} does not match ${tests.length} declared tests`)
  }

  const ui = inventory.ui_library_tests
  if (!ui || typeof ui !== 'object' || Array.isArray(ui)) fail('inventory must declare ui_library_tests')
  const uiRoot = safeRelativePath(ui.root, 'ui library root')
  const uiRunner = safeRelativePath(ui.runner, 'ui library runner')
  const uiDiscovery = safeRelativePath(ui.discovery, 'ui library discovery')
  const uiExclusions = safeRelativePath(ui.exclusions, 'ui library exclusions')
  if (uiRoot !== 'ui/public-tests' || uiRunner !== 'ui/public-tests/lib/runLibraryTests.mjs'
    || uiDiscovery !== 'ui/public-tests/lib/libraryTestDiscovery.mjs'
    || uiExclusions !== 'ui/public-tests/library-test-exclusions.json') {
    fail('ui_library_tests must bind the maintained public library test runner and discovery contract')
  }
  if (!Array.isArray(ui.tests) || ui.tests.length === 0) fail('ui_library_tests.tests must be a non-empty array')
  const uiTests = []
  const seenUiTests = new Set()
  for (const relativePath of ui.tests) {
    if (typeof relativePath !== 'string' || !relativePath.endsWith('.test.ts') || relativePath.startsWith('/') || relativePath.includes('\\') || relativePath.split('/').includes('..')) {
      fail(`invalid ui library test ${JSON.stringify(relativePath)}`)
    }
    if (seenUiTests.has(relativePath)) fail(`duplicate ui library test ${relativePath}`)
    seenUiTests.add(relativePath)
    uiTests.push(relativePath)
  }
  if (uiTests.length !== inventory.contract.ui_library_test_count) {
    fail(`ui_library_test_count ${inventory.contract.ui_library_test_count} does not match ${uiTests.length} declared UI tests`)
  }

  return Object.freeze({
    root,
    path,
    version: inventory.version,
    contract: Object.freeze({ ...inventory.contract }),
    prerequisites,
    tests,
    resources: [...resources].sort(),
    uiLibraryTests: Object.freeze({ root: uiRoot, runner: uiRunner, discovery: uiDiscovery, exclusions: uiExclusions, tests: uiTests }),
  })
}

export function verifyPublishedTestResources(inventory, exists = existsSync) {
  for (const test of inventory.tests) {
    const testPath = resolve(inventory.root, test.path)
    if (!exists(testPath)) fail(`declared test is missing: ${test.path}`)
  }
  for (const resource of inventory.resources) {
    const resourcePath = resolve(inventory.root, resource)
    if (!exists(resourcePath)) fail(`declared resource is missing: ${resource}`)
  }
  const ui = inventory.uiLibraryTests
  for (const resource of [ui.runner, ui.discovery, ui.exclusions, 'ui/package.json']) {
    if (!exists(resolve(inventory.root, resource))) fail(`declared UI library resource is missing: ${resource}`)
  }
  for (const relativePath of ui.tests) {
    if (!exists(resolve(inventory.root, ui.root, relativePath))) fail(`declared UI library test is missing: ${relativePath}`)
  }
  if (readPackageVersion(inventory.root) !== inventory.version) {
    fail(`inventory version ${inventory.version} does not match ui/package.json`)
  }
  return inventory
}

export async function verifyPublishedUiLibraryMembership(inventory, discover = discoverLibraryTests) {
  const ui = inventory.uiLibraryTests
  const discovered = await discover({
    publicTestsRoot: resolve(inventory.root, ui.root),
    exclusionsPath: resolve(inventory.root, ui.exclusions),
  })
  const discoveredTests = new Set(discovered.included)
  const missing = ui.tests.filter((path) => !discoveredTests.has(path))
  if (missing.length > 0) {
    fail(`declared UI library test membership differs from maintained discovery (missing ${missing.join(', ')})`)
  }
  return discovered
}

export function defaultProbe(command, args, timeout = PUBLISHED_PREREQUISITE_PROBE_TIMEOUT_MS) {
  const result = spawnSync(command, args, {
    encoding: 'utf8',
    timeout,
  })
  return { ok: result.status === 0 && !result.error, error: result.error, status: result.status }
}

export function resolvePublishedPrerequisites(inventory, {
  environment = process.env,
  probe = defaultProbe,
} = {}) {
  const required = new Set(inventory.tests.flatMap((test) => test.requires))
  const resolved = new Map()
  for (const id of required) {
    const prerequisite = inventory.prerequisites.get(id)
    const configured = prerequisite.environment ? environment[prerequisite.environment] : undefined
    const candidates = configured ? [configured] : prerequisite.candidates
    let selected
    for (const candidate of candidates) {
      const result = probe(candidate, prerequisite.probe)
      if (result?.ok) {
        selected = candidate
        break
      }
    }
    if (!selected) {
      const remediation = prerequisite.environment
        ? `set ${prerequisite.environment} or install one of ${prerequisite.candidates.join(', ')}`
        : `install one of ${prerequisite.candidates.join(', ')} on PATH`
      fail(`required prerequisite ${id} is unavailable; ${remediation}`)
    }
    resolved.set(id, Object.freeze({ ...prerequisite, command: selected }))
  }
  return resolved
}
