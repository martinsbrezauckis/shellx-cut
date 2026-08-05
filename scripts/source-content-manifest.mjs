#!/usr/bin/env node
import { writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { sourceContentManifest } from './lib/source-content-manifest.mjs'

const repoRoot = resolve(fileURLToPath(new URL('..', import.meta.url)))
const outIndex = process.argv.indexOf('--out')
const out = outIndex >= 0 ? process.argv[outIndex + 1] : ''
const manifest = sourceContentManifest(repoRoot)
const json = `${JSON.stringify(manifest, null, 2)}\n`
if (out) writeFileSync(resolve(out), json, { encoding: 'utf8', flag: 'wx' })
if (process.argv.includes('--sha256')) process.stdout.write(`${manifest.sha256}\n`)
else if (!out) process.stdout.write(json)
