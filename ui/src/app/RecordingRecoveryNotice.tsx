// Tells the person about recordings that were interrupted or saved while the
// app was closing, at start and whenever the open project changes, and offers
// to add what was saved to the timeline. Uses the recording banner style.

import { useCallback, useEffect, useState } from 'react'
import { events } from '../lib/events'
import { callVerb } from '../lib/client'
import { usePublishedUiNotices, type UiNotice } from '../lib/uiNotices'
import { pendingNotices, projectCaptureNotices, recoverOutcome, type RecoveryNotice } from './recordingRecoveryModel'

export const OPEN_RECORDING_RECOVERY_EVENT = 'cut:open-recording-recovery'

export type Row = RecoveryNotice & { busy?: boolean; result?: string }
export type Outcome = Partial<Pick<Row, 'busy' | 'result' | 'canRecover'>>

/** What the two row actions need from the app, passed in so the actions can be
 *  tested without a mounted page. */
export interface RecoveryEffects {
  call: typeof callVerb
  update: (row: Row, patch: Outcome) => void
}

/** "Add to timeline": open the recording's project when it is another one,
 *  then recover the capture and show what happened. */
export async function recoverRecording(row: Row, { call, update }: RecoveryEffects): Promise<void> {
  update(row, { busy: true, result: row.projectPath ? `Opening “${row.projectName ?? 'the project'}”…` : 'Recovering…' })
  try {
    if (row.projectPath) {
      const opened = await call('project.open', { path: row.projectPath })
      if (!opened.ok) {
        update(row, { busy: false, result: `The project could not be opened: ${opened.error?.message ?? 'unknown error'}.` })
        return
      }
      update(row, { result: 'Recovering…' })
    }
    const reply = await call('screen_record.recover', { capture_id: row.captureId })
    update(row, { busy: false, canRecover: !reply.ok && row.canRecover, result: recoverOutcome(reply) })
  } catch (error) {
    update(row, { busy: false, result: `Recovery failed: ${error instanceof Error ? error.message : 'server unavailable'}.` })
  }
}

/** "Dismiss" / "Later" / "OK": tell the engine the note was seen (app-wide
 *  notes only, and only before an outcome is shown), then hide the row. */
export async function dismissRecording(
  row: Row,
  { call, update, forget, hide }: RecoveryEffects & { forget: (captureId: string) => void; hide: (captureId: string) => void },
): Promise<void> {
  if (row.situation !== 'project_capture' && !row.result) {
    const reply = await call('screen_record.pending_dismiss', { capture_id: row.captureId })
    if (!reply.ok && reply.error?.code !== 'not_found') {
      update(row, { result: `Could not dismiss: ${reply.error?.message ?? 'unknown error'}.` })
      return
    }
  }
  forget(row.captureId)
  hide(row.captureId)
}

/** One banner row with its two actions. */
export function RecoveryRowView({ row, onRecover, onDismiss }: {
  row: Row
  onRecover: () => void
  onDismiss: () => void
}) {
  return (
    <div
      className="app__recording-session"
      data-cut-recording-session={row.result && !row.busy ? 'done' : 'recovery'}
      data-cut-recording-recovery-capture={row.captureId}
      data-cut-recording-recovery-situation={row.situation}
      role="status"
    >
      <span>
        {row.result ?? row.message}
        {!row.result && !row.canRecover && row.filesFolder && <> The files stay in {row.filesFolder}.</>}
      </span>
      {!row.result && row.canRecover && (
        <button type="button" data-cut-action="recording-recover" disabled={row.busy} onClick={onRecover}>
          Add to timeline
        </button>
      )}
      <button type="button" data-cut-action="recording-recovery-dismiss" disabled={row.busy} onClick={onDismiss}>
        {row.result ? 'OK' : row.situation === 'project_capture' ? 'Later' : 'Dismiss'}
      </button>
    </div>
  )
}

/** The `ui.state.notices` entries for the rows shown (at most three) and the rest. */
export function recoveryNotices(visible: readonly Row[]): UiNotice[] {
  const shown: UiNotice[] = visible.slice(0, 3).map((row) => ({
    kind: 'recording_recovery',
    capture_id: row.captureId,
    situation: row.situation,
    can_recover: row.canRecover,
    ...(row.recoveredExtentState ? { recovered_extent_state: row.recoveredExtentState } : {}),
    ...(row.recoveredMs !== undefined
      ? { recovered_ms: row.recoveredMs, recorded_ms_at_least: row.recordedMsAtLeast, recorded_longer_unknown: row.recordedLongerUnknown }
      : {}),
    state: row.busy ? 'busy' : row.result ? 'done' : 'offered',
  }))
  return visible.length > 3 ? [...shown, { kind: 'recording_recovery_more', count: visible.length - 3 }] : shown
}

// Opening the recording's project re-renders the app and can remount this
// banner while a recovery is still running. Outcomes therefore live outside
// the component, and every mounted banner refreshes when one changes.
const outcomes = new Map<string, Row>()
const CHANGED_EVENT = 'cut:recording-recovery-changed'
function setOutcome(row: Row, outcome: Outcome): void {
  outcomes.set(row.captureId, { ...row, ...outcomes.get(row.captureId), ...outcome })
  window.dispatchEvent(new Event(CHANGED_EVENT))
}

/** Preparation seals the measured extent; refresh the existing banner then. */
export function subscribeRecoveryPreparation(refresh: () => void, source: Pick<typeof events, 'subscribe'> = events): () => void {
  return source.subscribe((event) => {
    if (event.type === 'job_progress' && event.kind === 'recording_recovery' && event.terminal) refresh()
  })
}

export default function RecordingRecoveryNotice({ projectKey, onVisibleChange }: {
  projectKey: string | null
  /** The app gives the notice its own layout row only while it shows. */
  onVisibleChange?: (visible: boolean) => void
}) {
  const [rows, setRows] = useState<Row[]>([])
  const [hidden, setHidden] = useState<ReadonlySet<string>>(new Set())

  const refresh = useCallback(async () => {
    try {
      const pending = await callVerb('screen_record.pending', {})
      const notices = pending.ok ? pendingNotices(pending.result) : []
      const known = new Set(notices.map((notice) => notice.captureId))
      let local: RecoveryNotice[] = []
      if (projectKey) {
        const status = await callVerb('screen_record.recovery_status', { limit: 100 })
        if (status.ok) local = projectCaptureNotices(status.result, known)
      }
      setRows(() => {
        const listed = [...notices, ...local]
        // Keep rows that have an outcome (in progress or finished) even when
        // the engine no longer lists them, so the person sees the result.
        const kept = [...outcomes.values()].filter((row) => !listed.some((notice) => notice.captureId === row.captureId))
        return [...kept, ...listed].map((row) => ({ ...row, ...outcomes.get(row.captureId) }))
      })
    } catch { /* the engine is unreachable; the next refresh tries again */ }
  }, [projectKey])

  useEffect(() => { void refresh() }, [refresh])
  useEffect(() => subscribeRecoveryPreparation(() => { void refresh() }), [refresh])
  useEffect(() => {
    const open = () => { setHidden(new Set()); void refresh() }
    const changed = () => {
      setRows((current) => current.map((row) => ({ ...row, ...outcomes.get(row.captureId) })))
      void refresh()
    }
    window.addEventListener(OPEN_RECORDING_RECOVERY_EVENT, open)
    window.addEventListener(CHANGED_EVENT, changed)
    return () => {
      window.removeEventListener(OPEN_RECORDING_RECOVERY_EVENT, open)
      window.removeEventListener(CHANGED_EVENT, changed)
    }
  }, [refresh])

  const update = (row: Row, patch: Outcome) => {
    setRows((current) => current.map((item) => (item.captureId === row.captureId ? { ...item, ...patch } : item)))
    setOutcome(row, patch)
  }

  const effects: RecoveryEffects = { call: callVerb, update }
  const recover = (row: Row) => recoverRecording(row, effects)
  const dismiss = (row: Row) => dismissRecording(row, {
    ...effects,
    forget: (captureId) => { outcomes.delete(captureId) },
    hide: (captureId) => setHidden((current) => new Set([...current, captureId])),
  })

  const visible = rows.filter((row) => !hidden.has(row.captureId))
  const shown = visible.length > 0
  useEffect(() => { onVisibleChange?.(shown) }, [onVisibleChange, shown])
  usePublishedUiNotices('recording-recovery', recoveryNotices(visible))
  if (!shown) return null
  return (
    <div data-cut-recording-recovery-notice="true">
      {visible.slice(0, 3).map((row) => (
        <RecoveryRowView key={row.captureId} row={row} onRecover={() => void recover(row)} onDismiss={() => void dismiss(row)} />
      ))}
      {visible.length > 3 && (
        <div className="app__recording-session" data-cut-recording-session="recovery" role="status">
          <span>{visible.length - 3} more recordings need attention. Handle the ones above first.</span>
        </div>
      )}
    </div>
  )
}
