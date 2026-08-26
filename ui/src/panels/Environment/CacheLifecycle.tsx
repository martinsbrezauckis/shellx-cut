import { useCallback, useEffect, useState } from 'react'
import { callVerb } from '../../lib/client'
import type { JobRecord } from '../../lib/clientModel'
import type { ProjectCachePreviewResult } from '../../lib/clientResults'

interface CacheLifecycleProps {
  hasProject: boolean
  projectSession: number
  onComplete: () => Promise<void>
}

export default function CacheLifecycle({ hasProject, projectSession, onComplete }: CacheLifecycleProps) {
  const [preview, setPreview] = useState<ProjectCachePreviewResult | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)
  const [confirmationOpen, setConfirmationOpen] = useState(false)
  const [jobId, setJobId] = useState<string | null>(null)
  const [job, setJob] = useState<JobRecord | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [cancelPending, setCancelPending] = useState(false)

  useEffect(() => {
    setPreview(null)
    setConfirmationOpen(false)
    setJobId(null)
    setJob(null)
    setError(null)
  }, [projectSession])

  const previewCache = useCallback(async () => {
    if (!hasProject) return
    setPreviewLoading(true)
    setConfirmationOpen(false)
    setError(null)
    try {
      const response = await callVerb('project.cache_preview', {})
      if (!response.ok || !response.result) {
        setPreview(null)
        setError(response.error?.message ?? 'Cache ownership could not be verified. Nothing was deleted.')
        return
      }
      setPreview(response.result)
    } catch {
      setPreview(null)
      setError('Cache cleanup preview could not be read. Nothing was deleted.')
    } finally {
      setPreviewLoading(false)
    }
  }, [hasProject])

  const confirmPurge = useCallback(async () => {
    if (!preview || preview.purgeable.files === 0) return
    setError(null)
    try {
      const response = await callVerb('project.cache_purge', { plan_id: preview.plan_id, confirm: true })
      if (!response.ok || !response.result) {
        setError(response.error?.message ?? 'Cache cleanup did not start.')
        return
      }
      setJobId(response.result.job_id)
      setJob(null)
      setConfirmationOpen(false)
      setPreview(null)
    } catch {
      setError('Cache cleanup did not start.')
    }
  }, [preview])

  const cancelPurge = useCallback(async () => {
    if (!jobId) return
    setCancelPending(true)
    setError(null)
    try {
      const response = await callVerb('jobs.cancel', { job_id: jobId })
      if (!response.ok) setError(response.error?.message ?? 'Cache cleanup could not be cancelled.')
    } catch {
      setError('Cache cleanup could not be cancelled.')
    } finally {
      setCancelPending(false)
    }
  }, [jobId])

  useEffect(() => {
    if (!jobId) return
    let disposed = false
    let timer: number | null = null
    const poll = async () => {
      try {
        const response = await callVerb('jobs.status', { job_id: jobId })
        if (disposed) return
        if (!response.ok || !response.result || response.result.job_id !== jobId) {
          setError(response.error?.message ?? 'Cache cleanup status could not be verified.')
          return
        }
        const record = response.result
        setJob(record)
        if (record.state === 'done' || record.state === 'failed') {
          setJobId(null)
          void onComplete()
          return
        }
        timer = window.setTimeout(() => { void poll() }, 400)
      } catch {
        if (!disposed) setError('Cache cleanup status could not be verified.')
      }
    }
    void poll()
    return () => {
      disposed = true
      if (timer !== null) window.clearTimeout(timer)
    }
  }, [jobId, onComplete])

  return (
    <aside className="settings-cache-lifecycle" data-cut-cache-lifecycle>
      <div>
        <strong>Rebuildable cache cleanup</strong>
        <p>Preview only checks ledger-owned, aged, unreferenced proxies and filmstrips. Sources, exports, captures, and receipts are never eligible.</p>
      </div>
      <div className="settings-cache-lifecycle-actions">
        <button type="button" className="env-btn env-btn--ghost" data-cut-cache-preview onClick={() => void previewCache()} disabled={!hasProject || previewLoading || jobId !== null}>
          {previewLoading ? 'Previewing…' : 'Preview cache cleanup'}
        </button>
        {preview && <span data-cut-cache-preview-status>{preview.purgeable.files} aged file{preview.purgeable.files === 1 ? '' : 's'} / {preview.purgeable.bytes.toLocaleString()} bytes eligible</span>}
      </div>
      {error && <p className="settings-cache-lifecycle-error" data-cut-cache-error>{error}</p>}
      {preview && preview.purgeable.files === 0 && <p data-cut-cache-empty>No ledger-owned cache files are eligible for cleanup.</p>}
      {preview && preview.purgeable.files > 0 && !confirmationOpen && <button type="button" className="env-btn" data-cut-cache-purge onClick={() => setConfirmationOpen(true)}>Review and confirm purge</button>}
      {preview && preview.purgeable.files > 0 && confirmationOpen && (
        <div className="settings-cache-lifecycle-confirm" data-cut-cache-confirm>
          <p>Remove only the {preview.purgeable.files} previewed ledger-owned cache file{preview.purgeable.files === 1 ? '' : 's'}? This cannot delete source media, exports, captures, or receipts.</p>
          <div className="settings-cache-lifecycle-actions">
            <button type="button" className="env-btn env-btn--ghost" data-cut-cache-confirm-cancel onClick={() => setConfirmationOpen(false)}>Keep cache</button>
            <button type="button" className="env-btn" data-cut-cache-confirm-purge onClick={() => void confirmPurge()}>Purge previewed cache</button>
          </div>
        </div>
      )}
      {(jobId || job) && (
        <div data-cut-cache-job data-cut-cache-job-state={job?.state ?? 'queued'}>
          <p>{job?.message ?? 'Cache cleanup is queued.'} {job ? `${Math.round(job.progress * 100)}%` : ''}</p>
          {jobId && <button type="button" className="env-btn env-btn--ghost" data-cut-cache-cancel onClick={() => void cancelPurge()} disabled={cancelPending}>{cancelPending ? 'Cancelling…' : 'Cancel cleanup'}</button>}
          {!jobId && <button type="button" className="env-btn env-btn--ghost" data-cut-cache-remeasure onClick={() => void previewCache()}>Remeasure cache</button>}
        </div>
      )}
    </aside>
  )
}
