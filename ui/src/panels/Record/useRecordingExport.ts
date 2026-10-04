//! Async screen-record export status and cancellation UI.

import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import { callVerb } from '../../lib/client'
import { withAuthorizedOutputPath } from '../../lib/exportDestination'

export interface FinishedRecording { source: string; plan: string }
export interface RecordingExportJob {
  id: string
  format: 'mp4' | 'gif'
  startedAt: number
  ownerKey: string
  projectKey: string
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
  projectKey: string | null
  ownerKey: string | null
  format: 'mp4' | 'gif'
  outputPath: string | null
  setNote: Dispatch<SetStateAction<string>>
}

/// Queue a fenced export, then tell the user exactly whether it is queued,
/// rendering, saved, cancelled, or failed. The server owns the output lease
/// after the immediate response, so the temporary UI authorization may end.
export function useRecordingExport({ capture, projectKey, ownerKey, format, outputPath, setNote }: Props) {
  const [job, setJob] = useState<RecordingExportJob | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const admittedRef = useRef<RecordingExportJob | null>(null)
  const pendingRef = useRef(false)
  const cancelRef = useRef(false)
  const statusInFlightRef = useRef<Promise<unknown> | null>(null)
  const wakeRef = useRef<(() => void) | null>(null)
  const liveRef = useRef(true)
  const ownerRef = useRef(ownerKey)
  const projectRef = useRef(projectKey)
  const ownerGenerationRef = useRef({ ownerKey, projectKey, value: 0 })
  if (ownerGenerationRef.current.ownerKey !== ownerKey || ownerGenerationRef.current.projectKey !== projectKey) {
    ownerGenerationRef.current = { ownerKey, projectKey, value: ownerGenerationRef.current.value + 1 }
  }
  ownerRef.current = ownerKey
  projectRef.current = projectKey

  useEffect(() => {
    liveRef.current = true
    return () => { liveRef.current = false }
  }, [])

  useEffect(() => {
    if (job?.projectKey !== projectKey && job) {
      setNote('A previous project export is unresolved. Return to that project to check its status.')
    } else if (job && job.ownerKey !== ownerKey) {
      setNote('A previous recording export is still being checked. Wait before exporting this capture.')
    }
  }, [job, ownerKey, projectKey, setNote])

  useEffect(() => {
    if (!job || job.projectKey !== projectKey) return
    let stale = false
    let timer: number | null = null
    let inFlight = false
    let wakeRequested = false
    let failures = 0
    const current = () => !stale && liveRef.current && admittedRef.current === job
      && job.projectKey === projectRef.current
    const unknown = () => {
      failures += 1
      if (job.ownerKey === ownerRef.current) setNote('Export status unknown; checking again…')
    }
    const poll = async () => {
      if (!current() || inFlight) return
      const prior = statusInFlightRef.current
      if (prior) {
        try { await prior } catch { /* the prior effect reports its own failure */ }
        if (!current() || inFlight) return
      }
      inFlight = true
      let request: ReturnType<typeof callVerb<'jobs.status'>> | null = null
      try {
        request = callVerb('jobs.status', { job_id: job.id, expected_origin_path_sha256: job.projectKey })
        statusInFlightRef.current = request
        const response = await request
        if (!current()) return
        const status = response.result
        if (!response.ok || !status) unknown()
        else if (status.job_id !== job.id || status.kind !== 'screen_record_export'
          || !['queued', 'running', 'done', 'failed'].includes(status.state)) unknown()
        else {
          failures = 0
          if (status.state === 'done' || status.state === 'failed') {
            admittedRef.current = null
            setJob(null)
            if (job.ownerKey !== ownerRef.current) setNote('Previous recording export finished. You can export this capture now.')
            else if (status.state === 'done') {
              const result = status.result as { elapsed_ms?: number } | undefined
              const duration = typeof result?.elapsed_ms === 'number' ? ` (${(result.elapsed_ms / 1000).toFixed(1)}s)` : ''
              setNote(`Saved ${job.format.toUpperCase()}.${duration}`)
            } else if (status.outcome === 'cancelled' || status.error?.code === 'job_cancelled' || status.error?.code === 'render_cancelled') {
              setNote(`Export cancelled after ${elapsed(job.startedAt)}.`)
            } else setNote(`export failed: ${status.error?.message ?? 'render failed'}`)
            return
          }
          if (job.ownerKey === ownerRef.current) {
            const verb = status.state === 'queued' ? 'Queued' : 'Rendering'
            const phase = typeof status.message === 'string' && status.message.trim()
              ? status.message.trim() : `${verb} ${job.format.toUpperCase()}…`
            setNote(`${phase} · ${elapsed(job.startedAt)}`)
          }
        }
      } catch {
        if (current()) unknown()
      } finally {
        if (statusInFlightRef.current === request) statusInFlightRef.current = null
        inFlight = false
        if (current()) {
          const delay = wakeRequested ? 0 : failures ? Math.min(3_000, 500 * 2 ** failures) : 500
          wakeRequested = false
          timer = window.setTimeout(() => { void poll() }, delay)
        }
      }
    }
    wakeRef.current = () => {
      if (!current()) return
      if (inFlight) { wakeRequested = true; return }
      if (timer !== null) window.clearTimeout(timer)
      timer = null
      void poll()
    }
    void poll()
    return () => {
      stale = true
      if (timer !== null) window.clearTimeout(timer)
      wakeRef.current = null
    }
  }, [job, projectKey, setNote])

  const exportClip = useCallback(async () => {
    if (!capture || !ownerKey || !projectKey) { setNote('export failed: no finished recording to export'); return }
    if (pendingRef.current || admittedRef.current) return
    pendingRef.current = true
    setSubmitting(true)
    setNote(`Queueing ${format.toUpperCase()} export…`)
    const path = outputPath ?? undefined
    const intentGeneration = ownerGenerationRef.current.value
    const intentCurrent = () => liveRef.current && ownerKey === ownerRef.current
      && projectKey === projectRef.current && intentGeneration === ownerGenerationRef.current.value
    let submitted = false
    try {
      const response = await withAuthorizedOutputPath(path, () => {
        if (!intentCurrent()) throw new Error('recording changed before export started')
        submitted = true
        return callVerb('screen_record.export', {
          source: capture.source,
          plan: capture.plan,
          format,
          path,
          expected_origin_path_sha256: projectKey,
        })
      })
      if (!liveRef.current) return
      if (!response.ok) {
        if (ownerKey === ownerRef.current) setNote(`export failed: ${response.error?.message ?? 'error'}`)
        return
      }
      const id = (response.result as { job_id?: unknown } | undefined)?.job_id
      if (typeof id !== 'string' || !id.trim() || id !== id.trim()) {
        if (projectKey === projectRef.current) setNote('Export admission could not be confirmed. Check Jobs before exporting again; another attempt may create a duplicate.')
        else setNote('A previous project export admission could not be confirmed. Check Jobs there before exporting again; another attempt may create a duplicate.')
        return
      }
      const admitted = { id, format, startedAt: Date.now(), ownerKey, projectKey }
      admittedRef.current = admitted
      setJob(admitted)
      if (projectKey !== projectRef.current) setNote('A previous project export is unresolved. Return to that project to check its status.')
      else if (ownerKey !== ownerRef.current) setNote('A previous recording export is still being checked. Wait before exporting this capture.')
    } catch (error) {
      if (!liveRef.current) return
      if (submitted) {
        if (projectKey === projectRef.current) setNote('Export admission could not be confirmed. Check Jobs before exporting again; another attempt may create a duplicate.')
        else setNote('A previous project export admission could not be confirmed. Check Jobs there before exporting again; another attempt may create a duplicate.')
        return
      }
      if (!intentCurrent()) {
        setNote('Recording changed before export started. Choose Export again.')
        return
      }
      const reason = failureReason(error)
      if (ownerKey === ownerRef.current) {
        setNote(path ? `export failed: ${reason} — ${OUTPUT_PATH_HINT}` : `export failed: ${reason}`)
      }
    } finally {
      pendingRef.current = false
      if (liveRef.current) setSubmitting(false)
    }
  }, [capture, format, outputPath, ownerKey, projectKey, setNote])

  const cancelExport = useCallback(async () => {
    const admitted = admittedRef.current
    if (!admitted || admitted.projectKey !== projectRef.current || cancelRef.current) return
    cancelRef.current = true
    setNote(`Cancelling ${admitted.format.toUpperCase()} export…`)
    try {
      const response = await callVerb('jobs.cancel', { job_id: admitted.id, expected_origin_path_sha256: admitted.projectKey })
      if (liveRef.current && admittedRef.current === admitted && admitted.projectKey === projectRef.current && !response.ok) {
        setNote('Export cancellation not confirmed; checking export status…')
      }
    } catch {
      if (liveRef.current && admittedRef.current === admitted && admitted.projectKey === projectRef.current) {
        setNote('Export cancellation not confirmed; checking export status…')
      }
    } finally {
      cancelRef.current = false
      wakeRef.current?.()
    }
  }, [setNote])

  return { exportJob: job, exportRunning: !!job || submitting,
    exportCancelable: !!job && job.projectKey === projectKey, exportClip, cancelExport }
}
