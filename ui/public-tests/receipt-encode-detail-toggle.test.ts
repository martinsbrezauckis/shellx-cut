// The Render history & checks tab hides ffmpeg's own error line behind a "Technical detail"
// disclosure. This renders the real Render history & checks tab and checks what that
// disclosure holds, when it is offered, and that it starts closed.

import { strict as assert } from 'node:assert'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { renderReceiptFrom } from '../src/lib/events'
import { register } from 'node:module'
register('./lib/css-module-loader.mjs', import.meta.url)
const { default: Receipts } = await import('../src/panels/Review/Receipts')

const base = {
  render_id: 'render_001',
  ts: 'receipt-time',
  output_path: '/p/exports/render_001.mp4',
  output_hash: 'sha256:abc',
  duration_ms: 1000,
  preset: 'standard',
  at_op: 'op_000001',
  checks: [],
  pass: true,
}
const render = (encode: Record<string, unknown> | undefined) => {
  const receipt = renderReceiptFrom({ ...base, ...(encode ? { encode } : {}) })
  assert.ok(receipt)
  return renderToStaticMarkup(createElement(Receipts, { receipts: [receipt], onSeek: () => {} }))
}

// Hardware was asked for and software ran: the reason shows, and ffmpeg's line sits behind the toggle, closed.
const fallback = render({
  encoder: 'libx264',
  hardware_requested: true,
  hardware_used: false,
  fallback_reason: 'No usable hardware encoder was found, so this export used software encoding.',
  fallback_detail: 'h264_nvenc: Required: 13.1 Found: 13.0',
})
const disclosure = fallback.match(/<details class="rr-rc__encode-detail">(.*?)<\/details>/)
assert.ok(disclosure, 'the technical detail is a disclosure')
assert.doesNotMatch(disclosure[0], /<details[^>]* open/, 'it starts closed')
assert.match(disclosure[1], /^<summary data-cut-action="receipt-encode-detail-toggle">Technical detail<\/summary>/, 'the toggle is the first thing in it')
assert.match(disclosure[1], /<code>h264_nvenc: Required: 13\.1 Found: 13\.0<\/code>/, 'ffmpeg\'s line is inside, after the toggle')
assert.match(fallback, /No usable hardware encoder was found/, 'the plain reason stays visible outside the disclosure')
assert.doesNotMatch(fallback.replace(disclosure[0], ''), /h264_nvenc/, 'the raw line is only inside the disclosure')

// No fallback, or no recorded detail: nothing to toggle.
assert.doesNotMatch(render({ encoder: 'h264_nvenc', hardware_requested: true, hardware_used: true }), /receipt-encode-detail-toggle/)
assert.doesNotMatch(render({ encoder: 'libx264', hardware_requested: true, hardware_used: false, fallback_reason: 'No usable hardware encoder.' }), /receipt-encode-detail-toggle/)
assert.doesNotMatch(render(undefined), /receipt-encode-detail-toggle/)

console.log('PASS Receipts: Technical detail disclosure holds the encoder line, starts closed, and appears only with a recorded detail')
