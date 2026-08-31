import { accessSync, constants, existsSync, lstatSync, readFileSync, readdirSync } from 'node:fs'
import { relative, resolve, sep } from 'node:path'

export const PUBLIC_TEST_INVENTORY_SCHEMA = 'shellx-cut/public-test-inventory@1'
export const PUBLIC_TEST_INVENTORY_GUARD = 'TEST-PUBLIC-INVENTORY-01'

const CLASSES = new Set(['source', 'python', 'requires-exact-cutd'])

function inventoryError(message) {
  throw new Error(`${PUBLIC_TEST_INVENTORY_GUARD}: ${message}`)
}

function allPublicTests(root, directory = resolve(root, 'scripts/public-tests')) {
  const files = []
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((left, right) => left.name.localeCompare(right.name))) {
    const path = resolve(directory, entry.name)
    if (entry.isDirectory()) files.push(...allPublicTests(root, path))
    if (entry.isFile() && (entry.name.endsWith('.test.mjs') || entry.name === 'judge-adapter-validation.py')) {
      files.push(relative(root, path).split(sep).join('/'))
    }
  }
  return files.sort((left, right) => left.localeCompare(right))
}

function readInventory(path) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'))
  } catch (error) {
    inventoryError(`cannot read inventory ${path}: ${error.message}`)
  }
}

export function checkPublicTestInventory({ repoRoot, inventoryPath } = {}) {
  const root = resolve(repoRoot || '.')
  const path = resolve(inventoryPath || root, 'scripts/public-tests/inventory.json')
  const inventory = readInventory(path)
  if (inventory.schema !== PUBLIC_TEST_INVENTORY_SCHEMA) {
    inventoryError(`inventory schema must be ${PUBLIC_TEST_INVENTORY_SCHEMA}`)
  }
  if (!Array.isArray(inventory.tests) || inventory.tests.length === 0) {
    inventoryError('inventory tests must be a non-empty array')
  }

  const declared = new Map()
  for (const entry of inventory.tests) {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) inventoryError('each inventory entry must be an object')
    const { path: candidate, class: classification } = entry
    if (typeof candidate !== 'string' || !candidate.startsWith('scripts/public-tests/') || candidate.includes('..')) {
      inventoryError(`invalid public-test path ${JSON.stringify(candidate)}`)
    }
    if (declared.has(candidate)) inventoryError(`duplicate inventory entry: ${candidate}`)
    if (!CLASSES.has(classification)) inventoryError(`${candidate} has unsupported class ${JSON.stringify(classification)}`)
    if (!Object.hasOwn(entry, 'ci')) inventoryError(`${candidate} must declare an explicit ci disposition`)
    const ci = entry.ci
    if (typeof ci !== 'boolean') inventoryError(`${candidate} ci must be boolean`)
    if (ci === false) {
      const nonCi = entry.non_ci
      if (!nonCi || typeof nonCi !== 'object' || Array.isArray(nonCi)
        || typeof nonCi.reason !== 'string' || !nonCi.reason.trim()
        || typeof nonCi.owner !== 'string' || !nonCi.owner.trim()) {
        inventoryError(`${candidate} non-CI disposition requires non_ci.reason and non_ci.owner`)
      }
    }
    if (classification === 'requires-exact-cutd') {
      if (ci !== true) inventoryError(`${candidate} requires-exact-cutd coverage must be enabled in CI`)
      if (!Array.isArray(entry.requires) || !entry.requires.includes('CUTD_BIN')) {
        inventoryError(`${candidate} requires-exact-cutd entry must declare CUTD_BIN`)
      }
    }
    if (classification === 'python' && candidate.endsWith('.py') === false) {
      inventoryError(`${candidate} python classification requires a Python test`)
    }
    if (classification !== 'python' && !candidate.endsWith('.test.mjs')) {
      inventoryError(`${candidate} ${classification} classification requires a .test.mjs test`)
    }
    declared.set(candidate, entry)
  }

  const discovered = allPublicTests(root)
  for (const candidate of discovered) {
    if (!declared.has(candidate)) inventoryError(`unclassified public test: ${candidate}`)
  }
  for (const candidate of declared.keys()) {
    if (!discovered.includes(candidate)) inventoryError(`inventory references missing public test: ${candidate}`)
  }

  const ci = [...declared.values()].filter((entry) => entry.ci)
  if (ci.length === 0) inventoryError('inventory must select at least one CI public test')
  return { root, path, tests: [...declared.values()], ci }
}

export function requireExactCutd(entry, env = process.env) {
  const cutd = env.CUTD_BIN
  if (!cutd) {
    inventoryError(`${entry.path} requires an exact current cutd via CUTD_BIN; build it before this class (for example: cargo build --manifest-path app/Cargo.toml -p server --bin cutd)`)
  }
  if (!existsSync(cutd)) inventoryError(`${entry.path} CUTD_BIN does not exist: ${cutd}`)
  try {
    accessSync(cutd, constants.X_OK)
  } catch {
    inventoryError(`${entry.path} CUTD_BIN is not executable: ${cutd}`)
  }
  if (!lstatSync(cutd).isFile()) inventoryError(`${entry.path} CUTD_BIN must be a regular executable file: ${cutd}`)
  return cutd
}

export function selectedPublicTests(inventory, { profile = 'source' } = {}) {
  if (!['source', 'ci', 'requires-exact-cutd', 'all'].includes(profile)) inventoryError(`unsupported run profile ${JSON.stringify(profile)}`)
  if (profile === 'ci') return inventory.ci
  if (profile === 'all') return inventory.tests
  if (profile === 'requires-exact-cutd') return inventory.tests.filter((entry) => entry.class === 'requires-exact-cutd')
  return inventory.tests.filter((entry) => entry.class === 'source')
}
