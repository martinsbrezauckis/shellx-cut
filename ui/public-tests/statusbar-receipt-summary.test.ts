import { strict as assert } from 'node:assert'
import type { RenderReceipt } from '../src/lib/client'
import { receiptSummary } from '../src/statusbar/receiptSummary'

const baseReceipt = (overrides: Partial<RenderReceipt> = {}): RenderReceipt => ({
  render_id: 'render_001',
  ts: '2026-09-21T19:12:00Z',
  output_path: '/project/exports/render_001.mp4',
  output_hash: 'sha256:abc',
  duration_ms: 13_300,
  preset: 'draft',
  at_op: 'op_001',
  pass: true,
  checks: [
    { name: 'lufs', pass: true, details: {}, evidence: {} },
    { name: 'caption_presence', pass: true, details: {}, evidence: {} },
  ],
  ...overrides,
})

assert.deepEqual(
  receiptSummary(baseReceipt()),
  { text: '13.3s · all checks pass', tone: 'pass', waived: 0, toFix: 0 },
  'a fully measured aggregate pass stays green',
)

assert.deepEqual(
  receiptSummary(baseReceipt({
    checks: [
      { name: 'cut_on_word', pass: true, details: {}, evidence: {} },
      { name: 'black_or_frozen_frames', pass: true, details: {}, evidence: {} },
      { name: 'uniform_border', pass: true, details: {}, evidence: {} },
      { name: 'duration_matches_edl', pass: true, details: {}, evidence: {} },
      { name: 'lufs', pass: true, details: { waived_by_profile: 'silent_screen_demo', measured_pass: false }, evidence: {} },
      { name: 'caption_presence', pass: true, details: { waived_by_profile: 'silent_screen_demo', measured_pass: true }, evidence: {} },
      { name: 'silence_at_edges', pass: true, details: { waived_by_profile: 'silent_screen_demo', measured_pass: false }, evidence: {} },
      { name: 'footage_profile', pass: true, details: { active_profile: 'silent_screen_demo' }, evidence: {} },
    ],
  })),
  { text: '13.3s · 4/7 PASS · 3 waived', tone: 'waived', waived: 3, toFix: 0 },
  'a profile-waived aggregate pass stays scope-explicit and amber',
)

assert.deepEqual(
  receiptSummary(baseReceipt({
    pass: false,
    checks: [
      { name: 'lufs', pass: false, details: { integrated_lufs: -9.2 }, evidence: {} },
      { name: 'caption_presence', pass: true, details: {}, evidence: {} },
    ],
  })),
  { text: '13.3s · 1 failed', tone: 'fail', waived: 0, toFix: 1 },
  'a failed measured check remains red and actionable',
)

console.log('PASS status-bar receipt summary distinguishes measured pass, profile waiver, and failure')
