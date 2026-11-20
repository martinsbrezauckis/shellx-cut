// Failed-job rows of the status bar: one row per failed job the person has not
// dismissed, in the same pill style as the retry row. The row names what
// failed and why; "Details" opens Health and Recovery, where the full text
// stays listed after the row is dismissed. Work a kill or power cut stopped
// (`jobs.interrupted`) uses the same row with its full text.

import type { FailedJobNotices } from '../lib/failedJobNotices'
import { interruptedKey } from '../lib/interruptedJobNotices'
import { usePublishedUiNotices, type UiNotice } from '../lib/uiNotices'

export interface FailedJobRowsProps {
  failed: FailedJobNotices
  onDismiss: (jobId: string) => void
  onOpenDetails: () => void
}

/** The `ui.state.notices` entries for the rows shown and the "+N more failed" count. */
export function failedJobUiNotices(failed: FailedJobNotices): UiNotice[] {
  const rows: UiNotice[] = failed.shown.map((job) => ({ kind: 'failed_job', job_id: job.job_id, job_kind: job.kind }))
  return failed.hidden > 0 ? [...rows, { kind: 'failed_jobs_more', count: failed.hidden }] : rows
}

export function FailedJobRows({ failed, onDismiss, onOpenDetails }: FailedJobRowsProps) {
  usePublishedUiNotices('failed-jobs', failedJobUiNotices(failed))
  return (
    <>
      {failed.shown.map((job) => (
        <span
          key={job.job_id}
          className="sb-job sb-job--failed"
          role="status"
          data-cut-job-failed-row={job.job_id}
          data-cut-job-failed-kind={job.kind}
          title={`${job.title}. ${job.text}`}
        >
          <span className="sb-job-dot sb-job-dot--failed" />
          <span className="sb-job-failed-title" data-cut-job-failed-title>{job.title}</span>
          <span className="sb-job-failed-text" data-cut-job-failed-text>{job.text}</span>
          {/* An interrupted job's row holds its whole text; Health lists project jobs only. */}
          {interruptedKey(job.job_id) === null ? <button
            type="button"
            className="sb-job-retry"
            data-cut-job-failed-details={job.job_id}
            title={`Read the full message for ${job.title.toLowerCase()}`}
            aria-label={`Read the full message for ${job.title.toLowerCase()}`}
            onClick={onOpenDetails}
          >
            Details
          </button> : null}
          <button
            type="button"
            className="sb-job-cancel"
            data-cut-job-failed-dismiss={job.job_id}
            title={`Dismiss: ${job.title.toLowerCase()}`}
            aria-label={`Dismiss: ${job.title.toLowerCase()}`}
            onClick={() => onDismiss(job.job_id)}
          >
            ×
          </button>
        </span>
      ))}
      {failed.hidden > 0 ? (
        <button
          type="button"
          className="sb-job-failed-more"
          data-cut-job-failed-more={failed.hidden}
          title="Open the list of failed jobs"
          onClick={onOpenDetails}
        >
          +{failed.hidden} more failed
        </button>
      ) : null}
    </>
  )
}
