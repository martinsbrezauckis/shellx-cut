#!/usr/bin/env node
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { buildRuntimeActionReceiptUnion } from '../lib/runtime-action-receipt-union.mjs'

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '../..')

function usage() {
  return [
    'Usage:',
    '  node scripts/release/merge-runtime-action-receipts.mjs [options] <results.json>...',
    '',
    'Options:',
    '  --manifest <path>  expected source-action manifest',
    '  --out <path>       write the union receipt to this path',
  ].join('\n')
}

function parseArgs(argv) {
  const parsed = {
    manifest: resolve(REPO, 'ui/public-tests/full-ui-action-manifest.json'),
    out: '',
    receipts: [],
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--help' || arg === '-h') return { ...parsed, help: true }
    if (arg === '--manifest' || arg === '--out') {
      const value = argv[index + 1]
      if (!value) throw new Error(`${arg} requires a path`)
      parsed[arg.slice(2)] = resolve(value)
      index += 1
      continue
    }
    if (arg.startsWith('-')) throw new Error(`unknown option: ${arg}`)
    parsed.receipts.push(resolve(arg))
  }
  return parsed
}

export function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv)
  if (args.help) {
    console.log(usage())
    return 0
  }
  const union = buildRuntimeActionReceiptUnion({
    receiptFiles: args.receipts,
    manifestFile: args.manifest,
  })
  const serialized = `${JSON.stringify(union, null, 2)}\n`
  if (args.out) {
    mkdirSync(dirname(args.out), { recursive: true })
    writeFileSync(args.out, serialized, 'utf8')
    console.log(`runtime action union receipt → ${args.out}`)
  } else {
    process.stdout.write(serialized)
  }
  const status = union.ok ? 'PASS' : 'FAIL'
  console.error(
    `[runtime-action-union] ${status}: observed=${union.runtimeSourceActionManifest.total}` +
    ` expected=${union.runtimeSourceActionManifest.expectedTotal}` +
    ` missing=${union.runtimeSourceActionManifest.missing.length}` +
    ` unexpected=${union.runtimeSourceActionManifest.unexpected.length}`,
  )
  return union.ok ? 0 : 1
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    process.exitCode = main()
  } catch (error) {
    console.error(`[runtime-action-union] ERROR: ${error.message}`)
    process.exitCode = 2
  }
}
