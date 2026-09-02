#!/usr/bin/env node
import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'

import {
  PUBLIC_TEST_INVENTORY_GUARD,
  checkPublicTestInventory,
  requireExactCutd,
  selectedPublicTests,
} from './lib/public-test-inventory.mjs'

function usage() {
  process.stderr.write('usage: node scripts/check-public-test-inventory.mjs [--ci] [--run] [--no-bail|--collect-all] [--profile source|ci|requires-exact-cutd|all] [--root <path>] [--inventory <path>]\n')
}

const args = process.argv.slice(2)
let root = process.cwd()
let inventoryPath
let run = false
let noBail = false
let profile = 'source'
for (let index = 0; index < args.length; index += 1) {
  const arg = args[index]
  if (arg === '--ci') {
    profile = 'ci'
  } else if (arg === '--run') {
    run = true
  } else if (arg === '--no-bail' || arg === '--collect-all') {
    noBail = true
  } else if (arg === '--profile' || arg === '--root' || arg === '--inventory') {
    const value = args[++index]
    if (!value) {
      usage()
      process.exitCode = 2
      break
    }
    if (arg === '--profile') profile = value
    if (arg === '--root') root = resolve(value)
    if (arg === '--inventory') inventoryPath = resolve(value)
  } else {
    usage()
    process.exitCode = 2
    break
  }
}

if (!process.exitCode) {
  try {
    const inventory = checkPublicTestInventory({ repoRoot: root, inventoryPath })
    const selected = selectedPublicTests(inventory, { profile })
    const skippedExactCutd = profile === 'source'
      ? inventory.tests.filter((entry) => entry.class === 'requires-exact-cutd')
      : []
    for (const entry of skippedExactCutd) {
      process.stdout.write(`${PUBLIC_TEST_INVENTORY_GUARD}: SKIP ${entry.path} (requires exact current cutd via CUTD_BIN; run --profile requires-exact-cutd after building it)\n`)
    }
    if (!run) {
      process.stdout.write(`PASS ${PUBLIC_TEST_INVENTORY_GUARD} (${inventory.tests.length} classified public tests; ${inventory.ci.length} selected for CI)\n`)
    } else {
      const failures = []
      for (const entry of selected) {
        try {
          if (entry.class === 'requires-exact-cutd') requireExactCutd(entry)
          const command = entry.class === 'python' ? 'python3' : process.execPath
          const commandArgs = entry.class === 'python'
            ? [resolve(inventory.root, entry.path)]
            : ['--test', resolve(inventory.root, entry.path)]
          process.stdout.write(`${PUBLIC_TEST_INVENTORY_GUARD}: RUN ${entry.path}\n`)
          const result = spawnSync(command, commandArgs, { cwd: inventory.root, env: process.env, stdio: 'inherit' })
          if (result.error) throw result.error
          if (result.signal) throw new Error(`${entry.path} terminated by ${result.signal}`)
          if (result.status !== 0) throw new Error(`${entry.path} exited with status ${result.status ?? 1}`)
        } catch (error) {
          failures.push({ path: entry.path, message: error.message })
          process.stderr.write(`${PUBLIC_TEST_INVENTORY_GUARD}: FAIL ${entry.path}: ${error.message}\n`)
          if (!noBail) break
        }
      }
      if (failures.length > 0) {
        process.stderr.write(`${PUBLIC_TEST_INVENTORY_GUARD}: ${failures.length} of ${selected.length} ${profile} tests failed\n`)
        for (const failure of failures) process.stderr.write(`- ${failure.path}: ${failure.message}\n`)
        process.exitCode = 1
      } else {
        process.stdout.write(`PASS ${PUBLIC_TEST_INVENTORY_GUARD} ran ${selected.length} ${profile} tests${noBail ? ' (no-bail)' : ''}\n`)
      }
    }
  } catch (error) {
    process.stderr.write(`${error.message}\n`)
    process.exitCode = 1
  }
}
