//! Async screen-record export status and cancellation UI.

import { useCallback, useEffect, useState, type Dispatch, type SetStateAction } from 'react'
import { callVerb } from '../../lib/client'
import { withAuthorizedOutputPath } from '../../lib/exportDestination'

export interface FinishedRecording { source: string; plan: string }
export interface RecordingExportJob {
  id: string
  path: string
  format: 'mp4' | 'gif'
  startedAt: number
}

const OUTPUT_PATH_HINT =
  'pick another file with "Choose file", or Clear it to use the default export folder'

function failureReason(error: unknown): string {
  if (error instanceof TypeError) return 'server unreachable'
  return error instanceof Error && error.message ? error.message : 'server unreachable'
}

function elapsed(startedAt: number): string {
  const seconds = Math.floor((Date.now() - startedAt) / 1000)
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`
}

interface Props {
  capture: FinishedRecording | null
  format: 'mp4' | 'gif'
  outputPath: string | null
  setNote: Dispatch<SetStateAction<string>>
}

/// Queue a fenced export, then tell the user exactly whether it is queued,
/// rendering, saved, cancelled, or failed. The server owns the output lease
/// after the immediate response, so the temporary UI authorization may end.
export function useRecordingExport({ capture, format, outputPath, setNote }: Props) {
  const [job, setJob] = useState<RecordingExportJob | null>(null)

  useEffect(() => {
    if (!job) return
    let stale = false
    const poll = async () => {
      const response = await callVerb('jobs.status', { job_id: job.id })
      if (stale) return
      if (!response.ok || !response.result) {
        setNote(`export failed: ${response.error?.message ?? 'could not read export status'}`)
        setJob(null)
        return
      }
      const status = response.result as {
        state?: string
        progress?: number
        message?: string
        result?: { path?: string; elapsed_ms?: number }
        outcome?: string
        error?: { code?: string; message?: string; cause?: string }
      }
      if (status.state === 'done') {
        const ms = status.result?.elapsed_ms
        const duration = typeof ms === 'number' ? ` (${(ms / 1000).toFixed(1)}s)` : ''
        setNote(`Saved ${job.format.toUpperCase()} → ${status.result?.path ?? job.path}${duration}`)
        setJob(null)
        return
      }
      if (status.state === 'failed') {
        if (status.outcome === 'cancelled' || status.error?.code === 'job_cancelled' || status.error?.code === 'render_cancelled') {
          setNote(`Export cancelled after ${elapsed(job.startedAt)}.`)
          setJob(null)
          return
        }
        const detail = status.error?.cause ? `: ${status.error.cause}` : ''
        setNote(`export failed: ${status.error?.message ?? 'render failed'}${detail}`)
        setJob(null)
        return
      }
      const verb = status.state === 'queued' ? 'Queued' : 'Rendering'
      // `message` is the server's bounded export phase. During the real
      // compositor pass it includes confirmed frame progress; because every
      // progress update is persisted, jobs.status.updated_ts remains the
      // durable last-progress timestamp for diagnostics as well.
      const phase = typeof status.message === 'string' && status.message.trim()
        ? status.message.trim()
        : `${verb} ${job.format.toUpperCase()}…`
      setNote(`${phase} · ${elapsed(job.startedAt)}`)
    }
    void poll()
    const timer = window.setInterval(() => { void poll() }, 500)
    return () => {
      stale = true
      window.clearInterval(timer)
    }
  }, [job, setNote])

  const exportClip = useCallback(async () => {
    if (!capture) { setNote('export failed: no finished recording to export'); return }
    if (job) return
    setNote(`Queueing ${format.toUpperCase()} export…`)
    const path = outputPath ?? undefined
    try {
      const response = await withAuthorizedOutputPath(path, () =>
        callVerb('screen_record.export', {
          source: capture.source,
          plan: capture.plan,
          format,
          path,
        }))
      if (!response.ok || !response.result) {
        setNote(`export failed: ${response.error?.message ?? 'error'}`)
        return
      }
      const result = response.result as { job_id?: string; path?: string }
      if (!result.job_id) {
        setNote('export failed: server returned no export job id')
        return
      }
      setJob({ id: result.job_id, path: result.path ?? 'output', format, startedAt: Date.now() })
    } catch (error) {
      const reason = failureReason(error)
      setNote(path
        ? `export failed: ${reason} — ${OUTPUT_PATH_HINT} (${path})`
        : `export failed: ${reason}`)
    }
  }, [capture, format, job, outputPath, setNote])

  const cancelExport = useCallback(async () => {
    if (!job) return
    setNote(`Cancelling ${job.format.toUpperCase()} export…`)
    const response = await callVerb('jobs.cancel', { job_id: job.id })
    if (!response.ok) {
      setNote(`export cancellation pending: ${response.error?.message ?? 'retry shortly'}`)
    }
  }, [job, setNote])

  return { exportJob: job, exportClip, cancelExport }
}
