import {
  callVerb,
  type ControlledVerbArgs,
  type VerbName,
  type VerbResult,
} from './client'

import { captionsFailureText } from './jobFailureText'

export const USER_ACTION_FEEDBACK_EVENT = 'cut:user-action-feedback'

export interface UserActionFeedbackDetail {
  message: string
  /** The Settings panel (a ui.open surface id) the notice's Open setup button opens. */
  setupSurface?: string
  /** For `ui.state.notices`: the verb whose result is shown, or `drop` / `app`. */
  source?: string
  /** The refusal or warning code behind the message, when there is one. */
  code?: string | null
  /** How many dropped items were not imported (`source: "drop"`). */
  count?: number
}

/** Where an app-side message comes from, so an agent can tell what it is about. */
export interface UserActionMessageOrigin {
  source: string
  code?: string | null
  count?: number
}

export function publishUserActionMessage(message: string, origin?: UserActionMessageOrigin): void {
  if (typeof document === 'undefined' || !message.trim()) return
  document.dispatchEvent(new CustomEvent<UserActionFeedbackDetail>(USER_ACTION_FEEDBACK_EVENT, {
    detail: { message: message.trim(), ...(origin ?? {}) },
  }))
}

const SETUP_SURFACE: Partial<Record<VerbName, UserActionFeedbackDetail['setupSurface']>> = {
  'audio.cleanup_voice': 'settings-video-performance',
  'audio.dub': 'settings-services-integrations',
  'edit.redact': 'settings-ai-transcription',
  'edit.stabilize': 'settings-video-performance',
  'media.diarize': 'settings-services-integrations',
}

/** The engine's code for a step refused because ffmpeg or ffprobe is not installed. */
export const TOOL_MISSING = 'tool_missing'

/** Plain words for a person when video processing is missing. The engine's
 *  suggested_action is written for agents, so it is not shown here; the
 *  notice's Open setup button leads to the platform's steps instead. */
export function missingVideoToolsMessage(result: VerbResult): string | null {
  if (result.ok || result.error?.code !== TOOL_MISSING) return null
  const what = result.error?.message?.trim() || 'Video processing is not installed'
  // A whole tool missing blocks all media work; a missing filter (e.g. no
  // vid.stab for Stabilize) needs a more complete build.
  return /is not installed$/.test(what)
    ? `${what}. Set up video processing to import, preview and export.`
    : `${what}. Open setup to get an FFmpeg that has it.`
}

export function userVerbFailureMessage(result: VerbResult, fallback: string): string | null {
  if (result.ok) return null
  const captions = captionsFailureText(result.error)
  if (captions) return captions
  const missing = missingVideoToolsMessage(result)
  if (missing) return missing
  const message = result.error?.message?.trim() || fallback
  const action = result.error?.suggested_action?.trim()
  if (!action || message.toLocaleLowerCase().includes(action.toLocaleLowerCase())) return message
  return `${message} ${action}`
}

function setupSurfaceForFailure(name: VerbName, result: VerbResult): UserActionFeedbackDetail['setupSurface'] {
  // A refusal that names its missing tool or condition says where to get it
  // (schema/requirements.json); only Settings panels get the button.
  const named = result.ok ? undefined : result.error?.requirement?.get?.settings
  if (named?.startsWith('settings-')) return named
  // Any verb refused for a missing ffmpeg/ffprobe leads to the same setup.
  if (!result.ok && result.error?.code === TOOL_MISSING) return 'settings-video-performance'
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
    detail: { ...detail, source: name, code: result.error?.code ?? null },
  }))
  return detail.message
}

/**
 * Run a verb initiated by a human control and guarantee a visible failure.
 * Successful state still comes from the normal project/event refresh path.
 */
export async function runUserVerb<N extends VerbName>(
  name: N,
  args: ControlledVerbArgs<N>,
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
        detail: { message, source: name, code: 'engine_unreachable' },
      }))
    }
    return null
  }
}
