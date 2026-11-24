/** Coalesces native and DOM callbacks for one physical F9 and blocks a handoff. */
export class RecordingToggleGate {
  private lastAccepted = -Infinity
  private inFlight = false

  claim(now: number): boolean {
    if (this.inFlight || now - this.lastAccepted < 400) return false
    this.lastAccepted = now
    return true
  }

  setInFlight(value: boolean): void { this.inFlight = value }
}

/** `reason: 'app_exit'` when Stop runs because the app is closing. */
export function stopArgs(captureId: string, raw: boolean, reason?: 'app_exit') {
  return reason
    ? { capture_id: captureId, autoedit: !raw, mux_raw: true, reason } as const
    : { capture_id: captureId, autoedit: !raw, mux_raw: true } as const
}

export function classifyStopFailure(
  captureId: string,
  status: { ok: boolean; code?: string; result?: { capture_id?: string; terminal?: boolean } },
): { state: 'live' | 'ended' | 'unknown'; detail: string } {
  if (status.ok && status.result?.capture_id === captureId) {
    return status.result.terminal === true
      ? { state: 'ended', detail: `The native capture ended. ${SAVED_PART_HINT}` }
      : status.result.terminal === false
        ? { state: 'live', detail: 'The native capture is still active.' }
        : { state: 'unknown', detail: 'The recorder did not confirm whether capture is still active.' }
  }
  if (!status.ok && status.code === 'not_found') {
    return { state: 'ended', detail: `The native capture is no longer active. ${SAVED_PART_HINT}` }
  }
  return { state: 'unknown', detail: 'Capture state could not be checked.' }
}

/** Where a saved part of a failed recording is offered. */
const SAVED_PART_HINT = 'If part of it was saved, add it with Recover recordings in Settings > Health & recovery.'

export interface EngineError { code?: string; message?: string; suggested_action?: string | null }

function sentence(text: string): string {
  const trimmed = text.trim().replace(/[.\s]+$/, '')
  return trimmed ? `${trimmed[0].toUpperCase()}${trimmed.slice(1)}.` : ''
}

/**
 * What the engine refused and the way forward it names, as the person reads
 * it: "<what happened>. <What to do>." A refusal never loses its action.
 */
export function refusalText(error: EngineError | null | undefined, fallback = 'unknown error'): string {
  const message = sentence(error?.message || fallback)
  const action = error?.suggested_action ? sentence(error.suggested_action) : ''
  return action ? `${message} ${action}` : message
}

/** A Stop that came before the first screen frame: there is no take at all. */
export function isNothingRecorded(error: EngineError | null | undefined): boolean {
  const message = error?.message ?? ''
  return message === 'nothing was recorded' || message.includes('before the first frame')
}

/** The whole truth for an early Stop: nothing to recover, and how to record. */
export function nothingRecordedText(error: EngineError | null | undefined): string {
  const action = error?.suggested_action
    ? sentence(error.suggested_action)
    : 'Start the recording again and wait until the timer is running before you stop.'
  return `Nothing was recorded: Stop came before the first screen frame was captured. ${action}`
}

/**
 * What Stop said is wrong with a take it saved anyway (`notes`), as one
 * sentence group the person reads after the saved message. Empty for an
 * exact take: a take is never discarded because a check found a problem.
 */
export function savedWithProblemText(notes: unknown): string {
  if (!Array.isArray(notes)) return ''
  const lines = notes.filter((note): note is string => typeof note === 'string' && note.trim().length > 0)
  return lines.length > 0 ? `Saved with a problem: ${lines.join(' ')}` : ''
}

/** The saved message with any notes after it. */
export function withSavedNotes(message: string, notes: unknown): string {
  const problem = savedWithProblemText(notes)
  return problem ? `${message} ${problem}` : message
}

/**
 * The finalizing text while Stop runs, from `screen_record.status.saving`
 * ({parts_done, parts_total}); the plain text when the engine reports none.
 */
export function savingProgressText(saving: unknown, plain: string): string {
  const s = saving as { parts_done?: unknown; parts_total?: unknown } | null | undefined
  const done = typeof s?.parts_done === 'number' ? s.parts_done : null
  const total = typeof s?.parts_total === 'number' ? s.parts_total : null
  if (done === null || total === null || total < 1) return plain
  return `Saving the recording: part ${Math.min(done + 1, total)} of ${total}…`
}


/** A finished recording's notice (saved, clip added, input events) belongs to the project
 *  it was made in: once another project is open, or none, it is no longer true to show. */
export function recordingNoticeBelongsElsewhere(
  state: { phase: string; resultProjectIdentity?: { origin_path_sha256?: string } | null },
  openProject: { origin_path_sha256?: string } | null | undefined,
): boolean {
  if (state.phase !== 'done' && state.phase !== 'error') return false
  if (!state.resultProjectIdentity) return false
  return !openProject || openProject.origin_path_sha256 !== state.resultProjectIdentity.origin_path_sha256
}
