#!/usr/bin/env node
import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'

import {
  PUBLISHED_TEST_INVENTORY_GUARD,
  loadPublishedTestInventory,
  resolvePublishedPrerequisites,
  verifyPublishedTestResources,
  verifyPublishedNodeTestMembership,
  verifyPublishedUiLibraryMembership,
} from './lib/published-test-inventory.mjs'

function usage() {
  process.stderr.write('usage: node scripts/check-published-test-inventory.mjs [--run] [--no-bail|--collect-all] [--allow-unlisted] [--root <path>] [--inventory <path>]\n')
}

const args = process.argv.slice(2)
let root = process.cwd()
let inventoryPath
let run = false
let noBail = false
let exact = true
for (let index = 0; index < args.length; index += 1) {
  const argument = args[index]
  if (argument === '--run') {
    run = true
  } else if (argument === '--allow-unlisted') {
    exact = false
  } else if (argument === '--no-bail' || argument === '--collect-all') {
    noBail = true
  } else if (argument === '--root' || argument === '--inventory') {
    const value = args[++index]
    if (!value) {
      usage()
      process.exitCode = 2
      break
    }
    if (argument === '--root') root = resolve(value)
    else inventoryPath = value
  } else {
    usage()
    process.exitCode = 2
    break
  }
}

if (!process.exitCode) {
  try {
    const inventory = loadPublishedTestInventory({ repoRoot: root, inventoryPath })
    verifyPublishedTestResources(inventory)
    // The manifest is the positive public projection. Declared tests must exist and be discovered, and the check is
    // exact the other way: a public test (Node or UI library) that the inventory does not list fails. A checkout
    // where other tests share these folders passes --allow-unlisted and compares with its own export policy.
    await verifyPublishedUiLibraryMembership(inventory, undefined, { exact })
    verifyPublishedNodeTestMembership(inventory, { exact })
    if (!run) {
      process.stdout.write(`PASS ${PUBLISHED_TEST_INVENTORY_GUARD} (${inventory.version}; ${inventory.tests.length} node tests, ${inventory.uiLibraryTests.tests.length} UI library tests)\n`)
    } else {
      const prerequisites = resolvePublishedPrerequisites(inventory)
      const environment = { ...process.env }
      for (const prerequisite of prerequisites.values()) {
        if (prerequisite.environment) environment[prerequisite.environment] = prerequisite.command
      }

      const failures = []
      for (const entry of inventory.tests) {
        process.stdout.write(`${PUBLISHED_TEST_INVENTORY_GUARD}: RUN ${entry.path}\n`)
        const result = spawnSync(process.execPath, ['--test', resolve(inventory.root, entry.path)], {
          cwd: inventory.root,
          env: environment,
          stdio: 'inherit',
        })
        const message = result.error?.message
          ?? (result.signal ? `terminated by ${result.signal}` : null)
          ?? (result.status !== 0 ? `exited with status ${result.status ?? 1}` : null)
        if (message) {
          failures.push({ path: entry.path, message })
          process.stderr.write(`${PUBLISHED_TEST_INVENTORY_GUARD}: FAIL ${entry.path}: ${message}\n`)
          if (!noBail) break
        }
      }
      if (failures.length > 0) {
        process.stderr.write(`${PUBLISHED_TEST_INVENTORY_GUARD}: ${failures.length} of ${inventory.tests.length} published tests failed\n`)
        for (const failure of failures) process.stderr.write(`- ${failure.path}: ${failure.message}\n`)
        process.exitCode = 1
      } else {
        process.stdout.write(`PASS ${PUBLISHED_TEST_INVENTORY_GUARD} ran ${inventory.tests.length} published node tests and verified ${inventory.uiLibraryTests.tests.length} UI library tests${noBail ? ' (collect-all)' : ''}\n`)
      }
    }
  } catch (error) {
    process.stderr.write(`${error.message}\n`)
    process.exitCode = 1
  }
}
