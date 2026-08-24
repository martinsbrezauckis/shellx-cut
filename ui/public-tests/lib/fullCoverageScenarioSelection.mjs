// Narrow FCV_ONLY routing shared by lifecycle metadata contracts and the runner.

export const EXPORT_LIFECYCLE_SCENARIO_ID = 'e2e-export-lifecycle-01'
export const RELINK_DERIVATION_SCENARIO_ID = 'e2e-relink-derivation-01'

export function normalizeFullCoverageOnly(value) {
  return String(value || '').trim().toLowerCase()
}

export function matchesFullCoverageOnly(only, actionName) {
  const filter = normalizeFullCoverageOnly(only)
  return !filter || String(actionName || '').toLowerCase().includes(filter)
}

// A generic FCV_ONLY selector is useful only when it identifies and reaches at
// least one action probe. Without this fail-closed check, a spelling/casing
// drift can finish a selected section with only support rows and look green.
export function assessFullCoverageOnlyExecution({ only = '', declaredProbes = 0, executedProbes = 0 } = {}) {
  const filter = normalizeFullCoverageOnly(only)
  if (!filter) return Object.freeze({ ok: true, filter, reason: '' })
  if (declaredProbes === 0) {
    return Object.freeze({ ok: false, filter, reason: `FCV_ONLY=${JSON.stringify(String(only))} matched zero declared action probes` })
  }
  if (executedProbes === 0) {
    return Object.freeze({ ok: false, filter, reason: `FCV_ONLY=${JSON.stringify(String(only))} declared ${declaredProbes} action probe(s) but executed zero` })
  }
  return Object.freeze({ ok: true, filter, reason: '' })
}

export function selectedLifecycleScenario(only) {
  const normalized = normalizeFullCoverageOnly(only)
  if (normalized === EXPORT_LIFECYCLE_SCENARIO_ID) return EXPORT_LIFECYCLE_SCENARIO_ID
  if (normalized === RELINK_DERIVATION_SCENARIO_ID) return RELINK_DERIVATION_SCENARIO_ID
  return ''
}
