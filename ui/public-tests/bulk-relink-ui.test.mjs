import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const source = readFileSync(resolve(import.meta.dirname, '../src/panels/Assets/BulkRelinkPanel.tsx'), 'utf8')
const assets = readFileSync(resolve(import.meta.dirname, '../src/panels/Assets/index.tsx'), 'utf8')

assert.match(source, /media\.relink_preview/)
assert.match(source, /media\.relink_apply/)
assert.match(source, /PortableB5RelinkReceipt/, 'B5 UI retains the full durable receipt shape for B6 handoff')
assert.match(source, /expected_revision: preview\.project_revision/)
assert.match(source, /request_id: request/)
assert.match(source, /scopeKey: string/, 'B5 preview is scoped to an App project session plus revision')
assert.match(source, /activeScope\.current !== scopeKey/, 'late B5 preview/apply results are ignored after a scope change')
assert.match(source, /data-cut-media-relink-bulk/)
assert.match(source, /data-cut-media-relink-bulk-open/)
assert.match(source, /data-cut-media-relink-accept/)
assert.match(source, /data-cut-media-relink-apply/)
assert.match(source, /data-cut-media-relink-cancel/)
assert.match(source, /Possible replacement — review individually/)
assert.match(source, /data-cut-media-relink-review/)
assert.match(source, /onReviewIndividually/)
assert.match(source, /disabled=\{!selectable/)
assert.doesNotMatch(source, /metadata_only/)
assert.match(assets, /<BulkRelinkPanel/)
assert.match(assets, /onReviewIndividually=\{\(assetId\) => \{ void relinkAsset\(assetId\) \}\}/)

console.log('PASS B5 bulk relink UI wires preview, selective acceptance, cancellation, and revision-guarded apply')
