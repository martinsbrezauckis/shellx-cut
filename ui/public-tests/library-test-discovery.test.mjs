import assert from 'node:assert/strict'
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { discoverLibraryTests } from './lib/libraryTestDiscovery.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const uiRoot = resolve(here, '..')
const packageJson = JSON.parse(await readFile(resolve(uiRoot, 'package.json'), 'utf8'))
assert.equal(packageJson.scripts['test:lib'], 'node public-tests/lib/runLibraryTests.mjs')

const current = await discoverLibraryTests()
for (const required of [
  'receipt-rerun-control.test.ts',
  'record-cursor-correlation.test.ts',
  'source-navigation.test.ts',
]) {
  assert.ok(current.included.includes(required), `${required} must be discovered by the canonical library suite`)
}
assert.deepEqual(current.excluded, [], 'public tests need no workflow-specific exclusions')

// Discovery is a rule, not a list: every test source under this folder is found, nothing outside it is.
async function testSources(directory, prefix = '') {
  const found = []
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (entry.isDirectory()) found.push(...await testSources(join(directory, entry.name), `${prefix}${entry.name}/`))
    else if (entry.isFile() && /[.]test[.](?:ts|mjs|js)$/.test(entry.name)) found.push(`${prefix}${entry.name}`)
  }
  return found
}
assert.deepEqual(
  [...current.included].sort(),
  (await testSources(here)).sort(),
  'library test discovery equals the test sources found under the public tests folder',
)

const scratch = await mkdtemp(join(tmpdir(), 'shellx-cut-library-discovery-'))
try {
  const tests = join(scratch, 'public-tests')
  await mkdir(join(tests, 'nested'), { recursive: true })
  await writeFile(join(tests, 'new-contract.test.ts'), 'console.log("new")\n')
  await writeFile(join(tests, 'nested', 'new-helper.test.mjs'), 'console.log("nested")\n')
  const registry = join(tests, 'library-test-exclusions.json')
  await writeFile(registry, JSON.stringify({ schema: 'shellx-cut/library-test-exclusions@1', exclusions: [] }))
  const discovered = await discoverLibraryTests({ publicTestsRoot: tests, exclusionsPath: registry })
  assert.deepEqual(discovered.included, ['nested/new-helper.test.mjs', 'new-contract.test.ts'])

  await writeFile(registry, JSON.stringify({
    schema: 'shellx-cut/library-test-exclusions@1',
    exclusions: [{ path: 'missing.test.ts', reason: 'This stale exclusion must fail because it names no discovered test.', ownerCommand: 'node missing.test.ts' }],
  }))
  await assert.rejects(
    discoverLibraryTests({ publicTestsRoot: tests, exclusionsPath: registry }),
    /does not name a discovered test/,
  )
} finally {
  await rm(scratch, { recursive: true, force: true })
}

console.log('PASS automatic library-test discovery and exclusion registry')
