// Work someone asked for that was cut off because the app or the computer
// stopped while it ran (a kill, a crash, a power cut, Windows ending the
// session). The engine keeps a note per such job outside the project
// (`jobs.interrupted`), so the next start can say so before any project is
// open. The notice uses the failed-job row of the status bar.

import type { FailedJobNotice } from './failedJobNotices'
import { jobTaskName } from './jobPresentation'

export interface InterruptedJob {
  key: string
  job_id: string
  kind: string
  project_name: string
  leftover_files_removed: number
  /** Helpers of the dead run (its encoder) that were still running and were stopped. */
  helpers_stopped?: number
}

/** Row ids of interrupted jobs carry this prefix; dismissing one calls
 * `jobs.interrupted_dismiss` with the rest. */
export const INTERRUPTED_ROW_PREFIX = 'interrupted:'

export function interruptedKey(rowId: string): string | null {
  return rowId.startsWith(INTERRUPTED_ROW_PREFIX) ? rowId.slice(INTERRUPTED_ROW_PREFIX.length) : null
}

export function readInterruptedJobs(result: unknown): InterruptedJob[] {
  const items = (result as { interrupted?: unknown })?.interrupted
  if (!Array.isArray(items)) return []
  return items.filter((item): item is InterruptedJob => (
    typeof item?.key === 'string' && typeof item?.job_id === 'string' && typeof item?.kind === 'string'
  ))
}

export function interruptedJobNotices(jobs: readonly InterruptedJob[]): FailedJobNotice[] {
  return jobs.map((job) => {
    const task = jobTaskName(job.kind)
    // What the engine did, in the order it did it: a writer still running is
    // stopped first, so "removed" is never said while something refills it.
    const stopped = (job.helpers_stopped ?? 0) > 0
      ? ` Its ${task.toLowerCase()} was still running in the background and was stopped.`
      : ''
    const removed = job.leftover_files_removed > 0
      ? ` Its ${job.leftover_files_removed} unfinished file${job.leftover_files_removed === 1 ? ' was' : 's were'} removed.`
      : ''
    const where = job.project_name ? ` in “${job.project_name}”` : ''
    return {
      job_id: `${INTERRUPTED_ROW_PREFIX}${job.key}`,
      kind: job.kind,
      title: `${task} was interrupted`,
      text: `ShellX Cut stopped while this ${task.toLowerCase()} was running${where}, so it did not finish.${stopped}${removed} Start it again.`,
    }
  })
}
