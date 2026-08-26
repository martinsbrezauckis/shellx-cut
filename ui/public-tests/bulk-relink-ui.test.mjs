import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const source = readFileSync(resolve(import.meta.dirname, '../src/panels/Assets/BulkRelinkPanel.tsx'), 'utf8')
const assets = readFileSync(resolve(import.meta.dirname, '../src/panels/Assets/index.tsx'), 'utf8')

assert.match(source, /media\.relink_preview/)
assert.match(source, /media\.relink_apply/)
assert.match(source, /expected_revision: preview\.project_revision/)
assert.match(source, /request_id: request/)
assert.match(source, /data-cut-media-relink-bulk/)
assert.match(source, /data-cut-media-relink-bulk-open/)
assert.match(source, /data-cut-media-relink-accept/)
assert.match(source, /data-cut-media-relink-apply/)
assert.match(source, /data-cut-media-relink-cancel/)
assert.match(source, /metadata-only rows stay refused/)
assert.match(assets, /<BulkRelinkPanel/)

console.log('PASS B5 bulk relink UI wires preview, selective acceptance, cancellation, and revision-guarded apply')
