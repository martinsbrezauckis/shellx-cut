import assert from 'node:assert/strict'
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import test from 'node:test'

import {
  PUBLIC_TEST_INVENTORY_GUARD,
  checkPublicTestInventory,
  requireExactCutd,
  selectedPublicTests,
} from '../lib/public-test-inventory.mjs'

function fixture(t, tests, entries) {
  const root = mkdtempSync(join(tmpdir(), 'cut-public-test-inventory-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  mkdirSync(join(root, 'scripts/public-tests'), { recursive: true })
  for (const path of tests) writeFileSync(join(root, path), 'import test from "node:test"\ntest("fixture", () => {})\n')
  writeFileSync(join(root, 'scripts/public-tests/inventory.json'), JSON.stringify({
    schema: 'shellx-cut/public-test-inventory@1',
    tests: entries,
  }, null, 2))
  return root
}

test('public-test inventory classifies every public test and reserves CI coverage for exact-cutd tests', () => {
  const inventory = checkPublicTestInventory({ repoRoot: resolve(import.meta.dirname, '../..') })
  assert.equal(inventory.tests.length >= 79, true)
  assert.equal(inventory.ci.length, inventory.tests.length, 'every current public test is intentionally exercised in CI')
  const agent = inventory.tests.find((entry) => entry.path.endsWith('agent-chat-containment.test.mjs'))
  assert.deepEqual(agent, {
    path: 'scripts/public-tests/agent-chat-containment.test.mjs',
    class: 'requires-exact-cutd',
    ci: true,
    requires: ['CUTD_BIN'],
  })
  assert.equal(selectedPublicTests(inventory, { profile: 'source' }).some((entry) => entry === agent), false)
  assert.equal(selectedPublicTests(inventory, { profile: 'ci' }).some((entry) => entry === agent), true)
})

test('public-test inventory rejects an implicit CI disposition with an exact diagnostic', (t) => {
  const root = fixture(t,
    ['scripts/public-tests/implicit.test.mjs'],
    [{ path: 'scripts/public-tests/implicit.test.mjs', class: 'source' }],
  )
  assert.throws(
    () => checkPublicTestInventory({ repoRoot: root }),
    new RegExp(`${PUBLIC_TEST_INVENTORY_GUARD}: scripts/public-tests/implicit\\.test\\.mjs must declare an explicit ci disposition`),
  )
})

test('public-test inventory rejects an unclassified file with an exact diagnostic', (t) => {
  const root = fixture(t,
    ['scripts/public-tests/classified.test.mjs', 'scripts/public-tests/unclassified.test.mjs'],
    [{ path: 'scripts/public-tests/classified.test.mjs', class: 'source', ci: true }],
  )
  assert.throws(
    () => checkPublicTestInventory({ repoRoot: root }),
    new RegExp(`${PUBLIC_TEST_INVENTORY_GUARD}: unclassified public test: scripts/public-tests/unclassified\\.test\\.mjs`),
  )
})

test('exact-cutd tests fail before launch when their intentional prerequisite is absent', (t) => {
  const root = fixture(t,
    ['scripts/public-tests/cutd.test.mjs'],
    [{ path: 'scripts/public-tests/cutd.test.mjs', class: 'requires-exact-cutd', ci: true, requires: ['CUTD_BIN'] }],
  )
  const [entry] = checkPublicTestInventory({ repoRoot: root }).tests
  assert.throws(
    () => requireExactCutd(entry, {}),
    new RegExp(`${PUBLIC_TEST_INVENTORY_GUARD}: scripts/public-tests/cutd\\.test\\.mjs requires an exact current cutd via CUTD_BIN`),
  )
  const cutd = join(root, 'cutd-fixture')
  writeFileSync(cutd, '#!/usr/bin/env sh\nexit 0\n')
  chmodSync(cutd, 0o755)
  assert.equal(requireExactCutd(entry, { CUTD_BIN: cutd }), cutd)
})
