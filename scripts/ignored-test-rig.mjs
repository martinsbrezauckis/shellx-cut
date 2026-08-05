#!/usr/bin/env node

import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  loadIgnoredTestManifest,
  parseIgnoredRigArgs,
  runIgnoredTestRig,
} from './lib/ignored-test-rig.mjs'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const oneLine = (value) => String(value).replace(/[\r\n\t\0]/g, ' ')

let args
try {
  args = parseIgnoredRigArgs(process.argv.slice(2))
} catch (error) {
  process.stderr.write(`${oneLine(error.message)}\n`)
  process.exit(2)
}

const manifest = loadIgnoredTestManifest(repoRoot)
if (args.list) {
  for (const rig of manifest.tests) {
    console.log(`${rig.id}\t${rig.classification}\t${rig.rustTest}`)
  }
  process.exit(0)
}
if (!args.id) {
  console.error('usage: node scripts/ignored-test-rig.mjs --id <rig-id> [--out <receipt-dir>] [--allow-dirty]')
  process.exit(2)
}

const result = runIgnoredTestRig({
  repoRoot,
  id: args.id,
  outDir: args.outDir,
  allowDirty: args.allowDirty,
})
console.log(`${result.receipt.pass ? 'PASS' : 'FAIL'} ${args.id}`)
console.log(`receipt: ${result.receiptPath}`)
if (result.receipt.error) console.error(result.receipt.error)
process.exit(result.exitCode)
