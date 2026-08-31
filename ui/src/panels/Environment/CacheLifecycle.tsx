import { useCallback, useEffect, useState } from 'react'
import { callVerb } from '../../lib/client'
import type { JobRecord } from '../../lib/clientModel'
import type {
  ProjectCachePreviewResult,
  ProjectCacheRebuildCounts,
  ProjectCacheRebuildResult,
} from '../../lib/clientResults'

interface CacheLifecycleProps {
  hasProject: boolean
  projectSession: number
  /** Null until Health & Recovery has a complete revision-bound media aggregate. */
  rebuildAssetIds: string[] | null
  onComplete: () => Promise<void>
}

type CacheJobKind = 'cleanup' | 'rebuild'

interface ActiveCacheJob {
  id: string
  kind: CacheJobKind
}

function refusedCount(counts: ProjectCacheRebuildCounts): number {
  return counts.source_changed
    + counts.source_unavailable
    + counts.unowned_outputs
    + counts.legacy_outputs
    + counts.unsupported_assets
}

function attentionReasons(counts: ProjectCacheRebuildCounts): string[] {
  const plural = (count: number, singular: string, pluralValue = `${singular}s`) => `${count} ${count === 1 ? singular : pluralValue}`
  return [
    counts.source_changed > 0 ? `${plural(counts.source_changed, 'source')} changed` : null,
    counts.source_unavailable > 0 ? `${plural(counts.source_unavailable, 'source')} unavailable` : null,
    counts.unowned_outputs > 0 ? `${plural(counts.unowned_outputs, 'existing cache entry')} not owned by Cut` : null,
    counts.legacy_outputs > 0 ? `${plural(counts.legacy_outputs, 'existing cache entry')} uses an older Cut format` : null,
    counts.unsupported_assets > 0 ? `${plural(counts.unsupported_assets, 'media item')} is not supported for cache rebuild` : null,
  ].filter((reason): reason is string => reason !== null)
}

function rebuildSummary(result: ProjectCacheRebuildResult, remainingAssets: number): string {
  const queued = result.scheduled_assets
  const upToDate = result.counts.fresh_assets
  const more = remainingAssets > 0
    ? ` ${remainingAssets} more item${remainingAssets === 1 ? '' : 's'} can be rebuilt after this batch completes.`
    : ''
  if (result.status === 'already_queued') {
    return `${queued} item${queued === 1 ? '' : 's'} already queued.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}${more}`
  }
  if (result.status === 'not_needed') {
    return `No rebuild was needed.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}`
  }
  return `${queued} item${queued === 1 ? '' : 's'} queued.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}${more}`
}

function jobLabel(kind: CacheJobKind): string {
  return kind === 'rebuild' ? 'Cache rebuild' : 'Cache cleanup'
}

function isTerminalJob(record: JobRecord): boolean {
  return record.state === 'done'
    || record.state === 'failed'
    || record.outcome === 'cancelled'
    || record.outcome === 'interrupted'
    || record.outcome === 'superseded'
}

export default function CacheLifecycle({ hasProject, projectSession, rebuildAssetIds, onComplete }: CacheLifecycleProps) {
  const [preview, setPreview] = useState<ProjectCachePreviewResult | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)
  const [confirmationOpen, setConfirmationOpen] = useState(false)
  const [activeJob, setActiveJob] = useState<ActiveCacheJob | null>(null)
  const [job, setJob] = useState<JobRecord | null>(null)
  const [cleanupError, setCleanupError] = useState<string | null>(null)
  const [cleanupSuccess, setCleanupSuccess] = useState<string | null>(null)
  const [cancelPending, setCancelPending] = useState(false)
  const [rebuildLoading, setRebuildLoading] = useState(false)
  const [rebuildResult, setRebuildResult] = useState<ProjectCacheRebuildResult | null>(null)
  const [rebuildError, setRebuildError] = useState<string | null>(null)
  const [rebuildSuccess, setRebuildSuccess] = useState<string | null>(null)

  useEffect(() => {
    setPreview(null)
    setPreviewLoading(false)
    setConfirmationOpen(false)
    setActiveJob(null)
    setJob(null)
    setCleanupError(null)
    setCleanupSuccess(null)
    setCancelPending(false)
    setRebuildLoading(false)
    setRebuildResult(null)
    setRebuildError(null)
    setRebuildSuccess(null)
  }, [projectSession])

  const previewCache = useCallback(async () => {
    if (!hasProject || activeJob) return
    setPreviewLoading(true)
    setConfirmationOpen(false)
    setCleanupError(null)
    setCleanupSuccess(null)
    try {
      const response = await callVerb('project.cache_preview', {})
      if (!response.ok || !response.result) {
        setPreview(null)
        setCleanupError(response.error?.message ?? 'Cache ownership could not be verified. Nothing was deleted.')
        return
      }
      setPreview(response.result)
    } catch {
      setPreview(null)
      setCleanupError('Cache cleanup preview could not be read. Nothing was deleted.')
    } finally {
      setPreviewLoading(false)
    }
  }, [activeJob, hasProject])

  const rebuildBatch = rebuildAssetIds?.slice(0, 64) ?? []
  const remainingRebuildAssets = Math.max(0, (rebuildAssetIds?.length ?? 0) - rebuildBatch.length)

  const rebuildCache = useCallback(async () => {
    if (!hasProject || rebuildBatch.length === 0 || rebuildLoading || activeJob) return
    setRebuildLoading(true)
    setRebuildError(null)
    setRebuildSuccess(null)
    try {
      const response = await callVerb('project.cache_rebuild', { asset_ids: rebuildBatch })
      if (!response.ok || !response.result) {
        setRebuildResult(null)
        setRebuildError(response.error?.message ?? 'Cache rebuild could not be scheduled.')
        return
      }
      setRebuildResult(response.result)
      if (response.result.status === 'not_needed') {
        setRebuildSuccess('Editing cache is already up to date.')
      } else if (response.result.job_id) {
        setActiveJob({ id: response.result.job_id, kind: 'rebuild' })
        setJob(null)
      } else {
        setRebuildError('Cache rebuild did not return a job to track.')
      }
    } catch {
      setRebuildResult(null)
      setRebuildError('Cache rebuild could not be scheduled.')
    } finally {
      setRebuildLoading(false)
    }
  }, [activeJob, hasProject, rebuildBatch, rebuildLoading])

  const confirmPurge = useCallback(async () => {
    if (!preview || preview.purgeable.files === 0 || activeJob) return
    setCleanupError(null)
    setCleanupSuccess(null)
    try {
      const response = await callVerb('project.cache_purge', { plan_id: preview.plan_id, confirm: true })
      if (!response.ok || !response.result) {
        setCleanupError(response.error?.message ?? 'Cache cleanup did not start.')
        return
      }
      setActiveJob({ id: response.result.job_id, kind: 'cleanup' })
      setJob(null)
      setConfirmationOpen(false)
      setPreview(null)
    } catch {
      setCleanupError('Cache cleanup did not start.')
    }
  }, [activeJob, preview])

  const cancelActiveJob = useCallback(async () => {
    if (!activeJob) return
    setCancelPending(true)
    if (activeJob.kind === 'rebuild') setRebuildError(null)
    else setCleanupError(null)
    try {
      const response = await callVerb('jobs.cancel', { job_id: activeJob.id })
      if (!response.ok) {
        const message = response.error?.message ?? `${jobLabel(activeJob.kind)} could not be cancelled.`
        if (activeJob.kind === 'rebuild') setRebuildError(message)
        else setCleanupError(message)
      }
    } catch {
      const message = `${jobLabel(activeJob.kind)} could not be cancelled.`
      if (activeJob.kind === 'rebuild') setRebuildError(message)
      else setCleanupError(message)
    } finally {
      setCancelPending(false)
    }
  }, [activeJob])

  useEffect(() => {
    if (!activeJob) return
    let disposed = false
    let timer: number | null = null
    const poll = async () => {
      try {
        const response = await callVerb('jobs.status', { job_id: activeJob.id })
        if (disposed) return
        if (!response.ok || !response.result || response.result.job_id !== activeJob.id) {
          const message = response.error?.message ?? `${jobLabel(activeJob.kind)} status could not be verified.`
          if (activeJob.kind === 'rebuild') setRebuildError(message)
          else setCleanupError(message)
          timer = window.setTimeout(() => { void poll() }, 800)
          return
        }
        const record = response.result
        setJob(record)
        if (isTerminalJob(record)) {
          setActiveJob(null)
          if (record.state === 'done') {
            if (activeJob.kind === 'rebuild') setRebuildSuccess('Cache rebuild completed. Editing cache status was refreshed.')
            else setCleanupSuccess('Cache cleanup completed. Recheck the preview before any later cleanup.')
          } else {
            const message = record.message ?? (record.outcome === 'cancelled'
              ? `${jobLabel(activeJob.kind)} was cancelled. You can safely try again.`
              : `${jobLabel(activeJob.kind)} did not finish. You can safely try again.`)
            if (activeJob.kind === 'rebuild') setRebuildError(message)
            else setCleanupError(message)
          }
          void onComplete()
          return
        }
        timer = window.setTimeout(() => { void poll() }, 400)
      } catch {
        if (!disposed) {
          const message = `${jobLabel(activeJob.kind)} status could not be verified.`
          if (activeJob.kind === 'rebuild') setRebuildError(message)
          else setCleanupError(message)
          timer = window.setTimeout(() => { void poll() }, 800)
        }
      }
    }
    void poll()
    return () => {
      disposed = true
      if (timer !== null) window.clearTimeout(timer)
    }
  }, [activeJob, onComplete])

  const rebuildDisabled = !hasProject || rebuildAssetIds === null || rebuildBatch.length === 0 || rebuildLoading || activeJob !== null
  const cleanupDisabled = !hasProject || previewLoading || activeJob !== null
  const rebuildUnavailable = !hasProject
    ? 'Open a project to rebuild its editing cache.'
    : rebuildAssetIds === null
      ? 'Finish the Health & Recovery check before rebuilding cache.'
      : rebuildBatch.length === 0
        ? 'This completed check found no missing cache to rebuild.'
    : activeJob
      ? `${jobLabel(activeJob.kind)} is in progress.`
      : null

  return (
    <aside className="settings-cache-lifecycle" data-cut-cache-lifecycle>
      <section className="settings-cache-lifecycle-rebuild" aria-labelledby="settings-cache-rebuild-title" data-cut-cache-rebuild-section>
        <div>
          <strong id="settings-cache-rebuild-title">Editing cache</strong>
          <p>Rebuild only missing or stale Cut-owned proxies and base filmstrips. Original media stays unchanged.</p>
        </div>
        <div className="settings-cache-lifecycle-actions">
          <button
            type="button"
            className="env-btn env-btn--ghost"
            data-cut-cache-rebuild
            onClick={() => void rebuildCache()}
            disabled={rebuildDisabled}
            aria-describedby="cut-cache-rebuild-status"
          >
            {rebuildLoading ? 'Scheduling…' : 'Rebuild missing cache'}
          </button>
          {rebuildUnavailable && <span data-cut-cache-rebuild-disabled>{rebuildUnavailable}</span>}
        </div>
        {rebuildResult && (
          <div
            id="cut-cache-rebuild-status"
            data-cut-cache-rebuild-status
            data-cut-cache-rebuild-state={rebuildResult.status}
            data-cut-cache-rebuild-queued={rebuildResult.scheduled_assets}
            data-cut-cache-rebuild-up-to-date={rebuildResult.counts.fresh_assets}
            data-cut-cache-rebuild-attention-count={refusedCount(rebuildResult.counts)}
            aria-live="polite"
          >
            <p>{rebuildSummary(rebuildResult, remainingRebuildAssets)}</p>
            {refusedCount(rebuildResult.counts) > 0 && (
              <details className="settings-cache-lifecycle-attention" data-cut-cache-rebuild-attention>
                <summary data-cut-cache-rebuild-attention-toggle>{refusedCount(rebuildResult.counts)} item{refusedCount(rebuildResult.counts) === 1 ? ' needs' : 's need'} attention</summary>
                <ul>
                  {attentionReasons(rebuildResult.counts).map((reason) => <li key={reason}>{reason}</li>)}
                </ul>
              </details>
            )}
          </div>
        )}
        {rebuildSuccess && <p className="settings-cache-lifecycle-success" data-cut-cache-rebuild-success aria-live="polite">{rebuildSuccess}</p>}
        {rebuildError && <p className="settings-cache-lifecycle-error" data-cut-cache-rebuild-error role="alert">{rebuildError}</p>}
      </section>

      <section className="settings-cache-lifecycle-cleanup" aria-labelledby="settings-cache-cleanup-title" data-cut-cache-cleanup>
        <div>
          <strong id="settings-cache-cleanup-title">Cleanup preview</strong>
          <p>Preview only checks ledger-owned, aged, unreferenced proxies and filmstrips. Sources, exports, captures, and receipts are never eligible.</p>
        </div>
        <div className="settings-cache-lifecycle-actions">
          <button type="button" className="env-btn env-btn--ghost" data-cut-cache-preview onClick={() => void previewCache()} disabled={cleanupDisabled}>
            {previewLoading ? 'Previewing…' : 'Preview cache cleanup'}
          </button>
          {preview && <span data-cut-cache-preview-status>{preview.purgeable.files} aged file{preview.purgeable.files === 1 ? '' : 's'} / {preview.purgeable.bytes.toLocaleString()} bytes eligible</span>}
        </div>
        {cleanupError && <p className="settings-cache-lifecycle-error" data-cut-cache-error role="alert">{cleanupError}</p>}
        {cleanupSuccess && <p className="settings-cache-lifecycle-success" data-cut-cache-cleanup-success aria-live="polite">{cleanupSuccess}</p>}
        {preview && preview.purgeable.files === 0 && <p data-cut-cache-empty>No ledger-owned cache files are eligible for cleanup.</p>}
        {preview && preview.purgeable.files > 0 && !confirmationOpen && <button type="button" className="env-btn" data-cut-cache-purge onClick={() => setConfirmationOpen(true)} disabled={activeJob !== null}>Review and confirm purge</button>}
        {preview && preview.purgeable.files > 0 && confirmationOpen && (
          <div className="settings-cache-lifecycle-confirm" data-cut-cache-confirm>
            <p>Remove only the {preview.purgeable.files} previewed ledger-owned cache file{preview.purgeable.files === 1 ? '' : 's'}? This cannot delete source media, exports, captures, or receipts.</p>
            <div className="settings-cache-lifecycle-actions">
              <button type="button" className="env-btn env-btn--ghost" data-cut-cache-confirm-cancel onClick={() => setConfirmationOpen(false)}>Keep cache</button>
              <button type="button" className="env-btn" data-cut-cache-confirm-purge onClick={() => void confirmPurge()} disabled={activeJob !== null}>Purge previewed cache</button>
            </div>
          </div>
        )}
      </section>

      {(activeJob || job) && (
        <div data-cut-cache-job data-cut-cache-job-kind={activeJob?.kind ?? (job?.kind === 'cache_rebuild' ? 'rebuild' : 'cleanup')} data-cut-cache-job-state={job?.state ?? 'queued'} aria-live="polite">
          <p>{job?.message ?? `${jobLabel(activeJob?.kind ?? 'cleanup')} is queued.`} {job ? `${Math.round(job.progress * 100)}%` : ''}</p>
          {activeJob && <button type="button" className="env-btn env-btn--ghost" data-cut-cache-cancel onClick={() => void cancelActiveJob()} disabled={cancelPending}>{cancelPending ? 'Cancelling…' : `Cancel ${activeJob.kind}`}</button>}
          {!activeJob && job?.kind === 'cache_purge' && <button type="button" className="env-btn env-btn--ghost" data-cut-cache-remeasure onClick={() => void previewCache()}>Remeasure cache</button>}
        </div>
      )}
    </aside>
  )
}
