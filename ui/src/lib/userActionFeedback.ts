import {
  callVerb,
  type VerbArgs,
  type VerbName,
  type VerbResult,
} from './client'

export const USER_ACTION_FEEDBACK_EVENT = 'cut:user-action-feedback'

export interface UserActionFeedbackDetail {
  message: string
  setupSurface?: 'settings-video-performance' | 'settings-ai-transcription'
}

export function publishUserActionMessage(message: string): void {
  if (typeof document === 'undefined' || !message.trim()) return
  document.dispatchEvent(new CustomEvent<UserActionFeedbackDetail>(USER_ACTION_FEEDBACK_EVENT, {
    detail: { message: message.trim() },
  }))
}

const SETUP_SURFACE: Partial<Record<VerbName, UserActionFeedbackDetail['setupSurface']>> = {
  'audio.cleanup_voice': 'settings-video-performance',
  'edit.redact': 'settings-ai-transcription',
  'edit.stabilize': 'settings-video-performance',
}

export function userVerbFailureMessage(result: VerbResult, fallback: string): string | null {
  if (result.ok) return null
  const message = result.error?.message?.trim() || fallback
  const action = result.error?.suggested_action?.trim()
  if (!action || message.toLocaleLowerCase().includes(action.toLocaleLowerCase())) return message
  return `${message} ${action}`
}

function setupSurfaceForFailure(name: VerbName, result: VerbResult): UserActionFeedbackDetail['setupSurface'] {
  const candidate = SETUP_SURFACE[name]
  if (!candidate) return undefined
  const detail = [
    result.error?.code,
    result.error?.message,
    result.error?.cause,
    result.error?.suggested_action,
  ].filter(Boolean).join(' ').toLocaleLowerCase()
  return /install|setup|unavailable|missing|ffmpeg|model|perception|runtime|capabilit/.test(detail)
    ? candidate
    : undefined
}

export function userActionFailureDetail<N extends VerbName>(
  name: N,
  result: VerbResult,
  fallback: string,
): UserActionFeedbackDetail | null {
  const message = userVerbFailureMessage(result, fallback)
  return message ? { message, setupSurface: setupSurfaceForFailure(name, result) } : null
}

export function publishUserActionFailure<N extends VerbName>(
  name: N,
  result: VerbResult,
  fallback: string,
): string | null {
  const detail = userActionFailureDetail(name, result, fallback)
  if (!detail || typeof document === 'undefined') return detail?.message ?? null
  document.dispatchEvent(new CustomEvent<UserActionFeedbackDetail>(USER_ACTION_FEEDBACK_EVENT, {
    detail,
  }))
  return detail.message
}

/**
 * Run a verb initiated by a human control and guarantee a visible failure.
 * Successful state still comes from the normal project/event refresh path.
 */
export async function runUserVerb<N extends VerbName>(
  name: N,
  args: VerbArgs[N],
  fallback: string,
) {
  try {
    const result = await callVerb(name, args)
    publishUserActionFailure(name, result, fallback)
    return result
  } catch {
    const base = fallback.trim().replace(/[.:;\s]+$/, '')
    const message = `${base}: the local engine is unreachable.`
    if (typeof document !== 'undefined') {
      document.dispatchEvent(new CustomEvent<UserActionFeedbackDetail>(USER_ACTION_FEEDBACK_EVENT, {
        detail: { message },
      }))
    }
    return null
  }
}
