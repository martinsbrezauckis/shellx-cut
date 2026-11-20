// Recordings the person has not been told about yet, as one list: the app-wide
// notes from screen_record.pending (interrupted, saved while closing, failed),
// plus captures of the open project that still need recovery but have no note
// (for example from a version without notes). Pure, for tests.

export type RecoverySituation =
  | 'interrupted' | 'nothing_saved' | 'saved_on_close' | 'saved_not_added' | 'failed' | 'files_missing' | 'project_capture'

export interface RecoveryNotice {
  captureId: string
  projectPath: string | null
  projectName: string | null
  situation: RecoverySituation
  canRecover: boolean
  /** Sentence shown to the person. */
  message: string
  filesFolder: string | null
  recoveredExtentState?: 'not_measured' | 'measured'
  /** The numbers behind the "about N s" sentence in `message`, when the engine measured them. */
  recoveredMs?: number
  recordedMsAtLeast?: number
  recordedLongerUnknown?: boolean
}

const SITUATIONS = new Set<RecoverySituation>(['interrupted', 'nothing_saved', 'saved_on_close', 'saved_not_added', 'failed', 'files_missing'])

function text(value: unknown): string | null {
  return typeof value === 'string' && value.length > 0 ? value : null
}

/** Decode `screen_record.pending` safely; malformed rows are dropped. */
export function pendingNotices(result: unknown): RecoveryNotice[] {
  const rows = (result as { recordings?: unknown } | null)?.recordings
  if (!Array.isArray(rows)) return []
  return rows.flatMap((row): RecoveryNotice[] => {
    const r = row as Record<string, unknown>
    const captureId = text(r.capture_id)
    const message = text(r.message)
    const situation = r.situation as RecoverySituation
    if (!captureId || !message || !SITUATIONS.has(situation)) return []
    return [{
      captureId,
      projectPath: text(r.project_path),
      projectName: text(r.project_name),
      situation,
      canRecover: r.can_recover === true,
      message,
      filesFolder: text(r.files_folder),
      ...(r.recovered_extent_state === 'not_measured' || r.recovered_extent_state === 'measured'
        ? { recoveredExtentState: r.recovered_extent_state }
        : {}),
      ...(typeof r.recovered_ms === 'number' && typeof r.recorded_ms_at_least === 'number'
        ? { recoveredMs: r.recovered_ms, recordedMsAtLeast: r.recorded_ms_at_least, recordedLongerUnknown: r.recorded_longer_unknown === true }
        : {}),
    }]
  })
}

const PROJECT_CAPTURE_STATES = new Set(['owner_ambiguous', 'interrupted', 'recovered', 'quarantined'])

/** Open-project captures that still need recovery and have no app-wide note. */
export function projectCaptureNotices(recoveryStatus: unknown, known: ReadonlySet<string>): RecoveryNotice[] {
  const rows = (recoveryStatus as { captures?: unknown } | null)?.captures
  if (!Array.isArray(rows)) return []
  return rows.flatMap((row): RecoveryNotice[] => {
    const r = row as { capture_id?: unknown; state?: unknown; recoverable?: unknown; receipt?: { recovered_segments?: unknown } | null }
    const captureId = text(r.capture_id)
    if (!captureId || known.has(captureId) || !PROJECT_CAPTURE_STATES.has(String(r.state))) return []
    // An engine that reports `recoverable` is trusted; without it, an
    // interrupted capture whose receipt kept nothing has nothing to offer.
    const recoverable = typeof r.recoverable === 'boolean'
      ? r.recoverable
      : !(r.state === 'interrupted' && r.receipt?.recovered_segments === 0)
    if (!recoverable) return []
    return [{
      captureId,
      projectPath: null,
      projectName: null,
      situation: 'project_capture',
      canRecover: true,
      message: 'A recording in this project was interrupted. What was saved can be added to the timeline.',
      filesFolder: null,
    }]
  })
}

/** The person-facing result of screen_record.recover or its refusal. */
export function recoverOutcome(reply: { ok: boolean; result?: unknown; error?: { message?: string; suggested_action?: string | null } }): string {
  if (reply.ok) {
    const message = text((reply.result as { message?: unknown } | undefined)?.message)
    const skipped = (reply.result as { skipped_files?: unknown } | undefined)?.skipped_files
    const left = Array.isArray(skipped) && skipped.length > 0 ? ` Some files were not used: ${skipped.filter((item) => typeof item === 'string').join('; ')}.` : ''
    return (message ?? 'The recording was added to the end of the timeline.') + left
  }
  const message = reply.error?.message ?? 'The recording could not be recovered.'
  const action = reply.error?.suggested_action
  return action ? `${capitalize(message)}. ${capitalize(action)}.` : `${capitalize(message)}.`
}

function capitalize(value: string): string {
  return value ? value[0].toUpperCase() + value.slice(1) : value
}
