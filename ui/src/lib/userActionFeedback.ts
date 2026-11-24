import {
  callVerb,
  type ControlledVerbArgs,
  type VerbName,
  type VerbResult,
} from './client'

import { captionsFailureText, dedupeSentences, formatSentence, formatSentenceStart } from './jobFailureText'
import { VERB_NAMES } from './generatedVerbBehavior'

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
  /** When true, stays shown until explicitly dismissed rather than timing out. */
  persistent?: boolean
}

/** Where an app-side message comes from, so an agent can tell what it is about. */
export interface UserActionMessageOrigin {
  source: string
  code?: string | null
  count?: number
  persistent?: boolean
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

const VERB_NAME_SET = new Set(VERB_NAMES)

/** True when text contains engine verb names, JSON arguments, or command calls meant for agents. */
export function isAgentInstruction(text: string | null | undefined): boolean {
  const trimmed = text?.trim() || ''
  if (!trimmed) return false
  const tokens = trimmed.match(/[a-z0-9_]+(?:\.[a-z0-9_]+)+/gi) ?? []
  return tokens.some(token => VERB_NAME_SET.has(token.toLowerCase()))
    || /\{\s*["']|\b(?:call|run)\s+[a-z0-9_]+\.[a-z0-9_]+/i.test(trimmed)
}

/** Do not put engine instructions or command-shaped diagnostics into the interface. */
export function humanFailureReason(message: string | null | undefined, fallback: string): string {
  const text = message?.trim() || ''
  const reason = !text || isAgentInstruction(text) ? fallback : text
  return formatSentenceStart(reason)
}

function humanCauseSentence(
  cause: string | null | undefined,
  reason: string,
  action: string,
): string | null {
  const trimmed = cause?.trim() || ''
  if (!trimmed || isAgentInstruction(trimmed)) return null
  // Bare identifiers (e.g. track ids like "a1") and raw filesystem paths do
  // not add plain explanation for a person.
  if (!/\s/.test(trimmed) || /^(?:\/|\\\\|[a-z]:[\\/])/i.test(trimmed)) return null
  const needle = trimmed.replace(/[.:;\s]+$/, '').toLocaleLowerCase()
  if (!needle) return null
  const context = `${reason} ${action}`.toLocaleLowerCase()
  if (context.includes(needle)) return null
  return formatSentence(trimmed)
}

const GENERIC_CODE_ACTIONS: Record<string, string> = {
  tool_missing: 'Open video processing setup, then try again.',
  no_project: 'Open a project, then try again.',
  not_found: 'Check your selection and try again.',
  invalid_args: 'Check your choices and try again.',
  conflict: 'Review the current project and try again.',
  io: 'Check the file or folder and try again.',
  ffmpeg: 'Check video processing setup and try again.',
  sidecar: 'Check the relevant tools in Settings and try again.',
  capability_missing: 'Open the relevant setup in Settings, then try again.',
  unreadable_media: 'Choose a readable copy of the media and try again.',
  odd_project_size: 'Choose an even project size in project settings, then try again.',
  not_enough_space: 'Free space on the destination drive, then try again.',
  out_of_memory: 'Close other apps or make the export smaller, then try again.',
  render_stalled: 'Check the timeline and try exporting again.',
  render_cancelled: 'Start the export again when ready.',
  job_queue_timeout: 'Wait for other work to finish, then try again.',
  job_unknown: 'Start the task again.',
  job_expired: 'Start the task again.',
  job_not_visible: 'Return to the project where the task was started, then try again.',
  job_failed: 'Review the details and try again.',
  unsupported_platform: 'Open setup to see the options for this computer.',
  unimplemented: 'Choose another way to make this edit.',
  no_ui_client: 'Open the editor window, then try again.',
  guardrail: 'Review the selection and settings before trying again.',
}

function endsWithInstruction(reason: string): boolean {
  const sentences = reason.split(/(?<=[.!?])\s+/)
  const last = (sentences[sentences.length - 1] ?? '').trim()
  if (!last) return false
  if (/\b(?:failed|did not|could not|cannot|can't|unable|unavailable|not found|not installed|not ready|not writable|missing|locked|offline|timed out|refused|denied|error)\b/i.test(last)) {
    return false
  }
  return /^(?:please\s+)?(?:try\s+again|update\s+video|unlock\b|choose\b|select\b|open\b|install\b|repair\b|check\b|review\b|start\b|restart\b|wait\b|return\b|free\b|close\b|press\b|create\b|relink\b|trim\b|split\b|remove\b|set\b|switch\b|enable\b|disable\b|clear\b)\b/i.test(last)
}

/** Format a failure message for a person: message, optional human cause, then action. */
export function humanFailureMessage(
  error: {
    code?: string | null
    message?: string | null
    cause?: string | null
    suggested_action?: string | null
    requirement?: { id?: string } | null
  } | null | undefined,
  fallback: string,
): string {
  const captions = captionsFailureText(error)
  if (captions) return captions
  if (error?.requirement?.id === 'vc_runtime') {
    return 'This step needs the Microsoft Visual C++ runtime. Install it in Settings > AI & transcription, then try again.'
  }
  const rawMessage = error?.message?.trim() || ''
  const usedFallback = !rawMessage || isAgentInstruction(rawMessage)
  const reason = humanFailureReason(error?.message, fallback).replace(/[.:;\s]+$/, '')
  const code = error?.code?.toLowerCase() ?? ''

  const engineAction = error?.suggested_action?.trim() || ''
  let action = ''
  if (engineAction && !isAgentInstruction(engineAction)) {
    action = formatSentence(engineAction)
  } else if (code === 'invalid_args' && /audio is too long to label speakers/i.test(reason)) {
    action = 'Trim or split the asset and run speaker labelling on the parts.'
  } else if (code === 'guardrail' && /\blocked\b/i.test(reason)) {
    action = 'Unlock the track, then try again.'
  } else if (/(?:export|output)\s+folder\s+not\s+found\b/i.test(reason)) {
    action = 'Choose an existing export folder, then try again.'
  } else if (/(?:export|output)\s+folder\b.*\b(?:read.only|not\s+writable|cannot\s+be\s+written)\b/i.test(reason)) {
    action = 'Choose a writable export folder, then try again.'
  } else if (!endsWithInstruction(reason) && (code || usedFallback)) {
    action = GENERIC_CODE_ACTIONS[code] ?? 'Check your settings and try again.'
  }

  const cause = humanCauseSentence(error?.cause, reason, action)
  return dedupeSentences([formatSentence(reason), cause ?? '', action])
}

/** Plain words for a person when video processing is missing. The engine's
 *  suggested_action is written for agents, so it is not shown here; the
 *  notice's Open setup button leads to the platform's steps instead. */
export function missingVideoToolsMessage(result: VerbResult): string | null {
  if (result.ok || result.error?.code !== TOOL_MISSING) return null
  if (result.error?.requirement?.id === 'vc_runtime') return humanFailureMessage(result.error, 'This step could not start')
  const what = humanFailureReason(result.error?.message, 'Video processing is not installed')
  // A whole tool missing blocks all media work; a missing filter (e.g. no
  // vid.stab for Stabilize) needs a more complete build.
  return /is not installed$/.test(what)
    ? `${what}. Set up video processing to import, preview and export.`
    : `${what}. Open setup to get an FFmpeg that has it.`
}

export function userVerbFailureMessage(result: VerbResult, fallback: string): string | null {
  if (result.ok) return null
  const missing = missingVideoToolsMessage(result)
  if (missing) return missing
  return humanFailureMessage(result.error, fallback)
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
