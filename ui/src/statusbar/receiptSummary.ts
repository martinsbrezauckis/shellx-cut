import { isNotApplicable } from '../lib/checkApplicability'
import type { CheckResult, RenderReceipt } from '../lib/client'

export type ReceiptSummaryTone = 'pass' | 'fail' | 'unmeasured' | 'waived'

export interface ReceiptSummary {
  text: string
  tone: ReceiptSummaryTone
  waived: number
  toFix: number
}

function isUnmeasured(check: CheckResult): boolean {
  const details = check.details
  return details !== null && typeof details === 'object'
    && (Reflect.get(details, 'status') === 'unmeasured' || Reflect.get(details, 'measured') === false)
}

const NOT_MEASURED_PREFIX = 'not measured, because '

/** The engine's plain reason the output checks were not measured, without its
 * "not measured, because" lead-in (for example "captions and transcription are
 * not installed"). Null when no unmeasured check carries one. */
export function unmeasuredReason(checks: readonly CheckResult[]): string | null {
  for (const check of checks) {
    if (!isUnmeasured(check)) continue
    const reason = Reflect.get(check.details as object, 'reason')
    if (typeof reason !== 'string' || !reason.trim()) continue
    return reason.startsWith(NOT_MEASURED_PREFIX) ? reason.slice(NOT_MEASURED_PREFIX.length) : reason
  }
  return null
}

function isWaived(check: CheckResult): boolean {
  const details = check.details
  return details !== null && typeof details === 'object'
    && typeof Reflect.get(details, 'waived_by_profile') === 'string'
}

/** Summarize the receipt truthfully without changing its engine-owned aggregate.
 * Profile waivers retain a passing aggregate but must not look like every
 * check measured a pass. */
export function receiptSummary(receipt: RenderReceipt): ReceiptSummary {
  const duration = (receipt.duration_ms / 1000).toFixed(1)
  // Checks that do not apply to this export are neither passes nor failures.
  const all = receipt.checks.filter((check) => check.name !== 'footage_profile')
  const checks = all.filter((check) => !isNotApplicable(check))
  const notApplicable = all.length - checks.length
  const unmeasured = checks.filter(isUnmeasured).length
  const failing = checks.filter((check) => !check.pass && !isUnmeasured(check)).length
  const waived = checks.filter(isWaived).length
  const fixes = receipt.fix_actions
  const toFix = failing > 0 && Array.isArray(fixes) && fixes.length > 0
    ? fixes.length
    : failing

  if (failing > 0) {
    const detail = Array.isArray(fixes) && fixes.length > 0
      ? `${fixes.length} to fix`
      : `${failing} failed${unmeasured > 0 ? ` · ${unmeasured} unmeasured` : ''}`
    return { text: `${duration}s · ${detail}`, tone: 'fail', waived, toFix }
  }
  if (unmeasured > 0) {
    const reason = unmeasuredReason(checks)
    return {
      text: `${duration}s · ${unmeasured} unmeasured${reason ? ` (${reason})` : ''}`,
      tone: 'unmeasured',
      waived,
      toFix,
    }
  }
  if (waived > 0) {
    const measured = checks.length - waived
    const detail = measured === 0
      ? `${waived} waived`
      : `${measured}/${checks.length} PASS · ${waived} waived`
    return { text: `${duration}s · ${detail}`, tone: 'waived', waived, toFix }
  }
  if (receipt.pass) {
    const text = notApplicable > 0
      ? `${duration}s · ${checks.length} checks pass · ${notApplicable} not applicable`
      : `${duration}s · all checks pass`
    return { text, tone: 'pass', waived, toFix }
  }
  return { text: `${duration}s · needs review`, tone: 'fail', waived, toFix }
}
