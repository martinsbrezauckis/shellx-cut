export const INSTALLED_STARTUP_READINESS_SCHEMA = 'shellx-cut/installed-startup-readiness@1'

export const COLD_LAUNCH_BUDGET_IDS = Object.freeze([
  'listener',
  'api',
  'uiClient',
  'domRoot',
])

const CHECKPOINTS = Object.freeze([
  'shellSpawned',
  'listener',
  'api',
  'uiClient',
  'domRoot',
  'slowFfmpeg',
])

function assert(condition, message) {
  if (!condition) throw new Error(message)
}

function validEpochMs(value) {
  return Number.isSafeInteger(value) && value > 0
}

function checkpointIndex(id) {
  const index = CHECKPOINTS.indexOf(id)
  assert(index >= 0, `unknown startup readiness checkpoint: ${id}`)
  return index
}

export function normalizeColdLaunchBudgets(budgets = {}) {
  assert(budgets && typeof budgets === 'object' && !Array.isArray(budgets),
    'cold-launch budgets must be an object')
  const unknown = Object.keys(budgets).filter((id) => !COLD_LAUNCH_BUDGET_IDS.includes(id))
  assert(unknown.length === 0, `unknown cold-launch budget field(s): ${unknown.join(', ')}`)

  const ms = {}
  for (const id of COLD_LAUNCH_BUDGET_IDS) {
    const value = budgets[id]
    if (value === undefined || value === null || value === '') continue
    assert(Number.isSafeInteger(value) && value > 0,
      `cold-launch ${id} budget must be a positive integer milliseconds value`)
    ms[id] = value
  }
  const missing = COLD_LAUNCH_BUDGET_IDS.filter((id) => ms[id] === undefined)
  return {
    configured: missing.length === 0,
    missing,
    ms,
  }
}

function budgetResult(budgets, checkpoints) {
  if (!budgets.configured) {
    return {
      status: 'caller-budget-required',
      claimEligible: false,
      missing: budgets.missing,
      reason: 'Cold-launch timing is measured only until the caller supplies every explicit budget field.',
    }
  }

  const results = Object.fromEntries(COLD_LAUNCH_BUDGET_IDS.map((id) => {
    const checkpoint = checkpoints[id]
    return [id, {
      elapsedMs: checkpoint.elapsedMs,
      budgetMs: budgets.ms[id],
      ok: checkpoint.elapsedMs <= budgets.ms[id],
    }]
  }))
  const passed = Object.values(results).every((result) => result.ok)
  return {
    status: passed ? 'pass' : 'fail',
    claimEligible: passed,
    results,
  }
}

/**
 * Starts a measurement before the caller launches the native shell. This is
 * deliberately evidence-only: it never decides a product performance policy.
 */
export function beginInstalledStartupReadiness({
  surface,
  t0EpochMs = Date.now(),
  budgets = {},
} = {}) {
  assert(typeof surface === 'string' && surface.length > 0, 'startup readiness requires a surface')
  assert(validEpochMs(t0EpochMs), 'startup readiness t0 must be a positive integer epoch milliseconds value')
  const normalizedBudgets = normalizeColdLaunchBudgets(budgets)
  const checkpoints = {}
  let lastCheckpoint = -1

  function mark(id, { atEpochMs = Date.now(), evidence = {} } = {}) {
    const index = checkpointIndex(id)
    assert(!checkpoints[id], `startup readiness checkpoint '${id}' was already recorded`)
    assert(index >= lastCheckpoint, `startup readiness checkpoint '${id}' is out of order`)
    assert(validEpochMs(atEpochMs) && atEpochMs >= t0EpochMs,
      `startup readiness checkpoint '${id}' predates t0`)
    checkpoints[id] = {
      atEpochMs,
      elapsedMs: atEpochMs - t0EpochMs,
      evidence,
    }
    lastCheckpoint = index
    return checkpoints[id]
  }

  function build({ generatedAt = new Date().toISOString() } = {}) {
    for (const id of CHECKPOINTS.slice(0, 5)) {
      assert(checkpoints[id], `startup readiness is missing '${id}'`)
    }
    if (checkpoints.slowFfmpeg) {
      assert(checkpoints.uiClient.atEpochMs < checkpoints.slowFfmpeg.atEpochMs,
        'connected UI readiness did not precede the test-owned slow FFmpeg marker')
      assert(checkpoints.domRoot.atEpochMs < checkpoints.slowFfmpeg.atEpochMs,
        'DOM root readiness did not precede the test-owned slow FFmpeg marker')
    }
    return {
      schema: INSTALLED_STARTUP_READINESS_SCHEMA,
      generatedAt,
      status: 'measured',
      classification: 'measurement-only',
      surface,
      t0: { epochMs: t0EpochMs, meaning: 'recorded before native shell spawn' },
      checkpoints: structuredClone(checkpoints),
      ...(checkpoints.slowFfmpeg ? {
        slowFfmpeg: {
          testOwned: true,
          uiAndDomPrecedeMarker: true,
        },
      } : {}),
      budget: budgetResult(normalizedBudgets, checkpoints),
    }
  }

  return { mark, build, t0EpochMs, budgets: normalizedBudgets }
}

export function validateInstalledStartupReadiness(receipt, {
  surface,
  requireBudget = false,
} = {}) {
  assert(receipt?.schema === INSTALLED_STARTUP_READINESS_SCHEMA,
    'installed startup readiness receipt has an invalid schema')
  assert(receipt.status === 'measured' && receipt.classification === 'measurement-only',
    'installed startup readiness receipt must remain measurement-only')
  if (surface) assert(receipt.surface === surface, 'installed startup readiness surface mismatch')
  assert(validEpochMs(receipt.t0?.epochMs), 'installed startup readiness t0 is invalid')
  const checkpoints = receipt.checkpoints
  for (const id of CHECKPOINTS.slice(0, 5)) {
    const checkpoint = checkpoints?.[id]
    assert(validEpochMs(checkpoint?.atEpochMs) && Number.isSafeInteger(checkpoint?.elapsedMs)
      && checkpoint.elapsedMs >= 0, `installed startup readiness '${id}' checkpoint is invalid`)
  }
  assert(checkpoints.shellSpawned.atEpochMs >= receipt.t0.epochMs,
    'installed startup shell spawn predates t0')
  if (checkpoints.slowFfmpeg) {
    assert(receipt.slowFfmpeg?.testOwned === true && receipt.slowFfmpeg.uiAndDomPrecedeMarker === true,
      'installed startup slow FFmpeg marker is incomplete')
    assert(checkpoints.uiClient.atEpochMs < checkpoints.slowFfmpeg.atEpochMs
      && checkpoints.domRoot.atEpochMs < checkpoints.slowFfmpeg.atEpochMs,
    'installed startup UI/DOM readiness does not precede the slow FFmpeg marker')
  }
  const budget = receipt.budget
  assert(['caller-budget-required', 'pass', 'fail'].includes(budget?.status),
    'installed startup readiness budget status is invalid')
  if (requireBudget) {
    assert(budget.status !== 'caller-budget-required',
      'installed startup readiness requires caller-supplied cold-launch budgets')
  }
  return receipt
}
