import type { JobRecord, VerbResult } from '../lib/client'

type StopJob = { capture_id: string; status: 'finalizing'; job_id: string }
type ReadJob = (jobId: string) => Promise<VerbResult<JobRecord>>
type PollOptions = { wait?: (ms: number) => Promise<void>; now?: () => number; timeoutMs?: number }

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function pendingJob(value: unknown): value is StopJob {
  return object(value) && value.status === 'finalizing' &&
    typeof value.capture_id === 'string' && typeof value.job_id === 'string' && value.job_id.length > 0
}

function failure(message: string): VerbResult {
  return { ok: false, error: { code: 'sidecar', message, cause: 'recording_stop_job' } }
}

/** Wait for a durable Stop job before the recording UI announces a saved MP4. */
export async function resolveRecordingStopJob(
  reply: VerbResult, captureId: string, readJob: ReadJob, options: PollOptions = {},
): Promise<VerbResult> {
  if (!reply.ok || !object(reply.result) || reply.result.status !== 'finalizing') return reply
  if (!pendingJob(reply.result)) return failure('Stop returned an invalid finalization job.')
  const { job_id: jobId } = reply.result
  if (reply.result.capture_id !== captureId) return failure('Stop returned a different capture ID.')
  const now = options.now ?? Date.now
  const wait = options.wait ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)))
  const deadline = now() + (options.timeoutMs ?? 15 * 60_000)
  while (now() < deadline) {
    const status = await readJob(jobId)
    if (!status.ok) return failure(`Could not check whether Stop finished: ${status.error?.message ?? 'server unavailable'}`)
    const job = status.result
    if (!job || job.job_id !== jobId) return failure('Stop job status did not match the requested job.')
    if (job.state === 'failed' || job.outcome === 'failed' || job.outcome === 'cancelled' ||
      job.outcome === 'interrupted' || job.outcome === 'superseded') {
      if (job.error) return { ok: false, error: job.error }
      return failure(`Stop did not save the recording: ${job.message ?? job.outcome ?? 'job failed'}`)
    }
    if (job.state === 'done') {
      if (!object(job.result) || job.result.ok !== true || !object(job.result.result) ||
        job.result.result.capture_id !== captureId ||
        typeof job.result.result.raw_path !== 'string' || !job.result.result.raw_path) {
        return failure('Stop job ended without a confirmed raw MP4 for this capture.')
      }
      return job.result as unknown as VerbResult
    }
    await wait(Math.min(1000, Math.max(0, deadline - now())))
  }
  return failure('Stop is still finalizing. Check capture status before another action.')
}
