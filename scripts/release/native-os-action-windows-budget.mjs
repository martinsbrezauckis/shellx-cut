// The Windows helper owns its operation and failure-persistence deadlines.
// Its parent must allow the complete bounded child lifecycle to finish so a
// late native dialog cannot be accepted successfully and then reported as a
// false timeout before focus verification or failure evidence is durable.

export const WINDOWS_NATIVE_ACTION_PREFLIGHT_MS = 40_000
export const WINDOWS_NATIVE_ACTION_ACT_MS = 20_000
export const WINDOWS_NATIVE_ACTION_DISMISSAL_MS = 15_000
export const WINDOWS_NATIVE_ACTION_FOCUS_MS = 10_000
export const WINDOWS_NATIVE_ACTION_FAILURE_PERSISTENCE_MS = 60_000
export const WINDOWS_NATIVE_ACTION_PIPE_DRAIN_MS = 1_000
export const WINDOWS_NATIVE_ACTION_RESPONSE_DELIVERY_MS = 5_000

function finitePositiveMs(value, fallback) {
  const parsed = Number(value)
  return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback
}

export function windowsNativeActionOperationBudgetMs(actionTimeoutMs) {
  return finitePositiveMs(actionTimeoutMs, 20_000)
    + WINDOWS_NATIVE_ACTION_ACT_MS
    + WINDOWS_NATIVE_ACTION_DISMISSAL_MS
    + WINDOWS_NATIVE_ACTION_FOCUS_MS
}

export function windowsNativeActionControllerBudgetMs(actionTimeoutMs) {
  return WINDOWS_NATIVE_ACTION_PREFLIGHT_MS
    + windowsNativeActionOperationBudgetMs(actionTimeoutMs)
    + WINDOWS_NATIVE_ACTION_FAILURE_PERSISTENCE_MS
}

export function windowsNativeActionProofTimeoutMs(actionTimeoutMs) {
  return windowsNativeActionControllerBudgetMs(actionTimeoutMs)
    + WINDOWS_NATIVE_ACTION_PIPE_DRAIN_MS
}

export function windowsNativeActionResponseTimeoutMs(actionTimeoutMs) {
  return windowsNativeActionProofTimeoutMs(actionTimeoutMs)
    + WINDOWS_NATIVE_ACTION_RESPONSE_DELIVERY_MS
}

export function nativeActionResponseTimeoutMs(platform, enabled, actionTimeoutMs, fallbackMs = 12_000) {
  return enabled && (platform === 'windows' || platform === 'win32')
    ? windowsNativeActionResponseTimeoutMs(actionTimeoutMs)
    : fallbackMs
}

export function createWindowsActionDeadline({ run, actionId, now = () => Date.now() }) {
  let deadlineMs = 0
  return {
    run(command, args, timeout = 10_000) {
      if (!deadlineMs) return run(command, args, timeout)
      const remainingMs = deadlineMs - now()
      if (remainingMs <= 0) {
        throw new Error(`Windows native action exceeded its bounded deadline for ${actionId}`)
      }
      return run(command, args, Math.min(timeout, remainingMs))
    },
    start(durationMs) {
      deadlineMs = now() + durationMs
    },
    async within(durationMs, task) {
      const priorDeadlineMs = deadlineMs
      deadlineMs = now() + durationMs
      try {
        return await task()
      } finally {
        deadlineMs = priorDeadlineMs
      }
    },
  }
}
