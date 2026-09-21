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
  const checks = receipt.checks.filter((check) => check.name !== 'footage_profile')
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
  if (unmeasured > 0) return { text: `${duration}s · ${unmeasured} unmeasured`, tone: 'unmeasured', waived, toFix }
  if (waived > 0) {
    const measured = checks.length - waived
    const detail = measured === 0
      ? `${waived} waived`
      : `${measured}/${checks.length} PASS · ${waived} waived`
    return { text: `${duration}s · ${detail}`, tone: 'waived', waived, toFix }
  }
  if (receipt.pass) return { text: `${duration}s · all checks pass`, tone: 'pass', waived, toFix }
  return { text: `${duration}s · needs review`, tone: 'fail', waived, toFix }
}
