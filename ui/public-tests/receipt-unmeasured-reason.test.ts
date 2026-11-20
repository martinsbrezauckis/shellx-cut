import assert from 'node:assert/strict'
import type { RenderReceipt } from '../src/lib/client'
import { receiptSummary } from '../src/statusbar/receiptSummary'

function receipt(reason: string | null): RenderReceipt {
  const unmeasured = (name: string) => ({
    name,
    pass: false,
    details: {
      status: 'unmeasured',
      measured: false,
      ...(reason ? { reason } : {}),
    },
    evidence: null,
  })
  return {
    render_id: 'render_001',
    output_path: '/tmp/render_001.mp4',
    output_hash: 'sha256:abc',
    duration_ms: 4000,
    preset: 'draft',
    pass: false,
    checks: [unmeasured('lufs'), unmeasured('uniform_border')],
    fix_actions: [],
  } as unknown as RenderReceipt
}

const missing = receiptSummary(receipt('not measured, because captions and transcription are not installed'))
assert.equal(missing.tone, 'unmeasured', 'a render that could not be measured is not a failed render')
assert.equal(missing.text, '4.0s · 2 unmeasured (captions and transcription are not installed)')

const noReason = receiptSummary(receipt(null))
assert.equal(noReason.text, '4.0s · 2 unmeasured')
console.log('receipt summary: unmeasured checks carry the engine reason')
