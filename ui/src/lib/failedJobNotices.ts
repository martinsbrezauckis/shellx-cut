// Failed background jobs that a person must see, whoever started them.
//
// The job list carries every terminal record. A retry-eligible failure has its
// own status-bar row; a cancelled, replaced or shutdown-interrupted job is not
// a failure. Everything else that failed after the project was opened becomes
// a notice, newest few shown, the rest counted so nothing disappears silently.
// An export that was WRITTEN but did not pass its own checks
// (`outcome_reason: checks_failed`, e.g. a cut-off source) is listed the same
// way: "finished with problems", with the checks in plain words. A batch
// (render queue) that finished with some items failed (`items_failed`) is
// listed too: "finished with failures", with the count and each failed item.

import { jobFailureText } from './jobFailureText'
import type { JobRecord } from './jobModel'
import { jobTaskName } from './jobPresentation'

/** Most notices shown at once; older ones are counted, not dropped. */
export const FAILED_JOB_NOTICE_LIMIT = 3

export interface FailedJobNotice {
  job_id: string
  kind: string
  /** "<what> failed", in the person's words. */
  title: string
  /** The plain reason followed by the suggested next step. */
  text: string
}

export interface FailedJobNotices {
  shown: FailedJobNotice[]
  /** Failures beyond the limit, still reachable in Health and Recovery. */
  hidden: number
}

function endedByRequest(job: JobRecord): boolean {
  return (
    job.outcome === 'cancelled'
    || job.outcome === 'superseded'
    || job.outcome === 'interrupted'
    || job.outcome_reason === 'user_cancelled'
    || job.outcome_reason === 'project_switch_cancelled'
    || job.outcome_reason === 'superseded'
    || job.outcome_reason === 'restart_interrupted'
    // Older records carry only the error code.
    || job.error?.code === 'job_cancelled'
    || job.error?.code === 'render_cancelled'
    || job.error?.code === 'job_superseded'
  )
}

/** A failed job the person has not been told about elsewhere: not retryable
 * (the retry row owns those), not ended on request, and ended since `sinceMs`
 * so history from an earlier session is not replayed. */
function isNoticeableFailure(job: JobRecord, sinceMs: number): boolean {
  const failed = job.state === 'failed' && job.retry?.eligible !== true && !endedByRequest(job)
  if (!failed && !finishedWithProblems(job)) return false
  const endedAt = Date.parse(job.updated_ts)
  return Number.isFinite(endedAt) && endedAt >= sinceMs
}

/** Done, but not as asked: a file that did not pass its own checks, or a
 * batch with some of its items failed. */
export function finishedWithProblems(job: JobRecord): boolean {
  return job.state === 'done' && (job.outcome_reason === 'checks_failed' || job.outcome_reason === 'items_failed')
}

/** The engine's own words: the verdict and the way forward, or for a batch
 * with failed items its summary line and each failed item. */
export function problemsText(job: JobRecord): string {
  if (job.outcome_reason === 'items_failed') {
    const batch = (job.result ?? {}) as { summary_line?: unknown; failures?: unknown }
    const line = typeof batch.summary_line === 'string' ? batch.summary_line.trim() : ''
    const failed = Array.isArray(batch.failures)
      ? batch.failures.filter((w): w is string => typeof w === 'string')
      : []
    const text = [line && `${line}.`, ...failed.map((w) => `${w}.`)].filter(Boolean).join(' ')
    return text || `${job.message ?? 'Some items failed.'} Open the job for details.`
  }
  const result = (job.result ?? {}) as { checks?: { summary?: unknown; action?: unknown } }
  const summary = typeof result.checks?.summary === 'string' ? result.checks.summary.trim() : ''
  const action = typeof result.checks?.action === 'string' ? result.checks.action.trim() : ''
  const text = [summary, action].filter(Boolean).join(' ')
  return text || 'The file was written but did not pass its checks. Open the receipt for details.'
}

export function failedJobNotices(
  records: readonly JobRecord[],
  sinceMs: number,
  dismissed: ReadonlySet<string>,
): FailedJobNotices {
  const newestFirst = records
    .filter((job) => !dismissed.has(job.job_id) && isNoticeableFailure(job, sinceMs))
    .sort((left, right) => right.updated_ts.localeCompare(left.updated_ts) || right.job_id.localeCompare(left.job_id))
  const task = (job: JobRecord) => jobTaskName(job.kind)
  const shown = newestFirst.slice(0, FAILED_JOB_NOTICE_LIMIT).reverse().map((job) => (
    finishedWithProblems(job)
      ? {
        job_id: job.job_id,
        kind: job.kind,
        title: `${task(job)} finished with ${job.outcome_reason === 'items_failed' ? 'failures' : 'problems'}`,
        text: problemsText(job),
      }
      : {
        job_id: job.job_id,
        kind: job.kind,
        title: `${task(job)} failed`,
        text: jobFailureText(job, `${task(job)} did not finish.`),
      }
  ))
  return { shown, hidden: Math.max(0, newestFirst.length - FAILED_JOB_NOTICE_LIMIT) }
}
