#!/usr/bin/env node
import { strict as assert } from 'node:assert'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { buildRuntimeActionReceiptUnion } from '../lib/runtime-action-receipt-union.mjs'

const root = mkdtempSync(join(tmpdir(), 'shellx-cut-runtime-action-union-'))
process.on('exit', () => rmSync(root, { recursive: true }))
const manifest = join(root, 'manifest.json')
const first = join(root, 'first.json')
const second = join(root, 'second.json')

writeFileSync(manifest, JSON.stringify({
  schema: 'shellx-cut/ui-source-action-manifest@1',
  actionCount: 3,
  actions: ['alpha', 'beta', 'gamma'],
}))
writeFileSync(first, JSON.stringify({
  ok: true,
  generatedAt: '2026-07-29T00:00:00.000Z',
  runtimeSourceActionManifest: { observed: ['beta', 'alpha', 'alpha'] },
}))
writeFileSync(second, JSON.stringify({
  ok: false,
  runtimeSourceActionManifest: { observed: ['gamma', 'beta'] },
}))

const complete = buildRuntimeActionReceiptUnion({
  receiptFiles: [first, second],
  manifestFile: manifest,
  generatedAt: '2026-07-29T01:00:00.000Z',
})
assert.equal(complete.schema, 'shellx-cut/runtime-action-receipt-union@1')
assert.equal(complete.ok, true)
assert.deepEqual(complete.runtimeSourceActionManifest.observed, ['alpha', 'beta', 'gamma'])
assert.deepEqual(complete.runtimeSourceActionManifest.missing, [])
assert.deepEqual(complete.runtimeSourceActionManifest.unexpected, [])
assert.equal(complete.inputs[0].actionTotal, 2)
assert.equal(complete.inputs[1].receiptOk, false)

const incomplete = buildRuntimeActionReceiptUnion({
  receiptFiles: [first],
  manifestFile: manifest,
})
assert.equal(incomplete.ok, false)
assert.deepEqual(incomplete.runtimeSourceActionManifest.missing, ['gamma'])

console.log('runtime action receipt union tests passed')
