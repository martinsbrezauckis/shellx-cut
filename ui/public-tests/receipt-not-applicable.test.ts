// A check with nothing to check on this export (details.status
// "not_applicable", e.g. caption_presence on an export without captions) is
// neither a pass nor a failure nor "unmeasured". The Render history & checks tab shows it as
// N/A with its reason and counts it apart; the status-bar chip does too.

import { strict as assert } from 'node:assert'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { notApplicable } from '../src/lib/checkApplicability'
import { renderReceiptFrom } from '../src/lib/events'
import { register } from 'node:module'
register('./lib/css-module-loader.mjs', import.meta.url)
const { default: Receipts } = await import('../src/panels/Review/Receipts')
import { receiptSummary } from '../src/statusbar/receiptSummary'

const na = (name: string, reason: string, applies_when: string) => ({
  name, pass: true, details: { status: 'not_applicable', reason, applies_when }, evidence: {},
})
const pass = (name: string) => ({ name, pass: true, details: {}, evidence: {} })
const receipt = renderReceiptFrom({
  render_id: 'render_001', ts: 't', output_path: '/p/exports/render_001.mp4', output_hash: 'sha256:abc',
  duration_ms: 4000, preset: 'standard', at_op: 'op_000001', pass: true,
  checks: [
    na('caption_presence', 'This export has no captions.', 'the export burns in captions (caption clips on a visible caption track)'),
    na('cut_on_word', 'No clip on the timeline has a transcript, so cuts cannot be compared with words.', 'a clip on the timeline has a transcript'),
    pass('duration_matches_edl'),
    pass('lufs'),
    { name: 'footage_profile', pass: true, details: { active_profile: 'talking_head', selection: 'explicit', proposed_profile: 'silent_screen_demo' }, evidence: {} },
  ],
  outcome: { verdict: 'passed', summary: 'The export was written and passed its checks.', not_applicable: ['cut_on_word', 'caption_presence'] },
})
assert.ok(receipt)
assert.deepEqual(receipt.outcome?.not_applicable, ['cut_on_word', 'caption_presence'])
assert.equal(notApplicable(receipt.checks[0])?.reason, 'This export has no captions.')
assert.equal(notApplicable(receipt.checks[2]), null, 'a measured pass is not N/A')

const html = renderToStaticMarkup(createElement(Receipts, { receipts: [receipt], onSeek: () => {} }))
assert.match(html, />2\/2 PASS · 2 N\/A</, 'the banner counts only checks that apply, N/A apart')
assert.match(html, /data-cut-check="caption_presence" data-cut-check-state="na"/, html)
assert.match(html, /rr-check__state--na">N\/A</, 'the row reads N/A, never PASS')
assert.match(html, /title="Not applicable: This export has no captions."/, 'the reason is on the row')
assert.match(html, /data-cut-check="lufs" data-cut-check-state="pass"/)
assert.doesNotMatch(html, /data-cut-receipt-outcome/, 'no problem line on a clean export')

assert.match(html, /data-cut-profile="talking_head"/, 'the serialized profile value stays exact')
assert.match(html, /Content type for quality checks: Talking head/)
assert.match(html, /Auto-detect proposes Screen demo/)

assert.equal(receiptSummary(receipt).text, '4.0s · 2 checks pass · 2 not applicable')
assert.equal(receiptSummary(receipt).tone, 'pass')

console.log('PASS Receipts and status bar: a check that does not apply reads N/A with its reason, never a pass or a failure')
