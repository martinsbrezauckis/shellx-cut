// A receipt check that does not apply to this export (the engine wrote
// `details.status: "not_applicable"`, e.g. caption_presence on an export with
// no captions). It is neither a pass nor a failure nor "unmeasured": surfaces
// count it apart and show it as N/A with the engine's reason.

import type { CheckResult } from './client'

export interface NotApplicable {
  reason: string
  appliesWhen: string
}

/** The not-applicable facts of a check, or null when the check applies. */
export function notApplicable(check: CheckResult): NotApplicable | null {
  const details = check.details
  if (details === null || typeof details !== 'object') return null
  if (Reflect.get(details, 'status') !== 'not_applicable') return null
  const text = (name: string) => {
    const value = Reflect.get(details, name)
    return typeof value === 'string' ? value : ''
  }
  return { reason: text('reason'), appliesWhen: text('applies_when') }
}

export function isNotApplicable(check: CheckResult): boolean {
  return notApplicable(check) !== null
}
