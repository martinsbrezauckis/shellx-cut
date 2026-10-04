import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import { withAuthorizedOutputPath } from '../../lib/exportDestination'
import { isTauri, pickExportOutput } from '../../lib/tauri'

interface CopyJob { id: string; projectKey: string; ownerKey: string }

/** Post-Stop Save a copy; the first raw MP4 has already been saved. */
export function useRawRecordingCopy(rawPath: string | null, projectKey: string | null, takeKey: string | null = null) {
  const [job, setJob] = useState<CopyJob | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [note, setNote] = useState('')
  const pendingRef = useRef(false)
  const admittedRef = useRef<CopyJob | null>(null)
  const cancelRef = useRef(false)
  const statusInFlightRef = useRef<Promise<unknown> | null>(null)
  const wakeRef = useRef<(() => void) | null>(null)
  const liveRef = useRef(true)
  const projectRef = useRef(projectKey)
  const ownerKey = projectKey && rawPath ? JSON.stringify([projectKey, takeKey, rawPath]) : null
  const ownerRef = useRef(ownerKey)
  const ownerGenerationRef = useRef({ projectKey, ownerKey, value: 0 })
  if (ownerGenerationRef.current.projectKey !== projectKey || ownerGenerationRef.current.ownerKey !== ownerKey) {
    ownerGenerationRef.current = { projectKey, ownerKey, value: ownerGenerationRef.current.value + 1 }
  }
  projectRef.current = projectKey
  ownerRef.current = ownerKey

  useEffect(() => {
    liveRef.current = true
    return () => { liveRef.current = false }
  }, [])

  useEffect(() => {
    if (job && job.projectKey !== projectKey) {
      setNote('A previous project copy is unresolved. Return to that project to check its status.')
    } else if (job && job.ownerKey !== ownerKey) {
      setNote('A previous recording copy is still being checked. Wait before copying this recording.')
    }
  }, [job, projectKey, ownerKey])

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
      if (job.ownerKey === ownerRef.current) setNote('Copy status unknown; checking again…')
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
        const reply = await request
        if (!current()) return
        const status = reply.result
        if (!reply.ok || !status) unknown()
        else if (status.job_id !== job.id || status.kind !== 'screen_record_copy_raw'
          || !['queued', 'running', 'done', 'failed'].includes(status.state)) unknown()
        else {
          failures = 0
          if (status.state === 'done' || status.state === 'failed') {
            admittedRef.current = null
            setJob(null)
            if (job.ownerKey !== ownerRef.current) setNote('Previous recording copy finished. You can copy this recording now.')
            else if (status.state === 'done') setNote('Copy saved.')
            else setNote(status.outcome === 'cancelled' || status.error?.code === 'job_cancelled'
              ? 'Copy cancelled.' : `Copy failed: ${status.error?.message ?? 'unknown error'}`)
            return
          }
          if (job.ownerKey === ownerRef.current) {
            const fallback = status.state === 'queued' ? 'Copy queued…' : 'Saving copy…'
            setNote(typeof status.message === 'string' && status.message.trim() ? status.message.trim() : fallback)
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
  }, [job, projectKey])

  const saveCopy = useCallback(async () => {
    if (!rawPath || !ownerKey || !projectKey || pendingRef.current || admittedRef.current) return
    if (!isTauri()) { setNote('Save a copy needs the desktop app. The original MP4 is already saved.'); return }
    pendingRef.current = true
    setSubmitting(true)
    setNote('Choosing where to save the copy…')
    const intentGeneration = ownerGenerationRef.current.value
    const intentCurrent = () => liveRef.current && projectKey === projectRef.current
      && ownerKey === ownerRef.current && intentGeneration === ownerGenerationRef.current.value
    let submitted = false
    try {
      const path = await pickExportOutput({ title: 'Save a copy of the raw recording', defaultPath: 'raw_recording-copy.mp4', filters: [{ name: 'MP4 video', extensions: ['mp4'] }] })
      if (!liveRef.current) return
      if (!intentCurrent()) {
        setNote('Recording changed before copy started. Choose Save a copy again.')
        return
      }
      if (!path) {
        if (ownerKey === ownerRef.current) setNote('Copy cancelled. The original MP4 is saved.')
        return
      }
      if (ownerKey === ownerRef.current) setNote('Queueing raw MP4 copy…')
      const reply = await withAuthorizedOutputPath(path, () => {
        if (!intentCurrent()) {
          throw new Error('recording changed before copy started')
        }
        submitted = true
        return callVerb('screen_record.copy_raw', { source: rawPath, path, expected_origin_path_sha256: projectKey })
      })
      if (!liveRef.current) return
      if (!reply.ok) {
        if (ownerKey === ownerRef.current) setNote(`Copy failed: ${reply.error?.message ?? 'unknown error'}`)
        return
      }
      const id = (reply.result as { job_id?: unknown } | undefined)?.job_id
      if (typeof id !== 'string' || !id.trim() || id !== id.trim()) {
        setNote(projectKey === projectRef.current
          ? 'Copy admission could not be confirmed. Check Jobs before copying again; another attempt may create a duplicate.'
          : 'A previous project copy admission could not be confirmed. Check Jobs there before copying again; another attempt may create a duplicate.')
        return
      }
      const admitted = { id, projectKey, ownerKey }
      admittedRef.current = admitted
      setJob(admitted)
      if (projectKey !== projectRef.current) setNote('A previous project copy is unresolved. Return to that project to check its status.')
      else if (ownerKey !== ownerRef.current) setNote('A previous recording copy is still being checked. Wait before copying this recording.')
    } catch (error) {
      if (!liveRef.current) return
      if (submitted) {
        setNote(projectKey === projectRef.current
          ? 'Copy admission could not be confirmed. Check Jobs before copying again; another attempt may create a duplicate.'
          : 'A previous project copy admission could not be confirmed. Check Jobs there before copying again; another attempt may create a duplicate.')
      } else if (!intentCurrent()) {
        setNote('Recording changed before copy started. Choose Save a copy again.')
      } else {
        setNote(`Could not save copy: ${error instanceof Error ? error.message : 'unknown error'}`)
      }
    } finally {
      pendingRef.current = false
      if (liveRef.current) setSubmitting(false)
    }
  }, [rawPath, ownerKey, projectKey])

  const cancelCopy = useCallback(async () => {
    const admitted = admittedRef.current
    if (!admitted || admitted.projectKey !== projectRef.current || cancelRef.current) return
    cancelRef.current = true
    setNote('Cancelling copy…')
    try {
      const reply = await callVerb('jobs.cancel', { job_id: admitted.id, expected_origin_path_sha256: admitted.projectKey })
      if (liveRef.current && admittedRef.current === admitted && admitted.projectKey === projectRef.current && !reply.ok) {
        setNote('Copy cancellation not confirmed; checking copy status…')
      }
    } catch {
      if (liveRef.current && admittedRef.current === admitted && admitted.projectKey === projectRef.current) {
        setNote('Copy cancellation not confirmed; checking copy status…')
      }
    } finally {
      cancelRef.current = false
      wakeRef.current?.()
    }
  }, [])

  return { jobId: job?.id ?? null, running: !!job || submitting,
    cancelable: !!job && job.projectKey === projectKey, note, saveCopy, cancelCopy }
}
