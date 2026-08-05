#!/usr/bin/env node

import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { createExactSourceReceipt, parseExactSourceArgs } from '../lib/exact-source-receipt.mjs'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const usage = [
  'usage: node scripts/release/exact-source-receipt.mjs',
  '  --surface <linux-control|windows-installed|macos-installed>',
  '  --capability <id> [--capability <id> ...]',
  '  --artifact <name=path> | --artifact-tree <name=path>',
  '  --evidence <name=passing-receipt.json> [--evidence ...]',
  '  --out <private-or-.shellx-scratch-path>',
].join('\n')

try {
  const args = parseExactSourceArgs(process.argv.slice(2))
  if (args.help) {
    console.log(usage)
    process.exit(0)
  }
  const result = createExactSourceReceipt({ repoRoot, ...args })
  console.log(`PASS exact-source receipt: ${result.receipt.surface}`)
  console.log(`source: ${result.receipt.source.gitCommit} version ${result.receipt.source.version}`)
  console.log(`capabilities: ${result.receipt.capabilities.join(', ')}`)
  console.log(`receipt: ${result.outPath}`)
} catch (error) {
  console.error(String(error.message || error).replace(/[\r\n\t\0]/g, ' '))
  console.error(usage)
  process.exit(2)
}
