// Person-facing notices the app is showing right now, so `ui.state.notices`
// lets an agent read what the person is being told: the shared action notice,
// the odd project size notice, the recording recovery rows and the failed-job
// rows of the status bar. Each component that shows such a notice publishes
// its entries here while it shows them and clears them when they go.
//
// Entries are structured and path-free on purpose (`ui.state` is path-safe):
// they name the notice, the verb or engine record behind it and its code, and
// the agent reads the full wording from that verb or record.

import { useEffect } from 'react'

export type UiNotice =
  | {
    /** The shared notice for a person's own action (`UserActionFeedback`). */
    kind: 'action_feedback'
    /** The verb whose result is shown, or `drop` / `app` for app-side notices. */
    source: string
    /** The refusal or warning code, when there is one. */
    code: string | null
    /** The `ui.open` panel behind the notice's Open setup button, when shown. */
    setup_surface: string | null
    /** How many dropped items were not imported (`source: "drop"` only). */
    count?: number
  }
  | {
    kind: 'odd_project_size'
    /** open: shown for the odd project; refused: an export was refused; done: changed. */
    state: 'open' | 'refused' | 'done'
    width: number
    height: number
    even_width: number
    even_height: number
  }
  | {
    kind: 'recording_recovery'
    capture_id: string
    /** The `situation` of `screen_record.pending`, or `project_capture`. */
    situation: string
    can_recover: boolean
    /** Whether preparation has measured which checkpoints recovery kept. */
    recovered_extent_state?: 'not_measured' | 'measured'
    /** What recovery would keep (ms), the recorded time the files prove (at least, ms) and whether it may have run longer: the numbers behind the notice's "about N s" line. Absent while checkpoint preparation has not measured recovery or the files tell nothing. */
    recovered_ms?: number
    recorded_ms_at_least?: number
    recorded_longer_unknown?: boolean
    /** offered: waiting for the person; busy: recovering; done: the outcome is shown. */
    state: 'offered' | 'busy' | 'done'
  }
  | { kind: 'recording_recovery_more'; count: number }
  | { kind: 'failed_job'; job_id: string; job_kind: string }
  | { kind: 'failed_jobs_more'; count: number }

const byOwner = new Map<string, readonly UiNotice[]>()
let snapshot: readonly UiNotice[] = []
const listeners = new Set<() => void>()

function publish(): void {
  snapshot = [...byOwner.values()].flat().map((entry) => ({ ...entry }))
  for (const listener of listeners) listener()
}

/** Replace the entries one owner (a mounted notice) shows; [] clears them. */
export function setUiNotices(owner: string, notices: readonly UiNotice[]): void {
  const current = byOwner.get(owner) ?? []
  if (JSON.stringify(current) === JSON.stringify(notices)) return
  if (notices.length === 0) byOwner.delete(owner)
  else byOwner.set(owner, notices.map((entry) => ({ ...entry })))
  publish()
}

/** Stable between changes, so it can back `useSyncExternalStore`. */
export function getUiNotices(): readonly UiNotice[] {
  return snapshot
}

export function subscribeUiNotices(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** Publish `notices` for `owner` while the calling component is mounted. */
export function usePublishedUiNotices(owner: string, notices: readonly UiNotice[]): void {
  const key = JSON.stringify(notices)
  useEffect(() => {
    setUiNotices(owner, JSON.parse(key) as UiNotice[])
  }, [owner, key])
  useEffect(() => () => setUiNotices(owner, []), [owner])
}
