#!/usr/bin/env node
import { resolve } from 'node:path'

import {
  UI_DIST_IDENTITY_GUARD,
  checkUiDistIdentity,
  writeUiDistIdentity,
} from './lib/ui-dist-identity.mjs'

const args = process.argv.slice(2)
let root = process.cwd()
let distPath
let mode
for (let index = 0; index < args.length; index += 1) {
  const arg = args[index]
  if (arg === '--check' || arg === '--write') {
    if (mode) {
      process.stderr.write('usage: node scripts/check-ui-dist-identity.mjs --check|--write [--root <path>] [--dist <path>]\n')
      process.exitCode = 2
      break
    }
    mode = arg
  } else if (arg === '--root' || arg === '--dist') {
    const value = args[++index]
    if (!value) {
      process.stderr.write('usage: node scripts/check-ui-dist-identity.mjs --check|--write [--root <path>] [--dist <path>]\n')
      process.exitCode = 2
      break
    }
    if (arg === '--root') root = resolve(value)
    else distPath = resolve(value)
  } else {
    process.stderr.write('usage: node scripts/check-ui-dist-identity.mjs --check|--write [--root <path>] [--dist <path>]\n')
    process.exitCode = 2
    break
  }
}

if (!mode && !process.exitCode) {
  process.stderr.write('usage: node scripts/check-ui-dist-identity.mjs --check|--write [--root <path>] [--dist <path>]\n')
  process.exitCode = 2
}
if (!process.exitCode) {
  try {
    const identity = mode === '--write'
      ? writeUiDistIdentity({ repoRoot: root, distPath })
      : checkUiDistIdentity({ repoRoot: root, distPath })
    process.stdout.write(`PASS ${UI_DIST_IDENTITY_GUARD} ${mode === '--write' ? 'wrote' : 'verified'} ${identity.version}\n`)
  } catch (error) {
    process.stderr.write(`${error.message}\n`)
    process.exitCode = 1
  }
}
