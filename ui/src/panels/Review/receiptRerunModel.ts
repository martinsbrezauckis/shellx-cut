// Pure identity and result validation for Review receipt output-check reruns.
// Keeping this separate from React makes stale-evidence rejection directly
// testable without mounting the Review panel or loading UI dependencies.

import type {
  OutputCheckRerun,
  OutputCheckRerunHandle,
  RenderReceipt,
} from '../../lib/client'

export type ReceiptRerunProfile = 'talking_head' | 'silent_screen_demo'

export interface ReceiptRerunIdentity {
  renderId: string
  outputHash: string
  outputPath: string
  atOp: string
  profile: ReceiptRerunProfile | null
}

export function profileFromReceipt(receipt: RenderReceipt): ReceiptRerunProfile | null {
  const entry = receipt.checks?.find((check) => check.name === 'footage_profile')
  if (!entry) return 'talking_head'
  if (!entry.details || typeof entry.details !== 'object') return null
  const value = Reflect.get(entry.details, 'active_profile')
  return value === 'talking_head' || value === 'silent_screen_demo' ? value : null
}

export function isRerunHandle(
  value: unknown,
  identity: ReceiptRerunIdentity,
): value is OutputCheckRerunHandle {
  if (!value || typeof value !== 'object') return false
  const handle = value as Partial<OutputCheckRerunHandle>
  return typeof handle.job_id === 'string' && handle.job_id.length > 0
    && handle.render_id === identity.renderId && handle.output_hash === identity.outputHash
}

const expectedCheckNames = new Set([
  'lufs',
  'black_or_frozen_frames',
  'uniform_border',
  'silence_at_edges',
  'duration_matches_receipt',
])

function isEvidenceObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

export function isRerunResult(
  value: unknown,
  identity: ReceiptRerunIdentity,
  jobId: string,
): value is OutputCheckRerun {
  if (!value || typeof value !== 'object' || !identity.profile) return false
  const result = value as Partial<OutputCheckRerun>
  if (result.scope !== 'rendered_output'
    || result.render_id !== identity.renderId
    || result.source_receipt_id !== identity.renderId
    || result.output_hash !== identity.outputHash
    || result.profile !== identity.profile
    || result.verification_receipt !== `receipts/verify_rerun_${jobId}.json`
    || typeof result.checked_at !== 'string' || result.checked_at.trim().length === 0
    || !Array.isArray(result.checks)
    || typeof result.pass !== 'boolean') return false

  if (result.checks.length !== expectedCheckNames.size) return false
  const seen = new Set<string>()
  for (const check of result.checks) {
    if (!check || typeof check !== 'object'
      || !expectedCheckNames.has(check.name) || seen.has(check.name)
      || typeof check.pass !== 'boolean'
      || !isEvidenceObject(check.details) || !isEvidenceObject(check.evidence)) return false
    seen.add(check.name)
  }
  return result.pass === result.checks.every((check) => check.pass)
}
