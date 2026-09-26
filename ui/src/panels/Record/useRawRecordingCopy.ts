import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import { withAuthorizedOutputPath } from '../../lib/exportDestination'
import { isTauri, pickExportOutput } from '../../lib/tauri'

/** Post-Stop Save a copy; the first raw MP4 has already been saved. */
export function useRawRecordingCopy(rawPath: string | null) {
  const [jobId, setJobId] = useState<string | null>(null)
  const [note, setNote] = useState('')
  const pendingRef = useRef(false)

  useEffect(() => {
    if (!jobId) return
    let stale = false
    const poll = async () => {
      const reply = await callVerb('jobs.status', { job_id: jobId })
      if (stale) return
      if (!reply.ok || !reply.result) { setNote(`Could not check copy: ${reply.error?.message ?? 'unknown error'}`); setJobId(null); return }
      const status = reply.result as { state?: string; progress?: number; message?: string; outcome?: string; result?: { path?: string }; error?: { message?: string } }
      if (status.state === 'done') { setNote('Copy saved.'); setJobId(null); return }
      if (status.state === 'failed') { setNote(status.outcome === 'cancelled' ? 'Copy cancelled.' : `Copy failed: ${status.error?.message ?? 'unknown error'}`); setJobId(null); return }
      setNote(status.message?.trim() || (status.state === 'queued' ? 'Copy queued…' : 'Saving copy…'))
    }
    void poll()
    const timer = window.setInterval(() => { void poll() }, 500)
    return () => { stale = true; window.clearInterval(timer) }
  }, [jobId])

  const saveCopy = useCallback(async () => {
    if (!rawPath || jobId || pendingRef.current) return
    if (!isTauri()) { setNote('Save a copy needs the desktop app. The original MP4 is already saved.'); return }
    pendingRef.current = true
    setNote('Choosing where to save the copy…')
    try {
      const path = await pickExportOutput({ title: 'Save a copy of the raw recording', defaultPath: 'raw_recording-copy.mp4', filters: [{ name: 'MP4 video', extensions: ['mp4'] }] })
      if (!path) { setNote('Copy cancelled. The original MP4 is saved.'); return }
      setNote('Queueing raw MP4 copy…')
      const reply = await withAuthorizedOutputPath(path, () => callVerb('screen_record.copy_raw', { source: rawPath, path }))
      if (!reply.ok || !reply.result) { setNote(`Copy failed: ${reply.error?.message ?? 'unknown error'}`); return }
      const id = (reply.result as { job_id?: string }).job_id
      if (!id) { setNote('Copy was not confirmed: no job ID returned.'); return }
      setJobId(id)
    } catch (error) {
      setNote(`Could not save copy: ${error instanceof Error ? error.message : 'unknown error'}`)
    } finally { pendingRef.current = false }
  }, [jobId, rawPath])

  const cancelCopy = useCallback(async () => {
    if (!jobId) return
    setNote('Cancelling copy…')
    const reply = await callVerb('jobs.cancel', { job_id: jobId })
    if (!reply.ok) setNote(`Could not confirm cancellation: ${reply.error?.message ?? 'unknown error'}`)
  }, [jobId])

  return { jobId, note, saveCopy, cancelCopy }
}
