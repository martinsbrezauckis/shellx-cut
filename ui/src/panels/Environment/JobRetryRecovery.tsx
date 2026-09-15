import { useEffect, useRef, useState } from 'react'
import { callVerb, type VerbResults } from '../../lib/client'
import type { JobsListResult } from '../../lib/clientResults'
import {
  failedJobRetryViews,
  retryAdmissionLine,
  type JobRetryRecoveryView,
} from './jobRetryRecoveryModel'

interface RetryAdmission {
  job_id: string
  retry_of: string
  root_job_id: string
  attempt: number
  status: 'queued'
  path?: string
  format?: 'mp4' | 'gif'
}

interface JobRetryRecoveryProps {
  jobs: JobsListResult | null
  projectSession: number
  onRefresh: () => Promise<void>
}

/** The sole mutation control for durable job retry. Status-bar chips navigate
 * here so pending/admitted state stays in one place and a second click cannot
 * create a second request while the first response is in flight. */
export default function JobRetryRecovery({ jobs, projectSession, onRefresh }: JobRetryRecoveryProps) {
  const [pending, setPending] = useState<Record<string, true>>({})
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [admissions, setAdmissions] = useState<Record<string, RetryAdmission>>({})
  const retrying = useRef(new Set<string>())

  useEffect(() => {
    retrying.current.clear()
    setPending({})
    setErrors({})
    setAdmissions({})
  }, [projectSession])

  const retry = async (job: JobRetryRecoveryView) => {
    if (!job.canRetry || retrying.current.has(job.jobId) || admissions[job.jobId]) return
    retrying.current.add(job.jobId)
    setErrors((previous) => {
      const next = { ...previous }
      delete next[job.jobId]
      return next
    })
    setPending((previous) => ({ ...previous, [job.jobId]: true }))
    try {
      const response = await callVerb('jobs.retry', { job_id: job.jobId })
      if (!response.ok || !response.result) {
        setErrors((previous) => ({
          ...previous,
          [job.jobId]: response.error?.message ?? 'The retry was not admitted.',
        }))
        return
      }
      const result = response.result as VerbResults['jobs.retry']
      setAdmissions((previous) => ({
        ...previous,
        [job.jobId]: {
          job_id: result.job_id,
          retry_of: result.retry_of,
          root_job_id: result.root_job_id,
          attempt: result.attempt,
          status: result.status,
          ...('path' in result && 'format' in result ? { path: result.path, format: result.format } : {}),
        },
      }))
      void onRefresh().catch(() => {
        // The admitted response and its lineage remain truthful even when the
        // follow-up health refresh cannot complete.
      })
    } catch {
      setErrors((previous) => ({
        ...previous,
        [job.jobId]: 'Server unreachable. Reconnect, then retry this job.',
      }))
    } finally {
      retrying.current.delete(job.jobId)
      setPending((previous) => {
        const next = { ...previous }
        delete next[job.jobId]
        return next
      })
    }
  }

  const views = failedJobRetryViews(jobs?.jobs ?? [])
  if (views.length === 0) return null

  return (
    <section className="settings-job-retry" aria-labelledby="settings-job-retry-title" data-cut-job-retry-recovery>
      <div className="settings-job-retry-head">
        <div>
          <h4 id="settings-job-retry-title">Failed job recovery</h4>
          <p>Only the engine’s durable retry eligibility can admit a fresh job. Cancelled and superseded jobs stay distinct from failures.</p>
        </div>
      </div>
      <ul className="settings-job-retry-list">
        {views.map((job) => {
          const admission = admissions[job.jobId]
          const isPending = pending[job.jobId] === true
          const error = errors[job.jobId]
          return (
            <li
              key={job.jobId}
              className="settings-job-retry-item"
              data-cut-job-retry-item={job.jobId}
              data-cut-job-retry-state={admission ? 'admitted' : job.canRetry ? 'eligible' : 'unavailable'}
            >
              <div className="settings-job-retry-copy">
                <div>
                  <strong>{job.label}</strong>
                  <span className={`settings-job-retry-terminal settings-job-retry-terminal--${job.terminalLabel.toLowerCase()}`}>{job.terminalLabel}</span>
                </div>
                {job.lineage && <small data-cut-job-retry-lineage>{job.lineage}</small>}
                {admission ? (
                  <p
                    className="settings-job-retry-admitted"
                    data-cut-job-retry-admitted={job.jobId}
                    data-cut-job-retry-child={admission.job_id}
                    data-cut-job-retry-of={admission.retry_of}
                    data-cut-job-retry-root={admission.root_job_id}
                    data-cut-job-retry-attempt={admission.attempt}
                    data-cut-job-retry-status={admission.status}
                    data-cut-job-retry-path={admission.path}
                    data-cut-job-retry-format={admission.format}
                  >{retryAdmissionLine(admission)}</p>
                ) : job.canRetry ? (
                  error && <p className="settings-job-retry-error" data-cut-job-retry-error={job.jobId}>{error}</p>
                ) : (
                  <p className="settings-job-retry-unavailable" data-cut-job-retry-unavailable={job.jobId}>{job.unavailableReason}</p>
                )}
              </div>
              {!admission && job.canRetry && (
                <button
                  type="button"
                  className="env-btn env-btn--ghost"
                  data-cut-job-retry-action={job.jobId}
                  data-cut-job-retry-source-kind={job.kind}
                  data-cut-job-retry-source-state={job.state}
                  data-cut-job-retry-source-eligible={job.eligible ? 'true' : 'false'}
                  data-cut-job-retry-source-root={job.rootJobId}
                  data-cut-job-retry-source-attempt={job.attempt}
                  data-cut-job-retry-pending={isPending ? 'true' : undefined}
                  disabled={isPending}
                  onClick={() => void retry(job)}
                >
                  {isPending ? 'Retrying…' : 'Retry'}
                </button>
              )}
            </li>
          )
        })}
      </ul>
    </section>
  )
}
