// Public jobs.list retry projection → Health & Recovery presentation.
//
// Eligibility remains engine-authored. This model only decides how to explain
// a terminal record and adds a defensive cancellation/supersession guard so a
// backward-compatible `state: failed` is never presented as an ordinary error.

import type { JobRecord } from '../../lib/clientModel'
import { jobFailureText } from '../../lib/jobFailureText'
import { jobTaskName } from '../../lib/jobPresentation'

export interface JobRetryRecoveryView {
  jobId: string
  kind: string
  state: string
  eligible: boolean
  rootJobId: string
  attempt: number
  label: string
  terminalLabel: 'Failed' | 'Cancelled' | 'Superseded' | 'Interrupted'
  canRetry: boolean
  /** The engine's reason and suggested next step, for a true failure. */
  failureText?: string
  unavailableReason?: string
  lineage?: string
}

function terminalLabel(job: JobRecord): JobRetryRecoveryView['terminalLabel'] {
  if (job.outcome === 'cancelled' || job.outcome_reason === 'user_cancelled' || job.outcome_reason === 'project_switch_cancelled') return 'Cancelled'
  if (job.outcome === 'superseded' || job.outcome_reason === 'superseded') return 'Superseded'
  if (job.outcome === 'interrupted' || job.outcome_reason === 'restart_interrupted') return 'Interrupted'
  return 'Failed'
}

function lineage(job: JobRecord): string | undefined {
  const retry = job.retry
  if (!retry) return undefined
  const parts = [`Attempt ${retry.attempt}`, `root ${retry.root_job_id}`]
  if (retry.retry_of) parts.push(`retry of ${retry.retry_of}`)
  if (retry.retried_by) parts.push(`admitted child ${retry.retried_by}`)
  return parts.join(' · ')
}

/** Engine eligibility is authoritative for failed-state records, including a
 * restart interruption. Cancelled/superseded records remain unavailable even
 * if a malformed legacy projection claims eligibility. */
export function failedJobRetryViews(records: JobRecord[]): JobRetryRecoveryView[] {
  return records
    .filter((job) => job.state === 'failed')
    .sort((left, right) => right.created_ts.localeCompare(left.created_ts) || right.job_id.localeCompare(left.job_id))
    .map((job) => {
      const terminal = terminalLabel(job)
      const engineEligible = job.retry?.eligible === true
      const canRetry = engineEligible && terminal !== 'Cancelled' && terminal !== 'Superseded'
      const unavailableReason = canRetry
        ? undefined
        : terminal === 'Cancelled'
          ? 'Cancelled jobs are not retried automatically.'
          : terminal === 'Superseded'
            ? 'Superseded jobs are not retried automatically.'
            : job.retry?.reason ?? 'This job has no engine-owned retry recipe.'
      return {
        jobId: job.job_id,
        kind: job.kind,
        state: job.state,
        eligible: engineEligible,
        rootJobId: job.retry?.root_job_id ?? job.job_id,
        attempt: job.retry?.attempt ?? 0,
        label: jobTaskName(job.kind),
        terminalLabel: terminal,
        canRetry,
        ...(terminal === 'Failed' && (job.error?.message || job.error?.suggested_action)
          ? { failureText: jobFailureText(job, '') }
          : {}),
        unavailableReason,
        lineage: lineage(job),
      }
    })
}

export function retryAdmissionLine(admission: {
  job_id: string
  retry_of: string
  root_job_id: string
  attempt: number
}): string {
  return `Retry queued · attempt ${admission.attempt} · retry of ${admission.retry_of} · root ${admission.root_job_id}`
}
