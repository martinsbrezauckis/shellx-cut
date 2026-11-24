// A receipt whose checks did not pass says so in words on the Render history & checks tab:
// which checks, what that means, the way forward (fix-list item 166). The
// engine writes the verdict into the receipt (`outcome`); this renders the
// real Render history & checks tab from a receipt as the engine writes it.

import { strict as assert } from 'node:assert'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { renderReceiptFrom } from '../src/lib/events'
import { register } from 'node:module'
register('./lib/css-module-loader.mjs', import.meta.url)
const { default: Receipts } = await import('../src/panels/Review/Receipts')

const SUMMARY = 'The export was written but did not pass its checks (duration_matches_edl, sources_decodable): The file is not as long as the timeline: part of the picture or sound is missing.'
const WAY = 'The file was written but is not a correct export of the timeline. Import a complete copy of the cut-off or damaged source (or replace it on the timeline), then export again.'
const base = {
  render_id: 'render_001', ts: 't', output_path: '/p/exports/render_001.mp4', output_hash: 'sha256:abc',
  duration_ms: 1000, preset: 'standard', at_op: 'op_000001',
  checks: [
    { name: 'duration_matches_edl', pass: false, details: {}, evidence: {} },
    { name: 'sources_decodable', pass: false, details: {}, evidence: {} },
  ],
  pass: false,
}
const render = (extra: Record<string, unknown>) => {
  const receipt = renderReceiptFrom({ ...base, ...extra })
  assert.ok(receipt)
  return renderToStaticMarkup(createElement(Receipts, { receipts: [receipt], onSeek: () => {} }))
}

const problems = render({ outcome: { verdict: 'problems', summary: SUMMARY, failed: [], action: WAY } })
assert.match(problems, /data-cut-receipt-outcome="render_001"/, problems)
assert.match(problems, /data-cut-receipt-verdict="problems"/)
assert.ok(problems.includes('did not pass its checks (duration_matches_edl, sources_decodable)'), 'the checks are named')
assert.ok(problems.includes('Import a complete copy of the cut-off or damaged source'), 'the way forward is shown')
assert.match(problems, /2 FAILED/, 'the banner still counts the failures')

const warnings = render({ checks: [{ name: 'lufs', pass: false, details: {}, evidence: {} }], outcome: { verdict: 'warnings', summary: 'The export was written; 1 quality check did not pass (lufs).', action: 'Open the receipt.' } })
assert.match(warnings, /data-cut-receipt-verdict="warnings"/)

assert.doesNotMatch(render({ checks: [], pass: true, outcome: { verdict: 'passed', summary: 'The export was written and passed its checks.' } }), /data-cut-receipt-outcome/, 'a pass adds no line')
assert.doesNotMatch(render({}), /data-cut-receipt-outcome/, 'an older receipt without a verdict renders as before')
assert.doesNotMatch(render({ outcome: { verdict: 'made_up', summary: 'x' } }), /data-cut-receipt-outcome/, 'an unknown verdict is not shown as one')

console.log('PASS Receipts: a receipt that did not pass its checks names them, their meaning and the way forward')
